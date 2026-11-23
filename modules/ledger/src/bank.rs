//! Chuyển khoản VietQR tự khớp (bước 36 mục 9, D-194): tài khoản ngân hàng của nút nhận, thanh toán
//! VietQR (mã QR mang mã chuyển khoản), biến động số dư qua `BankFeed` (webhook kiểu đẩy hay lịch
//! `ledger.poll_bank_feed` kiểu kéo), khớp tự động theo mã + số tiền (job
//! `ledger.match_bank_transaction`), sai lệch vào hàng chờ `ledger_exceptions`.
//!
//! Tiền vào thẳng tài khoản ngân hàng của người bán / khách (chế độ tự thu, D-187): nền tảng chỉ đọc
//! biến động để ghi sổ, không giữ tiền. Bút toán khi khớp: Nợ `cash.bank[tài khoản]` / Có
//! `receivable.customer`.

use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::payment::{BankAccountRef, BankDirection, BankFeed, BankTransaction};
use autoxide_kernel::reference::{self, Reference, Submitted};
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, TenantId};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QuerySelect, Set};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::api::{NewTransferPayment, Payment};
use crate::entity::{bank_account, bank_transaction, payment};
use crate::events;
use crate::payments::describe_payment;
use crate::permissions::{PAYMENTS_CREATE, READ, SETTINGS_MANAGE};
use crate::service::{LedgerService, exchange_rate_missing, now, valid_id, valid_name};
use crate::vietqr;

pub(crate) const MATCH_BANK_TRANSACTION: &str = "ledger.match_bank_transaction";
pub(crate) const POLL_BANK_FEED: &str = "ledger.poll_bank_feed";
/// Tên "đối tác" của thanh toán chuyển khoản VietQR.
pub(crate) const VIETQR: &str = "vietqr";

pub(crate) fn bank_account_not_found() -> Error {
    Error::not_found(
        "ledger.bank_account_not_found",
        "There is no such bank account in your scope.",
    )
}

fn bank_feed_not_configured(provider: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "ledger.bank_feed_not_configured",
        "The bank feed `{provider}` is not set up on this server.",
    )
    .with_param("provider", provider)
}

/// Job khớp một biến động đã lưu.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MatchBankTransaction {
    pub transaction_id: Uuid,
}

impl Job for MatchBankTransaction {
    const KIND: &'static str = MATCH_BANK_TRANSACTION;
}

/// Tài khoản ngân hàng mới (REST / MCP).
#[derive(Debug, Clone)]
pub(crate) struct NewBankAccount {
    pub org: OrgId,
    pub bank_bin: String,
    pub account_number: String,
    pub account_name: String,
    pub feed_provider: Option<String>,
}

/// Thay đổi tài khoản ngân hàng (REST). `None` là giữ nguyên; `feed_provider: Some(None)` là bỏ
/// nguồn biến động.
#[derive(Debug, Clone, Default)]
pub(crate) struct BankAccountChange {
    pub account_name: Option<String>,
    pub feed_provider: Option<Option<String>>,
    pub status: Option<String>,
}

/// Chuỗi VietQR mẫu và tài khoản nó trỏ tới.
#[derive(Debug, Clone)]
pub(crate) struct VietqrPreview {
    pub qr_payload: String,
    pub memo: String,
    pub amount: Money,
    pub account: bank_account::Model,
}

impl LedgerService {
    pub(crate) fn bank_feed(&self, provider: &str) -> Result<Arc<dyn BankFeed>, Error> {
        self.bank_feeds
            .iter()
            .find(|feed| feed.provider() == provider)
            .cloned()
            .ok_or_else(|| bank_feed_not_configured(provider))
    }

    // ---- Tài khoản ngân hàng ------------------------------------------------------------------

