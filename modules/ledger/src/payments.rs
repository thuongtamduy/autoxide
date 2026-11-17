//! Thanh toán qua đối tác (bước 36 mục 5): tạo lượt thu, nhận webhook, ghi sổ khi đã thu, hoàn
//! tiền, hết hạn.
//!
//! Không gọi mạng trong transaction ghi: mỗi lệnh là transaction ngắn ghi trạng thái → commit →
//! gọi đối tác → transaction ngắn ghi kết quả. Lỗi mạng giữ trạng thái `pending`; gọi lại cùng
//! `reference` thì gọi lại đối tác với cùng ID (đối tác chống trùng theo mã đơn của merchant).
//!
//! Bút toán (chế độ tự thu; chế độ thu hộ / chia tại cổng ở mục 6): đã thu → Nợ `clearing.gateway`
//! / Có `receivable.customer`; hoàn → ngược lại. Doanh thu và thuế ghi lúc lập chứng từ (Nợ
//! `receivable.customer` / Có `revenue.*`, `tax.*`), không ở đây.

use std::sync::Arc;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::payment::{
    GatewayNotification, NotificationKind, PaymentGateway, PaymentIntent, RefundIntent,
    RefundStatus,
};
use autoxide_kernel::reference::{self, Reference, Submitted};
use autoxide_kernel::rows::{self, Bounds};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use chrono::{DateTime, Duration, Utc};
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{
    AccountKey, Counterparty, LineAccount, NewEntry, NewLine, NewPayment, NewRefund, Payment,
    Refund,
};
use crate::entity::{gateway_event, payment, refund};
use crate::events;
use crate::permissions::{PAYMENTS_CREATE, READ, REFUNDS_CREATE};
use crate::service::{LedgerService, exchange_rate_missing, money, now, valid_id, valid_name};

pub(crate) const PROCESS_GATEWAY_EVENT: &str = "ledger.process_gateway_event";
pub(crate) const EXPIRE_PAYMENTS: &str = "ledger.expire_payments";

// ---- Lỗi ----------------------------------------------------------------------------------------

pub(crate) fn gateway_not_configured(provider: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "ledger.gateway_not_configured",
        "The payment partner `{provider}` is not set up on this server.",
    )
    .with_param("provider", provider)
}

pub(crate) fn payment_not_found() -> Error {
    Error::not_found("ledger.payment_not_found", "There is no such payment.")
}

fn payment_state(status: &str) -> Error {
    Error::conflict("ledger.payment_state", "The payment is `{status}`; this cannot be done now.")
        .with_param("status", status)
}

fn refund_exceeds(remaining: Money) -> Error {
    Error::new(
        ErrorKind::Validation,
        "ledger.refund_exceeds_payment",
        "The refund is more than what is left of the payment ({remaining}).",
    )
    .with_param("remaining", remaining)
}

// ---- Mô tả ----------------------------------------------------------------------------------------

