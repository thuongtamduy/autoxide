//! Chi trả người bán (bước 36 mục 4): số dư khả dụng (`payable.seller_available`) → đối tác chi hộ
//! (`PayoutGateway`) trả từ tài khoản thu hộ. Chỉ ở chế độ thu hộ — core không giữ tiền (D-187).
//!
//! Một transaction ghi lượt chi (`pending`) và bút toán Nợ `payable.seller_available[S]` / Có
//! `clearing.gateway[đối tác]` — tài khoản khả dụng có sàn 0 nên gửi đồng thời không chi quá số dư.
//! Sau commit mới gọi đối tác (ngoài transaction); đối tác từ chối → bút toán ngược lại, số dư trở về.
//! Đối tác không trả lời → giữ `pending`: gửi lại cùng `reference`, hoặc lịch `ledger.check_payouts`
//! gửi lại (đối tác chống trùng theo ID lượt chi) và hỏi trạng thái lượt `sent`.

use std::sync::Arc;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::payment::{PayoutGateway, PayoutIntent, PayoutStatus};
use autoxide_kernel::reference::{self, Reference, Submitted};
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, TenantId};
use chrono::{Duration, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect, Set};
use serde_json::json;
use uuid::Uuid;

use crate::api::{
    AccountKey, Counterparty, FlowMode, LineAccount, NewEntry, NewLine, NewPayout, Payout,
};
use crate::entity::payout;
use crate::events;
use crate::flows::flow_mode_unsupported;
use crate::permissions::{PAYOUTS_CREATE, READ};
use crate::posting::{draft_for, write_in};
use crate::service::{LedgerService, money, now};

pub(crate) const CHECK_PAYOUTS: &str = "ledger.check_payouts";

pub(crate) fn payout_not_found() -> Error {
    Error::not_found("ledger.payout_not_found", "There is no such payout.")
}

fn payout_not_configured(provider: &str) -> Error {
    Error::new(
        autoxide_kernel::error::ErrorKind::Validation,
        "ledger.payout_not_configured",
        "The payout partner `{provider}` is not set up for this unit or on this server.",
    )
    .with_param("provider", provider)
}