    pub(crate) async fn create_bank_account(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewBankAccount,
    ) -> Result<bank_account::Model, Error> {
        principal.require(&SETTINGS_MANAGE)?;
        principal
            .require_at(tenant.database().read(), &SETTINGS_MANAGE, OrgTarget::new(input.org, None))
            .await?;
        let mut problems = Vec::new();
        if vietqr::bank(&input.bank_bin).is_none() {
            problems.push(FieldError::new(
                "bank_bin",
                "unknown",
                "is not a known bank BIN (6 digits, e.g. `970436`)",
            ));
        }
        let number = input.account_number.trim().to_owned();
        if !(1..=19).contains(&number.len()) || !number.bytes().all(|b| b.is_ascii_alphanumeric()) {
            problems.push(FieldError::new(
                "account_number",
                "format",
                "must be 1 to 19 letters or digits",
            ));
        }
        let name = input.account_name.trim().to_owned();
        if !(1..=50).contains(&name.chars().count()) {
            problems.push(FieldError::new(
                "account_name",
                "length",
                "must be 1 to 50 characters long",
            ));
        }
        if let Some(provider) = input.feed_provider.as_deref()
            && self.bank_feed(provider).is_err()
        {
            problems.push(FieldError::new(
                "feed_provider",
                "unknown",
                "is not a bank feed set up on this server",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let id = Uuid::now_v7();
        let transaction = tenant.database().begin_write().await?;
        principal
            .require_write_at(&transaction, &SETTINGS_MANAGE, OrgTarget::new(input.org, None))
            .await?;
        let at = now();
        let inserted = bank_account::Entity::insert(bank_account::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(input.org.as_uuid()),
            bank_bin: Set(input.bank_bin.clone()),
            account_number: Set(number),
            account_name: Set(name),
            feed_provider: Set(input.feed_provider.clone()),
            feed_cursor: Set(None),
            status: Set("active".to_owned()),
            created_at: Set(at),
            updated_at: Set(at),
        })
        .on_conflict(
            OnConflict::columns([
                bank_account::Column::TenantId,
                bank_account::Column::BankBin,
                bank_account::Column::AccountNumber,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(&transaction)
        .await?;
        if inserted == 0 {
            return Err(Error::conflict(
                "ledger.bank_account_exists",
                "This bank account is already registered.",
            ));
        }
        let saved = bank_account::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| Error::internal("a bank account vanished after it was written"))?;
        transaction.commit().await?;
        Ok(saved)
    }

    /// Sửa tài khoản ngân hàng: tên chủ tài khoản, nguồn biến động, trạng thái. Mã ngân hàng và số
    /// tài khoản không đổi — tài khoản khác thì đăng ký mới (khoá chống trùng dựa vào hai cột đó).
    pub(crate) async fn update_bank_account(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        change: BankAccountChange,
    ) -> Result<bank_account::Model, Error> {
        principal.require(&SETTINGS_MANAGE)?;
        let read = tenant.database().read();
        // Ngoài phạm vi đọc thì như không có (404); đọc được mà không có quyền sửa ở nút → 403.
        let condition = principal.scope_condition(&READ, bank_account::Column::OrgId, None)?;
        let found = bank_account::Entity::find_by_id(id)
            .scoped(tenant)
            .filter(condition)
            .one(read)
            .await?
            .ok_or_else(bank_account_not_found)?;
        let org = OrgTarget::new(OrgId::from_uuid(found.org_id), None);
        principal.require_at(read, &SETTINGS_MANAGE, org).await?;

        let mut problems = Vec::new();
        let name = change.account_name.as_deref().map(|name| name.trim().to_owned());
        if let Some(name) = &name
            && !(1..=50).contains(&name.chars().count())
        {
            problems.push(FieldError::new(
                "account_name",
                "length",
                "must be 1 to 50 characters long",
            ));
        }
        if let Some(Some(provider)) = change.feed_provider.as_ref()
            && self.bank_feed(provider).is_err()
        {
            problems.push(FieldError::new(
                "feed_provider",
                "unknown",
                "is not a bank feed set up on this server",
            ));
        }
        if let Some(status) = &change.status
            && !["active", "disabled"].contains(&status.as_str())
        {
            problems.push(FieldError::new("status", "invalid", "must be `active` or `disabled`"));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }

        let transaction = tenant.database().begin_write().await?;
        principal.require_write_at(&transaction, &SETTINGS_MANAGE, org).await?;
        let mut update = bank_account::Entity::update_many()
            .col_expr(bank_account::Column::UpdatedAt, now().into())
            .filter(bank_account::Column::Id.eq(found.id))
            .filter(bank_account::Column::TenantId.eq(tenant.id().as_uuid()));
        if let Some(name) = name {
            update = update.col_expr(bank_account::Column::AccountName, name.into());
        }
        if let Some(provider) = change.feed_provider {
            // Đổi nguồn biến động thì con trỏ của nguồn cũ vô nghĩa với nguồn mới: đọc lại từ đầu.
            if provider != found.feed_provider {
                update = update.col_expr(bank_account::Column::FeedCursor, None::<String>.into());
            }
            update = update.col_expr(bank_account::Column::FeedProvider, provider.into());
        }
        if let Some(status) = change.status {
            update = update.col_expr(bank_account::Column::Status, status.into());
        }
        update.exec(&transaction).await?;
        let saved = bank_account::Entity::find_by_id(found.id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(bank_account_not_found)?;
        transaction.commit().await?;
        Ok(saved)
    }

    /// Chuỗi VietQR mẫu cho tài khoản `id` (không tạo thanh toán, không ghi gì): để kiểm số tài khoản
    /// đã đúng bằng cách quét thử bằng ứng dụng ngân hàng trước khi nhận tiền thật. Nội dung mặc định
    /// là một mã chuyển khoản mẫu cùng dạng mã thật.
    pub(crate) async fn preview_vietqr(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        amount: Money,
        memo: Option<String>,
    ) -> Result<VietqrPreview, Error> {
        principal.require(&READ)?;
        let condition = principal.scope_condition(&READ, bank_account::Column::OrgId, None)?;
        let account = bank_account::Entity::find_by_id(id)
            .scoped(tenant)
            .filter(condition)
            .one(tenant.database().read())
            .await?
            .ok_or_else(bank_account_not_found)?;
        let mut problems = Vec::new();
        if !amount.is_positive() || amount.currency() != Currency::VND {
            problems.push(FieldError::new(
                "amount",
                "range",
                "must be a positive amount in VND (VietQR transfers are in VND)",
            ));
        }
        let memo = memo.map(|memo| memo.trim().to_owned());
        if let Some(memo) = &memo
            && (!(1..=25).contains(&memo.len())
                || !memo.bytes().all(|b| b.is_ascii_alphanumeric() || b == b' '))
        {
            problems.push(FieldError::new(
                "memo",
                "format",
                "must be 1 to 25 letters, digits or spaces, without accents",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let memo = memo.unwrap_or_else(|| vietqr::transfer_code(Uuid::now_v7()));
        let qr_payload = vietqr::payload(&account.bank_bin, &account.account_number, amount, &memo)
            .ok_or_else(|| Error::internal("a checked VietQR transfer did not fit the QR"))?;
        Ok(VietqrPreview { qr_payload, memo, amount, account })
    }

    pub(crate) async fn list_bank_accounts(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<bank_account::Model>, Error> {
        let condition = principal.scope_condition(&READ, bank_account::Column::OrgId, None)?;
        let select = bank_account::Entity::find().scoped(tenant).filter(condition);
        fetch_page(select, bank_account::Column::Id, |row| row.id, page, tenant.database().read())
            .await
    }

    // ---- Thanh toán VietQR --------------------------------------------------------------------

    /// Thanh toán VietQR qua REST / MCP: cần `ledger.payments_create` ở nút nhận tiền.
    pub(crate) async fn create_transfer_payment_manual(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewTransferPayment,
    ) -> Result<Submitted<Payment>, Error> {
        principal.require(&PAYMENTS_CREATE)?;
        principal
            .require_at(tenant.database().read(), &PAYMENTS_CREATE, OrgTarget::new(input.org, None))
            .await?;
        self.create_transfer_payment(tenant, Actor::from(principal), input).await
    }

    /// Thanh toán VietQR (`dyn Ledger`).
    pub(crate) async fn create_transfer_payment(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        input: NewTransferPayment,
    ) -> Result<Submitted<Payment>, Error> {
        let mut problems = Vec::new();
        let reference = Reference::checked(&input.reference, "reference", &mut problems);
        if !input.amount.is_positive() || input.amount.currency() != Currency::VND {
            problems.push(FieldError::new(
                "amount",
                "range",
                "must be a positive amount in VND (VietQR transfers are in VND)",
            ));
        }
        let description = input.description.trim().to_owned();
        if !(1..=140).contains(&description.chars().count()) {
            problems.push(FieldError::new(
                "description",
                "length",
                "must be 1 to 140 characters long (no personal data)",
            ));
        }
        if !valid_name(&input.source_module) {
            problems.push(FieldError::new("source_module", "format", "must be a module name"));
        }
        if input.source_id.as_deref().is_some_and(|id| !valid_id(id)) {
            problems.push(FieldError::new(
                "source_id",
                "format",
                "must be 1 to 100 visible ASCII characters",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let Some(reference) = reference else {
            return Err(Error::internal("a checked reference went missing"));
        };
        let read = tenant.database().read();
        let mut select = bank_account::Entity::find()
            .scoped(tenant)
            .filter(bank_account::Column::OrgId.eq(input.org.as_uuid()))
            .filter(bank_account::Column::Status.eq("active"));
        if let Some(id) = input.bank_account {
            select = select.filter(bank_account::Column::Id.eq(id));
        }
        let accounts = select.limit(2).all(read).await?;
        let account = match accounts.as_slice() {
            [only] => only.clone(),
            [] => return Err(bank_account_not_found()),
            _ => {
                return Err(Error::validation(vec![FieldError::new(
                    "bank_account_id",
                    "required",
                    "is required: this unit has several bank accounts",
                )]));
            }
        };
        let book = self.ensure_book(tenant, input.org).await?;
        if book.base_currency != input.amount.currency().code() {
            let base = Currency::parse(&book.base_currency)?;
            let today = self.today(tenant, input.org).await;
            return Err(exchange_rate_missing(base, input.amount.currency(), today));
        }
        let fingerprint = reference::fingerprint(&json!({
            "org": input.org.as_uuid(), "bank_account": account.id, "amount": input.amount.minor(),
            "description": description, "source_id": input.source_id,
        }))?;
        let expiry = SignedDuration::from_mins(self.config.payment_expiry_minutes);
        let (database, outbox) = (tenant.database(), &self.outbox);
        let (reference, fingerprint, input, book, description, account) =
            (&reference, &fingerprint, &input, &book, &description, &account);
        let submitted = reference::submit_once(database, move || async move {
            let transaction = database.begin_write().await?;
            let stored = payment::Entity::find()
                .scoped(tenant)
                .filter(payment::Column::SourceModule.eq(input.source_module.as_str()))
                .filter(payment::Column::Reference.eq(reference.as_str()))
                .one(&transaction)
                .await?
                .map(|row| {
                    let fingerprint = row.reference_fingerprint.clone();
                    (row, fingerprint)
                });
            if let Some(existing) = reference::replayed(stored, reference, fingerprint)? {
                return Ok(Submitted::Replayed(existing));
            }
            let id = Uuid::now_v7();
            let code = vietqr::transfer_code(id);
            let qr =
                vietqr::payload(&account.bank_bin, &account.account_number, input.amount, &code)
                    .ok_or_else(|| Error::internal("the VietQR payload does not fit"))?;
            let at = now();
            let saved = payment::Entity::insert(payment::ActiveModel {
                id: Set(id),
                tenant_id: Set(tenant.id().as_uuid()),
                org_id: Set(book.org_id),
                book_id: Set(book.id),
                provider: Set(VIETQR.to_owned()),
                provider_ref: Set(None),
                method: Set(VIETQR.to_owned()),
                description: Set(description.clone()),
                amount_minor: Set(input.amount.minor()),
                amount_currency: Set(input.amount.currency().code().to_owned()),
                refunded_minor: Set(0),
                status: Set("pending".to_owned()),
                checkout_url: Set(None),
                qr_payload: Set(Some(qr)),
                return_url: Set(None),
                expires_at: Set(at + expiry),
                captured_at: Set(None),
                source_module: Set(input.source_module.clone()),
                source_id: Set(input.source_id.clone()),
                reference: Set(reference.as_str().to_owned()),
                reference_fingerprint: Set(fingerprint.clone()),
                created_at: Set(at),
                updated_at: Set(at),
                transfer_code: Set(Some(code)),
                bank_account_id: Set(Some(account.id)),
                captured_minor: Set(None),
            })
            .exec_with_returning(&transaction)
            .await?;
            let event = events::PaymentCreated(json!({
                "payment_id": saved.id, "org_id": saved.org_id, "provider": saved.provider,
                "amount": input.amount, "source_module": saved.source_module,
                "source_id": saved.source_id, "reference": saved.reference,
            }));
            outbox
                .record(&transaction, tenant, actor, Some(format!("ledger.payment:{id}")), &event)
                .await?;
            transaction.commit().await?;
            Ok(Submitted::Created(saved))
        })
        .await?;
        Ok(match submitted {
            Submitted::Created(row) => Submitted::Created(describe_payment(&row)?),
            Submitted::Replayed(row) => Submitted::Replayed(describe_payment(&row)?),
        })
    }

    // ---- Biến động số dư ----------------------------------------------------------------------

    /// Webhook của nguồn biến động `provider`: kiểm chữ ký trên thân thô, lưu biến động và xếp job
    /// khớp trong một transaction. Trả số biến động mới.
    pub(crate) async fn receive_bank_feed(
        &self,
        tenant: &TenantContext,
        provider: &str,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> Result<u64, Error> {
        let feed = self.bank_feed(provider)?;
        let transactions = match feed.verify_webhook(tenant.id(), headers, body).await {
            Ok(found) => found,
            Err(error) => {
                metrics::counter!("ledger_bank_feed_rejected_total", "provider" => feed.provider())
                    .increment(1);
                tracing::warn!(provider = feed.provider(), "bank feed webhook rejected");
                return Err(error);
            }
        };
        self.store_bank_transactions(tenant, feed.provider(), None, &transactions, None).await
    }

    /// Lưu biến động (gửi lại không ghi hai lần) và xếp job khớp, trong một transaction; kiểu kéo
    /// thì cập nhật con trỏ của tài khoản `account` cùng lúc. Biến động của tài khoản không đăng ký
    /// bị bỏ qua.
    async fn store_bank_transactions(
        &self,
        tenant: &TenantContext,
        provider: &str,
        account: Option<Uuid>,
        transactions: &[BankTransaction],
        cursor: Option<String>,
    ) -> Result<u64, Error> {
        let read = tenant.database().read();
        let numbers: Vec<String> =
            transactions.iter().map(|txn| txn.account_number.clone()).collect();
        let accounts = bank_account::Entity::find()
            .scoped(tenant)
            .filter(bank_account::Column::AccountNumber.is_in(numbers))
            .all(read)
            .await?;
        let (database, jobs, accounts, cursor) =
            (tenant.database(), &self.jobs, &accounts, &cursor);
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let mut stored = 0;
                for txn in transactions {
                    let Some(owner) =
                        accounts.iter().find(|found| found.account_number == txn.account_number)
                    else {
                        tracing::warn!(provider, "bank transaction for an unknown account skipped");
                        continue;
                    };
                    let id = Uuid::now_v7();
                    let inserted =
                        bank_transaction::Entity::insert(bank_transaction::ActiveModel {
                            id: Set(id),
                            tenant_id: Set(tenant.id().as_uuid()),
                            bank_account_id: Set(owner.id),
                            provider: Set(provider.to_owned()),
                            provider_txn_id: Set(txn.provider_txn_id.clone()),
                            direction: Set(txn.direction.as_str().to_owned()),
                            amount_minor: Set(txn.amount.minor()),
                            amount_currency: Set(txn.amount.currency().code().to_owned()),
                            memo: Set(txn.memo.chars().take(500).collect()),
                            booked_at: Set(txn.booked_at),
                            match_status: Set("pending".to_owned()),
                            payment_id: Set(None),
                            received_at: Set(now()),
                        })
                        .on_conflict(
                            OnConflict::columns([
                                bank_transaction::Column::TenantId,
                                bank_transaction::Column::Provider,
                                bank_transaction::Column::ProviderTxnId,
                            ])
                            .do_nothing()
                            .to_owned(),
                        )
                        .exec_without_returning(&transaction)
                        .await?;
                    if inserted == 1 {
                        stored += 1;
                        jobs.enqueue(
                            &transaction,
                            tenant,
                            &MatchBankTransaction { transaction_id: id },
                            EnqueueOptions::default().unique_key(id.to_string()),
                        )
                        .await?;
                    }
                }
                if let Some(account) = account {
                    bank_account::Entity::update_many()
                        .scoped(tenant)
                        .col_expr(bank_account::Column::FeedCursor, cursor.clone().into())
                        .col_expr(bank_account::Column::UpdatedAt, now().into())
                        .filter(bank_account::Column::Id.eq(account))
                        .exec(&transaction)
                        .await?;
                }
                transaction.commit().await?;
                Ok(stored)
            })
            .await
    }

    /// Lịch `ledger.poll_bank_feed`: kéo biến động của mọi tài khoản có nguồn trên `database`. Lỗi
    /// của một tài khoản không chặn tài khoản khác.
    pub(crate) async fn poll_bank_feeds(
        &self,
        database: &autoxide_kernel::database::Database,
    ) -> Result<u64, Error> {
        // tenant-scope: việc nền / kiểm tra quét mọi khách của database, chủ ý không lọc khách (rà soát bảo mật 2026-11)
        let accounts = bank_account::Entity::find()
            .filter(bank_account::Column::FeedProvider.is_not_null())
            .filter(bank_account::Column::Status.eq("active"))
            .limit(500)
            .all(database.read())
            .await?;
        let mut stored = 0;
        for account in accounts {
            let tenant = self.databases.tenant(TenantId::from_uuid(account.tenant_id)).await?;
            match self.poll_account(&tenant, &account).await {
                Ok(count) => stored += count,
                Err(error) => {
                    tracing::warn!(tenant_id = %tenant.id(), bank_account_id = %account.id, error = %error, "cannot read a bank feed");
                }
            }
        }
        Ok(stored)
    }

    async fn poll_account(
        &self,
        tenant: &TenantContext,
        account: &bank_account::Model,
    ) -> Result<u64, Error> {
        let Some(provider) = account.feed_provider.as_deref() else { return Ok(0) };
        let feed = self.bank_feed(provider)?;
        let reference = BankAccountRef::new(&account.bank_bin, &account.account_number);
        let mut cursor = account.feed_cursor.clone();
        let mut stored = 0;
        // Tối đa 10 trang một lượt: tài khoản nhiều giao dịch không chiếm cả lịch.
        for _ in 0..10 {
            let batch = feed.fetch(tenant.id(), &reference, cursor.clone()).await?;
            let next = batch.next_cursor.clone().or(cursor.clone());
            stored += self
                .store_bank_transactions(
                    tenant,
                    feed.provider(),
                    Some(account.id),
                    &batch.transactions,
                    next.clone(),
                )
                .await?;
            cursor = next;
            if !batch.has_more {
                break;
            }
        }
        Ok(stored)
    }

    /// Khớp biến động `id` (job). Chạy lại vô hại: chỉ xử lý biến động còn `pending`.
    pub(crate) async fn match_bank_transaction(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<(), Error> {
        let database = tenant.database();
        let Some(txn) =
            bank_transaction::Entity::find_by_id(id).scoped(tenant).one(database.write()).await?
        else {
            return Ok(());
        };
        if txn.match_status != "pending" {
            return Ok(());
        }
        let amount = Money::from_columns(txn.amount_minor, &txn.amount_currency)?;
        let subject = format!("ledger.bank_transaction:{id}");
        let (status, payment_id) = if txn.direction != BankDirection::Credit.as_str() {
            ("ignored", None)
        } else {
            let codes = vietqr::codes_in(&txn.memo);
            let found = if codes.is_empty() {
                None
            } else {
                // Chỉ thanh toán chờ tiền vào đúng tài khoản nhận biến động này: mã của người bán B
                // trong nội dung một khoản vào tài khoản của người bán A không làm B "đã nhận tiền"
                // (tiền nằm ở A) — khoản đó vào hàng chờ đối soát (rà soát bảo mật 2026-11).
                payment::Entity::find()
                    .scoped(tenant)
                    .filter(payment::Column::TransferCode.is_in(codes))
                    .filter(
                        Condition::any()
                            .add(payment::Column::BankAccountId.eq(txn.bank_account_id))
                            .add(payment::Column::BankAccountId.is_null()),
                    )
                    .one(database.write())
                    .await?
            };
            match found {
                None => {
                    self.open_exception(
                        tenant,
                        "unmatched_credit",
                        &subject,
                        json!({"bank_transaction_id": id, "provider": txn.provider,
                               "amount": amount, "received_minor": amount.minor()}),
                        &format!("unmatched_credit:bank:{id}"),
                    )
                    .await?;
                    ("exception", None)
                }
                Some(paid) => match self.claim_payment(tenant, id, paid.id).await? {
                    // Thanh toán đã xong, hay một biến động khác đã nhận nó trước (hai lần chuyển
                    // tiền cho một mã, kể cả khi tới cùng lúc): tiền lần này vào hàng chờ để người
                    // quyết hoàn hay giữ — không mất, không cộng hai lần.
                    Claim::Taken => {
                        self.open_exception(
                            tenant,
                            "duplicate_payment",
                            &format!("ledger.payment:{}", paid.id),
                            json!({"payment_id": paid.id, "bank_transaction_id": id,
                                   "amount": amount, "received_minor": amount.minor()}),
                            &format!("duplicate_payment:bank:{id}"),
                        )
                        .await?;
                        ("exception", Some(paid.id))
                    }
                    Claim::Ours { was } => {
                        self.capture(tenant, paid.id, amount, "bank_feed").await?;
                        let after = Self::find_payment(database.write(), tenant, paid.id).await?;
                        let clean = was != "expired"
                            && after.is_some_and(|row| {
                                row.status == "succeeded" && row.amount_minor == amount.minor()
                            });
                        (if clean { "matched" } else { "exception" }, Some(paid.id))
                    }
                },
            }
        };
        bank_transaction::Entity::update_many()
            .scoped(tenant)
            .col_expr(bank_transaction::Column::MatchStatus, status.into())
            .col_expr(bank_transaction::Column::PaymentId, payment_id.into())
            .filter(bank_transaction::Column::Id.eq(id))
            .exec(database.write())
            .await?;
        Ok(())
    }
}

/// Kết quả nhận một thanh toán cho một biến động số dư.
enum Claim {
    /// Biến động này giữ thanh toán (vừa nhận, hay đã nhận ở lần chạy trước); `was`: trạng thái của
    /// thanh toán lúc nhận.
    Ours { was: String },
    /// Thanh toán đã xong, hay một biến động khác đã nhận nó.
    Taken,
}

impl LedgerService {
    /// Biến động `txn` nhận thanh toán `payment_id` (ghi `payment_id` lên biến động) nếu chưa biến
    /// động nào khác nhận và thanh toán chưa xong. Khoá dòng thanh toán trong transaction ghi: hai
    /// biến động cùng mã chạy song song thì chỉ một bên nhận được (Postgres `FOR UPDATE`; SQLite một
    /// bên ghi mỗi lúc). Chạy lại cho cùng biến động → vẫn là của nó.
    async fn claim_payment(
        &self,
        tenant: &TenantContext,
        txn: Uuid,
        payment_id: Uuid,
    ) -> Result<Claim, Error> {
        let database = tenant.database();
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let Some(paid) = rows::lock_one(
                    &transaction,
                    payment::Entity::find_by_id(payment_id).scoped(tenant),
                )
                .await?
                else {
                    return Ok(Claim::Taken);
                };
                let mine = bank_transaction::Entity::find_by_id(txn)
                    .scoped(tenant)
                    .one(&transaction)
                    .await?
                    .is_some_and(|row| row.payment_id == Some(payment_id));
                if mine {
                    return Ok(Claim::Ours { was: paid.status });
                }
                let other = bank_transaction::Entity::find()
                    .select_only()
                    .column(bank_transaction::Column::Id)
                    .filter(bank_transaction::Column::PaymentId.eq(payment_id))
                    .filter(bank_transaction::Column::Id.ne(txn))
                    .scoped(tenant)
                    .into_tuple::<Uuid>()
                    .one(&transaction)
                    .await?;
                if paid.status == "succeeded" || other.is_some() {
                    return Ok(Claim::Taken);
                }
                bank_transaction::Entity::update_many()
                    .col_expr(bank_transaction::Column::PaymentId, Some(payment_id).into())
                    .filter(bank_transaction::Column::Id.eq(txn))
                    .scoped(tenant)
                    .exec(&transaction)
                    .await?;
                transaction.commit().await?;
                Ok(Claim::Ours { was: paid.status })
            })
            .await
    }
}

/// Job `ledger.match_bank_transaction`.
pub(crate) struct MatchBankTransactionJob(pub Arc<LedgerService>);

#[async_trait::async_trait]
impl JobHandler for MatchBankTransactionJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let payload: MatchBankTransaction =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        let tenant = job
            .tenant
            .as_ref()
            .ok_or_else(|| Error::internal("ledger.match_bank_transaction needs a tenant"))?;
        self.0.match_bank_transaction(tenant, payload.transaction_id).await
    }
}

/// Lịch `ledger.poll_bank_feed` (mọi database).
pub(crate) struct PollBankFeedJob(pub Arc<LedgerService>);

#[async_trait::async_trait]
impl JobHandler for PollBankFeedJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.poll_bank_feeds(&job.database).await.map(|_| ())
    }
}