pub(crate) fn describe_payment(row: &payment::Model) -> Result<Payment, Error> {
    Ok(Payment {
        id: row.id,
        org_id: row.org_id,
        book_id: row.book_id,
        provider: row.provider.clone(),
        provider_ref: row.provider_ref.clone(),
        method: row.method.clone(),
        description: row.description.clone(),
        amount: money(row.amount_minor, &row.amount_currency)?,
        refunded: money(row.refunded_minor, &row.amount_currency)?,
        status: row.status.clone(),
        checkout_url: row.checkout_url.clone(),
        qr_payload: row.qr_payload.clone(),
        transfer_code: row.transfer_code.clone(),
        bank_account_id: row.bank_account_id,
        expires_at: row.expires_at,
        captured_at: row.captured_at,
        source_module: row.source_module.clone(),
        source_id: row.source_id.clone(),
        reference: row.reference.clone(),
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

pub(crate) fn describe_refund(row: &refund::Model) -> Result<Refund, Error> {
    Ok(Refund {
        id: row.id,
        payment_id: row.payment_id,
        amount: money(row.amount_minor, &row.amount_currency)?,
        status: row.status.clone(),
        provider_ref: row.provider_ref.clone(),
        reason: row.reason.clone(),
        reference: row.reference.clone(),
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn payment_subject(id: Uuid) -> Option<String> {
    Some(format!("ledger.payment:{id}"))
}

/// Thân của event thanh toán (chỉ ID, số tiền, trạng thái).
fn payment_payload(row: &payment::Model) -> Result<Value, Error> {
    Ok(json!({
        "payment_id": row.id, "org_id": row.org_id, "provider": row.provider,
        "provider_ref": row.provider_ref, "amount": money(row.amount_minor, &row.amount_currency)?,
        "source_module": row.source_module, "source_id": row.source_id, "reference": row.reference,
    }))
}

/// Job xử lý một thông báo đã lưu.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ProcessGatewayEvent {
    pub event_id: Uuid,
}

impl Job for ProcessGatewayEvent {
    const KIND: &'static str = PROCESS_GATEWAY_EVENT;
}

/// Thông báo đã kiểm, lưu dạng JSON trong `ledger_gateway_events.notification`.
#[derive(Debug, Serialize, Deserialize)]
struct StoredNotification {
    kind: String,
    provider_ref: String,
    payment_id: Option<Uuid>,
    amount_minor: i64,
    currency: String,
    reason: Option<String>,
    refund_provider_ref: Option<String>,
    occurred_at: DateTime<Utc>,
}

impl StoredNotification {
    fn of(notification: &GatewayNotification) -> Self {
        let (kind, reason, refund_ref) = match &notification.kind {
            NotificationKind::PaymentSucceeded => ("payment_succeeded", None, None),
            NotificationKind::PaymentFailed { reason } => {
                ("payment_failed", Some(reason.clone()), None)
            }
            NotificationKind::RefundSucceeded { refund_provider_ref } => {
                ("refund_succeeded", None, Some(refund_provider_ref.clone()))
            }
            NotificationKind::RefundFailed { refund_provider_ref } => {
                ("refund_failed", None, Some(refund_provider_ref.clone()))
            }
        };
        StoredNotification {
            kind: kind.to_owned(),
            provider_ref: notification.provider_ref.clone(),
            payment_id: notification.payment_id,
            amount_minor: notification.amount.minor(),
            currency: notification.amount.currency().code().to_owned(),
            reason,
            refund_provider_ref: refund_ref,
            occurred_at: notification.occurred_at,
        }
    }
}

/// Bộ lọc danh sách thanh toán.
#[derive(Debug, Clone, Default)]
pub(crate) struct PaymentFilter {
    pub status: Option<String>,
    pub source_module: Option<String>,
    pub source_id: Option<String>,
}

impl LedgerService {
    // ---- Cổng ----------------------------------------------------------------------------------

    pub(crate) fn gateway(&self, provider: Option<&str>) -> Result<Arc<dyn PaymentGateway>, Error> {
        match provider {
            Some(name) => self
                .gateways
                .iter()
                .find(|gateway| gateway.provider() == name)
                .cloned()
                .ok_or_else(|| gateway_not_configured(name)),
            None => match self.gateways.as_slice() {
                [only] => Ok(only.clone()),
                [] => Err(gateway_not_configured("-")),
                _ => Err(Error::validation(vec![FieldError::new(
                    "provider",
                    "required",
                    "is required: this server has several payment partners",
                )])),
            },
        }
    }

    // ---- Tạo thanh toán ------------------------------------------------------------------------

    /// Tạo lượt thu (`dyn Ledger`).
    pub(crate) async fn create_payment(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        input: NewPayment,
    ) -> Result<Submitted<Payment>, Error> {
        let mut problems = Vec::new();
        let reference = Reference::checked(&input.reference, "reference", &mut problems);
        if !input.amount.is_positive() {
            problems.push(FieldError::new("amount", "range", "must be greater than zero"));
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
        if input.return_url.as_deref().is_some_and(|url| {
            !(url.starts_with("https://") || url.starts_with("http://")) || url.len() > 2000
        }) {
            problems.push(FieldError::new(
                "return_url",
                "format",
                "must be an http(s) URL of at most 2000 characters",
            ));
        }
        let gateway = match self.gateway(input.provider.as_deref()) {
            Ok(gateway) => Some(gateway),
            Err(error) if error.code() == "validation_failed" => {
                problems.extend(error.fields().iter().cloned());
                None
            }
            Err(error) => return Err(error),
        };
        if let Some(gateway) = &gateway
            && !gateway.capabilities().methods.contains(&input.method)
        {
            problems.push(
                FieldError::new("method", "unsupported", "must be one of {values}")
                    .with_param("values", gateway.capabilities().methods.join(", ")),
            );
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let (Some(reference), Some(gateway)) = (reference, gateway) else {
            return Err(Error::internal("checked payment input went missing"));
        };
        let book = self.ensure_book(tenant, input.org).await?;
        if book.base_currency != input.amount.currency().code() {
            let base = Currency::parse(&book.base_currency)?;
            let today = self.today(tenant, input.org).await;
            return Err(exchange_rate_missing(base, input.amount.currency(), today));
        }
        let fingerprint = reference::fingerprint(&json!({
            "org": input.org.as_uuid(), "provider": gateway.provider(), "method": input.method,
            "amount": input.amount.minor(), "currency": input.amount.currency().code(),
            "description": description, "source_id": input.source_id, "return_url": input.return_url,
        }))?;
        let expiry = Duration::minutes(self.config.payment_expiry_minutes);
        let (database, outbox) = (tenant.database(), &self.outbox);
        let (reference, fingerprint, input, book, description, provider) =
            (&reference, &fingerprint, &input, &book, &description, gateway.provider());
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
            let at = now();
            let row = payment::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                org_id: Set(book.org_id),
                book_id: Set(book.id),
                provider: Set(provider.to_owned()),
                provider_ref: Set(None),
                method: Set(input.method.clone()),
                description: Set(description.clone()),
                amount_minor: Set(input.amount.minor()),
                amount_currency: Set(input.amount.currency().code().to_owned()),
                refunded_minor: Set(0),
                status: Set("pending".to_owned()),
                checkout_url: Set(None),
                qr_payload: Set(None),
                return_url: Set(input.return_url.clone()),
                expires_at: Set(at + expiry),
                captured_at: Set(None),
                source_module: Set(input.source_module.clone()),
                source_id: Set(input.source_id.clone()),
                reference: Set(reference.as_str().to_owned()),
                reference_fingerprint: Set(fingerprint.clone()),
                created_at: Set(at),
                updated_at: Set(at),
                transfer_code: Set(None),
                bank_account_id: Set(None),
            };
            let saved = payment::Entity::insert(row).exec_with_returning(&transaction).await?;
            let event = events::PaymentCreated(payment_payload(&saved)?);
            outbox.record(&transaction, tenant, actor, payment_subject(saved.id), &event).await?;
            transaction.commit().await?;
            Ok(Submitted::Created(saved))
        })
        .await?;
        let created = matches!(submitted, Submitted::Created(_));
        let mut row = submitted.into_inner();
        // Ngoài transaction: gọi đối tác (cả khi gửi lại mà lần trước đối tác chưa trả lời).
        if row.status == "pending" && row.provider_ref.is_none() {
            let amount = money(row.amount_minor, &row.amount_currency)?;
            let intent =
                PaymentIntent::new(row.id, amount, &row.method, &row.description, row.expires_at)
                    .return_url(row.return_url.clone());
            let answer = gateway.create_payment(tenant.id(), &intent).await?;
            payment::Entity::update_many()
                .col_expr(payment::Column::ProviderRef, answer.provider_ref.clone().into())
                .col_expr(payment::Column::CheckoutUrl, answer.checkout_url.clone().into())
                .col_expr(payment::Column::QrPayload, answer.qr_payload.clone().into())
                .col_expr(payment::Column::UpdatedAt, now().into())
                .filter(payment::Column::Id.eq(row.id))
                .filter(payment::Column::TenantId.eq(tenant.id().as_uuid()))
                .filter(payment::Column::ProviderRef.is_null())
                .exec(database.write())
                .await?;
            row = payment::Entity::find_by_id(row.id)
                .scoped(tenant)
                .one(database.write())
                .await?
                .ok_or_else(payment_not_found)?;
        }
        let described = describe_payment(&row)?;
        Ok(if created { Submitted::Created(described) } else { Submitted::Replayed(described) })
    }

    /// Tạo lượt thu qua REST / MCP: cần `ledger.payments_create` ở nút nhận tiền.
    pub(crate) async fn create_payment_manual(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewPayment,
    ) -> Result<Submitted<Payment>, Error> {
        principal.require(&PAYMENTS_CREATE)?;
        principal
            .require_at(tenant.database().read(), &PAYMENTS_CREATE, OrgTarget::new(input.org, None))
            .await?;
        self.create_payment(tenant, Actor::from(principal), input).await
    }

    // ---- Đọc -----------------------------------------------------------------------------------

    pub(crate) async fn find_payment<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Option<payment::Model>, Error> {
        Ok(payment::Entity::find_by_id(id).scoped(tenant).one(connection).await?)
    }

    /// Thanh toán `id` cho người có `permission` ở nút nhận tiền; ngoài phạm vi: như không có.
    pub(crate) async fn payment_for(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        permission: &autoxide_kernel::access::Permission,
        id: Uuid,
    ) -> Result<payment::Model, Error> {
        principal.require(permission)?;
        let read = tenant.database().read();
        let found = Self::find_payment(read, tenant, id).await?.ok_or_else(payment_not_found)?;
        principal
            .require_at(read, permission, OrgTarget::new(OrgId::from_uuid(found.org_id), None))
            .await
            .map_err(|error| {
                if error.code() == "org_out_of_scope" { payment_not_found() } else { error }
            })?;
        Ok(found)
    }

    pub(crate) async fn list_payments(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: PaymentFilter,
        page: &PageRequest,
    ) -> Result<Page<payment::Model>, Error> {
        let condition = principal.scope_condition(&READ, payment::Column::OrgId, None)?;
        let mut select = payment::Entity::find().scoped(tenant).filter(condition);
        if let Some(status) = filter.status {
            select = select.filter(payment::Column::Status.eq(status));
        }
        if let Some(module) = filter.source_module {
            select = select.filter(payment::Column::SourceModule.eq(module));
        }
        if let Some(id) = filter.source_id {
            select = select.filter(payment::Column::SourceId.eq(id));
        }
        fetch_page(select, payment::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    pub(crate) async fn refunds_of(
        &self,
        tenant: &TenantContext,
        payment_id: Uuid,
    ) -> Result<Vec<refund::Model>, Error> {
        Ok(refund::Entity::find()
            .scoped(tenant)
            .filter(refund::Column::PaymentId.eq(payment_id))
            .order_by_asc(refund::Column::Id)
            .all(tenant.database().read())
            .await?)
    }

    // ---- Webhook -------------------------------------------------------------------------------

    /// Nhận webhook của đối tác `provider`: kiểm chữ ký trên thân thô, lưu nguyên văn và xếp job
    /// trong một transaction. Sai chữ ký → 401, không lưu gì. Gửi lại cùng mã thông báo → không làm
    /// gì thêm.
    pub(crate) async fn receive_webhook(
        &self,
        tenant: &TenantContext,
        provider: &str,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> Result<(), Error> {
        let gateway = self.gateway(Some(provider))?;
        let notification = match gateway.verify_webhook(tenant.id(), headers, body) {
            Ok(notification) => notification,
            Err(error) => {
                metrics::counter!("ledger_webhook_rejected_total", "provider" => gateway.provider())
                    .increment(1);
                tracing::warn!(provider = gateway.provider(), "payment webhook rejected");
                return Err(error);
            }
        };
        let stored = serde_json::to_string(&StoredNotification::of(&notification))
            .map_err(Error::internal)?;
        let text = String::from_utf8_lossy(body).into_owned();
        let (database, jobs, notification, provider) =
            (tenant.database(), &self.jobs, &notification, gateway.provider());
        database
            .write_with_retry(move || {
                let (stored, text) = (stored.clone(), text.clone());
                async move {
                    let transaction = database.begin_write().await?;
                    let id = Uuid::now_v7();
                    let inserted = gateway_event::Entity::insert(gateway_event::ActiveModel {
                        id: Set(id),
                        tenant_id: Set(tenant.id().as_uuid()),
                        provider: Set(provider.to_owned()),
                        provider_event_id: Set(notification.event_id.clone()),
                        kind: Set("payment".to_owned()),
                        body: Set(text),
                        notification: Set(stored),
                        status: Set("pending".to_owned()),
                        error: Set(None),
                        received_at: Set(now()),
                        processed_at: Set(None),
                    })
                    .on_conflict(
                        OnConflict::columns([
                            gateway_event::Column::TenantId,
                            gateway_event::Column::Provider,
                            gateway_event::Column::ProviderEventId,
                        ])
                        .do_nothing()
                        .to_owned(),
                    )
                    .exec_without_returning(&transaction)
                    .await?;
                    if inserted == 1 {
                        jobs.enqueue(
                            &transaction,
                            tenant,
                            &ProcessGatewayEvent { event_id: id },
                            EnqueueOptions::default().unique_key(id.to_string()),
                        )
                        .await?;
                    }
                    transaction.commit().await?;
                    Ok(())
                }
            })
            .await
    }

    /// Xử lý thông báo `event_id` đã lưu (job). Chạy lại vô hại.
    pub(crate) async fn process_gateway_event(
        &self,
        tenant: &TenantContext,
        event_id: Uuid,
    ) -> Result<(), Error> {
        let database = tenant.database();
        let Some(stored) = gateway_event::Entity::find_by_id(event_id)
            .scoped(tenant)
            .one(database.write())
            .await?
        else {
            return Ok(());
        };
        if stored.status != "pending" {
            return Ok(());
        }
        let notice: StoredNotification =
            serde_json::from_str(&stored.notification).map_err(Error::internal)?;
        let amount = Money::from_columns(notice.amount_minor, &notice.currency)?;
        let outcome = match notice.kind.as_str() {
            "payment_succeeded" => {
                let found = self.payment_by_ref(tenant, &stored.provider, &notice).await?;
                match found {
                    Some(row) => {
                        self.capture(tenant, row.id, amount, "webhook").await?;
                        "processed"
                    }
                    None => {
                        self.open_exception(
                            tenant,
                            "unmatched_credit",
                            &format!("ledger.gateway_event:{}", stored.id),
                            json!({"provider": stored.provider, "provider_ref": notice.provider_ref, "amount": amount}),
                            &format!("unmatched_credit:{}:{}", stored.provider, notice.provider_ref),
                        )
                        .await?;
                        "ignored"
                    }
                }
            }
            "payment_failed" => {
                if let Some(row) = self.payment_by_ref(tenant, &stored.provider, &notice).await? {
                    let reason = notice.reason.clone().unwrap_or_else(|| "declined".to_owned());
                    self.fail_payment(tenant, row.id, &reason).await?;
                }
                "processed"
            }
            "refund_succeeded" | "refund_failed" => {
                let reference = notice.refund_provider_ref.clone().unwrap_or_default();
                let found = refund::Entity::find()
                    .scoped(tenant)
                    .filter(refund::Column::ProviderRef.eq(reference))
                    .one(database.write())
                    .await?;
                if let Some(found) = found {
                    if notice.kind == "refund_succeeded" {
                        self.complete_refund(tenant, found.id).await?;
                    } else {
                        self.fail_refund(tenant, found.id).await?;
                    }
                }
                "processed"
            }
            _ => "ignored",
        };
        gateway_event::Entity::update_many()
            .col_expr(gateway_event::Column::Status, outcome.into())
            .col_expr(gateway_event::Column::ProcessedAt, Some(now()).into())
            .filter(gateway_event::Column::Id.eq(stored.id))
            .exec(database.write())
            .await?;
        Ok(())
    }

    async fn payment_by_ref(
        &self,
        tenant: &TenantContext,
        provider: &str,
        notice: &StoredNotification,
    ) -> Result<Option<payment::Model>, Error> {
        let write = tenant.database().write();
        let by_ref = payment::Entity::find()
            .scoped(tenant)
            .filter(payment::Column::Provider.eq(provider))
            .filter(payment::Column::ProviderRef.eq(notice.provider_ref.as_str()))
            .one(write)
            .await?;
        if by_ref.is_some() {
            return Ok(by_ref);
        }
        match notice.payment_id {
            Some(id) => Ok(Self::find_payment(write, tenant, id)
                .await?
                .filter(|row| row.provider == provider)),
            None => Ok(None),
        }
    }

    /// Đối tác đã thu `received` cho thanh toán `id` (`via`: `webhook`, `reconcile`, `manual`).
    /// Đúng số tiền → ghi bút toán (theo `reference` ổn định, gọi lại không ghi trùng), chuyển
    /// `succeeded`, phát `ledger.payment_succeeded`. Sai số → `needs_review` + hàng chờ.
    pub(crate) async fn capture(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        received: Money,
        via: &str,
    ) -> Result<(), Error> {
        let database = tenant.database();
        let current = Self::find_payment(database.write(), tenant, id)
            .await?
            .ok_or_else(payment_not_found)?;
        if current.status == "succeeded" {
            return Ok(());
        }
        let expected = money(current.amount_minor, &current.amount_currency)?;
        if received != expected && via != "manual" {
            self.mark_needs_review(tenant, &current, received).await?;
            return Ok(());
        }
        // Bút toán trước (gọi lại an toàn), trạng thái sau: lỗi giữa chừng thì job chạy lại.
        let book = OrgId::from_uuid(current.org_id);
        // Chuyển khoản VietQR: tiền đã vào thẳng tài khoản ngân hàng của nút (tự thu); qua cổng:
        // tiền nằm ở đối tác (`clearing.gateway`).
        let clearing = match current.bank_account_id {
            Some(account) => AccountKey::new(book, "cash.bank", received.currency())
                .with_counterparty(Counterparty::new("bank_account", account.to_string())),
            None => AccountKey::new(book, "clearing.gateway", received.currency())
                .with_counterparty(Counterparty::new("partner", current.provider.clone())),
        };
        let receivable = AccountKey::new(book, "receivable.customer", received.currency());
        let entry = NewEntry::new(
            book,
            "ledger",
            "payment",
            format!("payment:{id}:captured"),
            "Payment collected by the partner",
            vec![
                NewLine::debit(LineAccount::Key(clearing), received),
                NewLine::credit(LineAccount::Key(receivable), received),
            ],
        )
        .source_id(id.to_string());
        self.post(tenant, Actor::system(), entry).await?;
        let late = current.status == "expired";
        let outbox = &self.outbox;
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let Some(row) =
                    rows::lock_one(&transaction, payment::Entity::find_by_id(id).scoped(tenant))
                        .await?
                else {
                    return Err(payment_not_found());
                };
                if row.status == "succeeded" {
                    return Ok(());
                }
                let at = now();
                let mut active: payment::ActiveModel = row.into();
                active.status = Set("succeeded".to_owned());
                active.captured_at = Set(Some(at));
                active.updated_at = Set(at);
                let saved = payment::Entity::update(active).exec(&transaction).await?;
                let event = events::PaymentSucceeded(payment_payload(&saved)?);
                outbox
                    .record(&transaction, tenant, Actor::system(), payment_subject(id), &event)
                    .await?;
                transaction.commit().await?;
                Ok(())
            })
            .await?;
        if late {
            // Tiền về sau khi đã hết hạn: vẫn ghi (tiền đã ở đối tác), mở hàng chờ để người quyết
            // giữ hay hoàn.
            self.open_exception(
                tenant,
                "late_payment",
                &format!("ledger.payment:{id}"),
                json!({"payment_id": id, "amount": received}),
                &format!("late_payment:{id}"),
            )
            .await?;
        }
        Ok(())
    }

    async fn mark_needs_review(
        &self,
        tenant: &TenantContext,
        current: &payment::Model,
        received: Money,
    ) -> Result<(), Error> {
        payment::Entity::update_many()
            .col_expr(payment::Column::Status, "needs_review".into())
            .col_expr(payment::Column::UpdatedAt, now().into())
            .filter(payment::Column::Id.eq(current.id))
            .filter(payment::Column::Status.is_in(["pending", "expired"]))
            .exec(tenant.database().write())
            .await?;
        self.open_exception(
            tenant,
            "amount_mismatch",
            &format!("ledger.payment:{}", current.id),
            json!({
                "payment_id": current.id,
                "expected": money(current.amount_minor, &current.amount_currency)?,
                "received": received, "received_minor": received.minor(),
            }),
            &format!("amount_mismatch:{}", current.id),
        )
        .await
    }

    async fn fail_payment(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        reason: &str,
    ) -> Result<(), Error> {
        let database = tenant.database();
        let outbox = &self.outbox;
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let Some(row) =
                    rows::lock_one(&transaction, payment::Entity::find_by_id(id).scoped(tenant))
                        .await?
                else {
                    return Ok(());
                };
                // Chỉ đi tới, không lùi: đã thu thì thông báo "thất bại" đến muộn bị bỏ qua.
                if row.status != "pending" {
                    return Ok(());
                }
                let status = if reason == "expired" { "expired" } else { "failed" };
                let mut active: payment::ActiveModel = row.into();
                active.status = Set(status.to_owned());
                active.updated_at = Set(now());
                let saved = payment::Entity::update(active).exec(&transaction).await?;
                let mut payload = payment_payload(&saved)?;
                payload["reason"] = Value::from(reason);
                let event = events::PaymentFailed(payload);
                outbox
                    .record(&transaction, tenant, Actor::system(), payment_subject(id), &event)
                    .await?;
                transaction.commit().await?;
                Ok(())
            })
            .await
    }

    /// Thanh toán `pending` quá hạn trên `database` → `expired` (job mỗi phút). Trả số đã đổi.
    pub(crate) async fn expire_payments(
        &self,
        database: &autoxide_kernel::database::Database,
    ) -> Result<u64, Error> {
        let due: Vec<(Uuid, Uuid)> = payment::Entity::find()
            .select_only()
            .column(payment::Column::TenantId)
            .column(payment::Column::Id)
            .filter(payment::Column::Status.eq("pending"))
            .filter(payment::Column::ExpiresAt.lt(Utc::now()))
            .limit(500)
            .into_tuple()
            .all(database.read())
            .await?;
        let mut expired = 0;
        for (tenant, id) in due {
            let tenant = self
                .databases
                .tenant(autoxide_kernel::tenancy::TenantId::from_uuid(tenant))
                .await?;
            self.fail_payment(&tenant, id, "expired").await?;
            expired += 1;
        }
        Ok(expired)
    }

    // ---- Hoàn tiền -----------------------------------------------------------------------------

    /// Hoàn (một phần) thanh toán (`dyn Ledger`).
    pub(crate) async fn refund(
        &self,
        tenant: &TenantContext,
        _actor: Actor,
        input: NewRefund,
    ) -> Result<Submitted<Refund>, Error> {
        let mut problems = Vec::new();
        let reference = Reference::checked(&input.reference, "reference", &mut problems);
        if !input.amount.is_positive() {
            problems.push(FieldError::new("amount", "range", "must be greater than zero"));
        }
        let reason = input.reason.trim().to_owned();
        if !(1..=200).contains(&reason.chars().count()) {
            problems.push(FieldError::new(
                "reason",
                "length",
                "must be 1 to 200 characters long (no personal data)",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let Some(reference) = reference else {
            return Err(Error::internal("a checked reference went missing"));
        };
        let found = Self::find_payment(tenant.database().read(), tenant, input.payment)
            .await?
            .ok_or_else(payment_not_found)?;
        let gateway = self.gateway(Some(&found.provider))?;
        let capabilities = gateway.capabilities();
        if !capabilities.refunds
            || (!capabilities.partial_refunds && input.amount.minor() != found.amount_minor)
        {
            return Err(autoxide_kernel::payment::not_supported(gateway.provider()));
        }
        if input.amount.currency().code() != found.amount_currency {
            return Err(crate::service::currency_mixed());
        }
        let fingerprint = reference::fingerprint(&json!({
            "payment": input.payment, "amount": input.amount.minor(), "reason": reason,
        }))?;
        let database = tenant.database();
        let (reference, fingerprint, input, reason) = (&reference, &fingerprint, &input, &reason);
        let submitted = reference::submit_once(database, move || async move {
            let transaction = database.begin_write().await?;
            let stored = refund::Entity::find()
                .scoped(tenant)
                .filter(refund::Column::PaymentId.eq(input.payment))
                .filter(refund::Column::Reference.eq(reference.as_str()))
                .one(&transaction)
                .await?
                .map(|row| {
                    let fingerprint = row.reference_fingerprint.clone();
                    (row, fingerprint)
                });
            if let Some(existing) = reference::replayed(stored, reference, fingerprint)? {
                return Ok(Submitted::Replayed(existing));
            }
            let row = rows::lock_one(
                &transaction,
                payment::Entity::find_by_id(input.payment).scoped(tenant),
            )
            .await?
            .ok_or_else(payment_not_found)?;
            if row.status != "succeeded" {
                return Err(payment_state(&row.status));
            }
            // Giữ chỗ số hoàn trong một câu: hai lượt hoàn cùng lúc không vượt số đã thu.
            let changed = rows::adjust::<payment::Entity, _>(
                &transaction,
                payment::Column::RefundedMinor,
                payment::Column::Id.eq(row.id),
                input.amount.minor(),
                Bounds { floor: 0, ceiling: row.amount_minor },
            )
            .await?;
            if changed == 0 {
                let remaining = row.amount_minor.saturating_sub(row.refunded_minor);
                return Err(refund_exceeds(money(remaining, &row.amount_currency)?));
            }
            let at = now();
            let saved = refund::Entity::insert(refund::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                payment_id: Set(row.id),
                amount_minor: Set(input.amount.minor()),
                amount_currency: Set(row.amount_currency.clone()),
                status: Set("pending".to_owned()),
                provider_ref: Set(None),
                reason: Set(reason.clone()),
                reference: Set(reference.as_str().to_owned()),
                reference_fingerprint: Set(fingerprint.clone()),
                created_at: Set(at),
                updated_at: Set(at),
            })
            .exec_with_returning(&transaction)
            .await?;
            transaction.commit().await?;
            Ok(Submitted::Created(saved))
        })
        .await?;
        let created = matches!(submitted, Submitted::Created(_));
        let mut row = submitted.into_inner();
        if row.status == "pending" && row.provider_ref.is_none() {
            let provider_ref = found.provider_ref.clone().unwrap_or_default();
            let amount = money(row.amount_minor, &row.amount_currency)?;
            let intent = RefundIntent::new(row.id, provider_ref, amount, &row.reason);
            let answer = gateway.refund(tenant.id(), &intent).await?;
            refund::Entity::update_many()
                .col_expr(refund::Column::ProviderRef, Some(answer.provider_ref.clone()).into())
                .col_expr(refund::Column::UpdatedAt, now().into())
                .filter(refund::Column::Id.eq(row.id))
                .filter(refund::Column::ProviderRef.is_null())
                .exec(database.write())
                .await?;
            match answer.status {
                RefundStatus::Succeeded => self.complete_refund(tenant, row.id).await?,
                RefundStatus::Failed => self.fail_refund(tenant, row.id).await?,
                RefundStatus::Pending => {}
            }
            row = refund::Entity::find_by_id(row.id)
                .scoped(tenant)
                .one(database.write())
                .await?
                .ok_or_else(|| Error::internal("a refund vanished"))?;
        }
        let described = describe_refund(&row)?;
        Ok(if created { Submitted::Created(described) } else { Submitted::Replayed(described) })
    }

    /// Hoàn qua REST / MCP: cần `ledger.refunds_create` ở nút nhận tiền.
    pub(crate) async fn refund_manual(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewRefund,
    ) -> Result<Submitted<Refund>, Error> {
        self.payment_for(tenant, principal, &REFUNDS_CREATE, input.payment).await?;
        self.refund(tenant, Actor::from(principal), input).await
    }

    /// Đối tác đã hoàn: ghi bút toán, `succeeded`, event. Gọi lại vô hại.
    async fn complete_refund(&self, tenant: &TenantContext, id: Uuid) -> Result<(), Error> {
        let database = tenant.database();
        let Some(current) =
            refund::Entity::find_by_id(id).scoped(tenant).one(database.write()).await?
        else {
            return Ok(());
        };
        if current.status != "pending" {
            return Ok(());
        }
        let paid = Self::find_payment(database.write(), tenant, current.payment_id)
            .await?
            .ok_or_else(payment_not_found)?;
        let amount = money(current.amount_minor, &current.amount_currency)?;
        let book = OrgId::from_uuid(paid.org_id);
        let clearing = AccountKey::new(book, "clearing.gateway", amount.currency())
            .with_counterparty(Counterparty::new("partner", paid.provider.clone()));
        let receivable = AccountKey::new(book, "receivable.customer", amount.currency());
        let entry = NewEntry::new(
            book,
            "ledger",
            "refund",
            format!("refund:{id}"),
            "Payment refunded by the partner",
            vec![
                NewLine::debit(LineAccount::Key(receivable), amount),
                NewLine::credit(LineAccount::Key(clearing), amount),
            ],
        )
        .source_id(paid.id.to_string());
        self.post(tenant, Actor::system(), entry).await?;
        let (outbox, paid) = (&self.outbox, &paid);
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let Some(row) =
                    rows::lock_one(&transaction, refund::Entity::find_by_id(id).scoped(tenant))
                        .await?
                else {
                    return Ok(());
                };
                if row.status != "pending" {
                    return Ok(());
                }
                let mut active: refund::ActiveModel = row.clone().into();
                active.status = Set("succeeded".to_owned());
                active.updated_at = Set(now());
                refund::Entity::update(active).exec(&transaction).await?;
                let event = events::PaymentRefunded(json!({
                    "payment_id": row.payment_id, "refund_id": row.id,
                    "amount": money(row.amount_minor, &row.amount_currency)?,
                    "source_module": paid.source_module, "source_id": paid.source_id,
                }));
                outbox
                    .record(
                        &transaction,
                        tenant,
                        Actor::system(),
                        payment_subject(row.payment_id),
                        &event,
                    )
                    .await?;
                transaction.commit().await?;
                Ok(())
            })
            .await
    }

    /// Đối tác từ chối hoàn: `failed`, trả lại số đã giữ chỗ.
    async fn fail_refund(&self, tenant: &TenantContext, id: Uuid) -> Result<(), Error> {
        let database = tenant.database();
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let Some(row) =
                    rows::lock_one(&transaction, refund::Entity::find_by_id(id).scoped(tenant))
                        .await?
                else {
                    return Ok(());
                };
                if row.status != "pending" {
                    return Ok(());
                }
                rows::adjust::<payment::Entity, _>(
                    &transaction,
                    payment::Column::RefundedMinor,
                    payment::Column::Id.eq(row.payment_id),
                    -row.amount_minor,
                    Bounds::NON_NEGATIVE,
                )
                .await?;
                let mut active: refund::ActiveModel = row.into();
                active.status = Set("failed".to_owned());
                active.updated_at = Set(now());
                refund::Entity::update(active).exec(&transaction).await?;
                transaction.commit().await?;
                Ok(())
            })
            .await
    }
}

/// Job `ledger.process_gateway_event`.
pub(crate) struct ProcessGatewayEventJob(pub Arc<LedgerService>);

#[async_trait::async_trait]
impl JobHandler for ProcessGatewayEventJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let payload: ProcessGatewayEvent =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        let tenant = job
            .tenant
            .as_ref()
            .ok_or_else(|| Error::internal("ledger.process_gateway_event needs a tenant"))?;
        self.0.process_gateway_event(tenant, payload.event_id).await
    }
}

/// Job `ledger.expire_payments` (mỗi phút, mọi database).
pub(crate) struct ExpirePaymentsJob(pub Arc<LedgerService>);

#[async_trait::async_trait]
impl JobHandler for ExpirePaymentsJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.expire_payments(&job.database).await.map(|_| ())
    }
}