pub(crate) fn describe_payout(row: &payout::Model) -> Result<Payout, Error> {
    Ok(Payout {
        id: row.id,
        book_id: row.book_id,
        org_id: row.org_id,
        payee_org_id: row.payee_org_id,
        amount: money(row.amount_minor, &row.amount_currency)?,
        provider: row.provider.clone(),
        provider_ref: row.provider_ref.clone(),
        status: row.status.clone(),
        failure_reason: row.failure_reason.clone(),
        description: row.description.clone(),
        reference: row.reference.clone(),
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// Bộ lọc danh sách lượt chi.
#[derive(Debug, Clone, Default)]
pub(crate) struct PayoutFilter {
    pub status: Option<String>,
    pub payee_org_id: Option<Uuid>,
}

fn seller_available(book: OrgId, payee: Uuid, amount: Money) -> LineAccount {
    LineAccount::Key(
        AccountKey::new(book, "payable.seller_available", amount.currency())
            .with_counterparty(Counterparty::new("org", payee.to_string())),
    )
}

fn clearing(book: OrgId, provider: &str, amount: Money) -> LineAccount {
    LineAccount::Key(
        AccountKey::new(book, "clearing.gateway", amount.currency())
            .with_counterparty(Counterparty::new("partner", provider.to_owned())),
    )
}

impl LedgerService {
    pub(crate) fn payout_gateway(&self, provider: &str) -> Result<Arc<dyn PayoutGateway>, Error> {
        self.payout_gateways
            .iter()
            .find(|gateway| gateway.provider() == provider)
            .cloned()
            .ok_or_else(|| payout_not_configured(provider))
    }

    /// Chi trả qua REST / MCP: cần `ledger.payouts_create` ở nút người nhận.
    pub(crate) async fn create_payout_manual(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewPayout,
    ) -> Result<Submitted<Payout>, Error> {
        principal.require(&PAYOUTS_CREATE)?;
        principal
            .require_at(
                tenant.database().read(),
                &PAYOUTS_CREATE,
                OrgTarget::new(input.payee, None),
            )
            .await?;
        self.create_payout(tenant, Actor::from(principal), input).await
    }

    /// Chi trả (`dyn Ledger`).
    pub(crate) async fn create_payout(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        input: NewPayout,
    ) -> Result<Submitted<Payout>, Error> {
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
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let Some(reference) = reference else {
            return Err(Error::internal("a checked reference went missing"));
        };
        let settings = self.flow_settings(tenant, input.payee).await?;
        let (FlowMode::PartnerCollect, Some(operator)) = (settings.mode, settings.source_org_id)
        else {
            return Err(flow_mode_unsupported(settings.mode, "-"));
        };
        let provider = settings.payout_provider.clone().unwrap_or_default();
        let gateway = self.payout_gateway(&provider)?;
        let book_org = OrgId::from_uuid(operator);
        let book = self.ensure_book(tenant, book_org).await?;
        let day = self.today(tenant, book_org).await;
        let fingerprint = reference::fingerprint(&json!({
            "payee": input.payee.as_uuid(), "amount": input.amount.minor(),
            "currency": input.amount.currency().code(), "description": description,
        }))?;
        let (database, outbox) = (tenant.database(), &self.outbox);
        let (reference, fingerprint, input, book, description, provider) =
            (&reference, &fingerprint, &input, &book, &description, gateway.provider());
        let submitted = reference::submit_once(database, move || async move {
            let transaction = database.begin_write().await?;
            let stored = payout::Entity::find()
                .scoped(tenant)
                .filter(payout::Column::Reference.eq(reference.as_str()))
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
            let at = now();
            let payee = input.payee.as_uuid();
            let entry = NewEntry::new(
                book_org,
                "ledger",
                "payout",
                format!("payout:{id}"),
                "Seller payout sent to the payout partner",
                vec![
                    NewLine::debit(seller_available(book_org, payee, input.amount), input.amount),
                    NewLine::credit(clearing(book_org, provider, input.amount), input.amount),
                ],
            )
            .source_id(id.to_string());
            let draft = draft_for(entry, book.clone(), day, None, None)?;
            // Số dư khả dụng có sàn 0: không đủ → `ledger.insufficient_balance`, cả lượt chi bỏ.
            write_in(&transaction, tenant, actor, &draft, outbox).await?;
            let saved = payout::Entity::insert(payout::ActiveModel {
                id: Set(id),
                tenant_id: Set(tenant.id().as_uuid()),
                book_id: Set(book.id),
                org_id: Set(book.org_id),
                payee_org_id: Set(payee),
                amount_minor: Set(input.amount.minor()),
                amount_currency: Set(input.amount.currency().code().to_owned()),
                provider: Set(provider.to_owned()),
                provider_ref: Set(None),
                status: Set("pending".to_owned()),
                failure_reason: Set(None),
                description: Set(description.clone()),
                reference: Set(reference.as_str().to_owned()),
                reference_fingerprint: Set(fingerprint.clone()),
                created_by: Set(actor.user_id),
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
        let row = submitted.into_inner();
        let row = if row.status == "pending" { self.send_payout(tenant, &row).await? } else { row };
        let described = describe_payout(&row)?;
        Ok(if created { Submitted::Created(described) } else { Submitted::Replayed(described) })
    }

    /// Gửi (lại) lệnh chi tới đối tác, ngoài transaction; ghi kết quả.
    async fn send_payout(
        &self,
        tenant: &TenantContext,
        row: &payout::Model,
    ) -> Result<payout::Model, Error> {
        let gateway = self.payout_gateway(&row.provider)?;
        let amount = money(row.amount_minor, &row.amount_currency)?;
        let intent =
            PayoutIntent::new(row.id, row.payee_org_id.to_string(), amount, &row.description);
        let answer = gateway.payout(tenant.id(), &intent).await?;
        self.apply_payout(tenant, row.id, Some(answer.provider_ref), answer.status).await
    }

    /// Ghi kết quả của đối tác cho lượt chi `id` (chỉ đi tới: đã xong thì giữ nguyên).
    async fn apply_payout(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        provider_ref: Option<String>,
        status: PayoutStatus,
    ) -> Result<payout::Model, Error> {
        let database = tenant.database();
        let current = payout::Entity::find_by_id(id)
            .scoped(tenant)
            .one(database.read())
            .await?
            .ok_or_else(payout_not_found)?;
        let book_org = OrgId::from_uuid(current.org_id);
        let book = self.ensure_book(tenant, book_org).await?;
        let day = self.today(tenant, book_org).await;
        let (outbox, book, provider_ref, status) = (&self.outbox, &book, &provider_ref, &status);
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let row =
                    rows::lock_one(&transaction, payout::Entity::find_by_id(id).scoped(tenant))
                        .await?
                        .ok_or_else(payout_not_found)?;
                if row.status == "succeeded" || row.status == "failed" {
                    return Ok(row);
                }
                let amount = money(row.amount_minor, &row.amount_currency)?;
                let mut active: payout::ActiveModel = row.clone().into();
                if let Some(found) = provider_ref {
                    active.provider_ref = Set(Some(found.clone()));
                }
                let subject = Some(format!("ledger.payout:{id}"));
                let payload = json!({
                    "payout_id": id, "org_id": row.org_id, "payee_org_id": row.payee_org_id,
                    "amount": amount, "provider": row.provider,
                });
                match status {
                    PayoutStatus::Pending => active.status = Set("sent".to_owned()),
                    PayoutStatus::Succeeded => {
                        active.status = Set("succeeded".to_owned());
                        let event = events::PayoutSent(payload);
                        outbox
                            .record(&transaction, tenant, Actor::system(), subject, &event)
                            .await?;
                    }
                    PayoutStatus::Failed { reason } => {
                        // Trả lại số dư khả dụng: bút toán ngược với lúc chi.
                        let entry = NewEntry::new(
                            book_org,
                            "ledger",
                            "payout_failed",
                            format!("payout:{id}:failed"),
                            "Seller payout refused by the partner; balance restored",
                            vec![
                                NewLine::debit(clearing(book_org, &row.provider, amount), amount),
                                NewLine::credit(
                                    seller_available(book_org, row.payee_org_id, amount),
                                    amount,
                                ),
                            ],
                        )
                        .source_id(id.to_string());
                        let draft = draft_for(entry, book.clone(), day, None, None)?;
                        write_in(&transaction, tenant, Actor::system(), &draft, outbox).await?;
                        let reason: String = reason.chars().take(200).collect();
                        active.status = Set("failed".to_owned());
                        active.failure_reason = Set(Some(reason.clone()));
                        let mut payload = payload;
                        payload["reason"] = reason.into();
                        let event = events::PayoutFailed(payload);
                        outbox
                            .record(&transaction, tenant, Actor::system(), subject, &event)
                            .await?;
                    }
                }
                active.updated_at = Set(now());
                let saved = payout::Entity::update(active).exec(&transaction).await?;
                transaction.commit().await?;
                Ok(saved)
            })
            .await
    }

    pub(crate) async fn payout_for(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<payout::Model, Error> {
        principal.require(&READ)?;
        let read = tenant.database().read();
        let found = payout::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(payout_not_found)?;
        principal
            .require_at(read, &READ, OrgTarget::new(OrgId::from_uuid(found.org_id), None))
            .await
            .map_err(
                |error| if error.code() == "org_out_of_scope" { payout_not_found() } else { error },
            )?;
        Ok(found)
    }

    pub(crate) async fn list_payouts(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: PayoutFilter,
        page: &PageRequest,
    ) -> Result<Page<payout::Model>, Error> {
        let condition = principal.scope_condition(&READ, payout::Column::OrgId, None)?;
        let mut select = payout::Entity::find().scoped(tenant).filter(condition);
        if let Some(status) = filter.status {
            select = select.filter(payout::Column::Status.eq(status));
        }
        if let Some(payee) = filter.payee_org_id {
            select = select.filter(payout::Column::PayeeOrgId.eq(payee));
        }
        fetch_page(select, payout::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Lịch `ledger.check_payouts`: gửi lại lượt `pending` cũ hơn một phút (đối tác chống trùng theo
    /// ID), hỏi trạng thái lượt `sent`. Lỗi của một lượt không chặn lượt khác.
    pub(crate) async fn check_payouts(
        &self,
        database: &autoxide_kernel::database::Database,
    ) -> Result<u64, Error> {
        let stale = Utc::now() - Duration::minutes(1);
        let due: Vec<(Uuid, Uuid)> = payout::Entity::find()
            .select_only()
            .column(payout::Column::TenantId)
            .column(payout::Column::Id)
            .filter(payout::Column::Status.is_in(["pending", "sent"]))
            .filter(payout::Column::UpdatedAt.lt(stale))
            .limit(200)
            .into_tuple()
            .all(database.read())
            .await?;
        let mut done = 0;
        for (tenant, id) in due {
            let tenant = self.databases.tenant(TenantId::from_uuid(tenant)).await?;
            let outcome = self.check_payout(&tenant, id).await;
            match outcome {
                Ok(()) => done += 1,
                Err(error) => {
                    tracing::warn!(tenant_id = %tenant.id(), payout_id = %id, error = %error, "cannot check a payout");
                }
            }
        }
        Ok(done)
    }

    async fn check_payout(&self, tenant: &TenantContext, id: Uuid) -> Result<(), Error> {
        let Some(row) =
            payout::Entity::find_by_id(id).scoped(tenant).one(tenant.database().read()).await?
        else {
            return Ok(());
        };
        match (row.status.as_str(), row.provider_ref.as_deref()) {
            ("pending", _) => {
                self.send_payout(tenant, &row).await?;
            }
            ("sent", Some(provider_ref)) => {
                let gateway = self.payout_gateway(&row.provider)?;
                let status = gateway.payout_status(tenant.id(), provider_ref).await?;
                if status != PayoutStatus::Pending {
                    self.apply_payout(tenant, id, None, status).await?;
                } else {
                    // Chưa xong: dời mốc để lượt sau hỏi lại, không quét mãi một dòng.
                    payout::Entity::update_many()
                        .col_expr(payout::Column::UpdatedAt, now().into())
                        .filter(payout::Column::Id.eq(id))
                        .filter(payout::Column::Status.eq("sent"))
                        .exec(tenant.database().write())
                        .await?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// Lịch `ledger.check_payouts` (5 phút, mọi database).
pub(crate) struct CheckPayoutsJob(pub Arc<LedgerService>);

#[async_trait::async_trait]
impl JobHandler for CheckPayoutsJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.check_payouts(&job.database).await.map(|_| ())
    }
}
