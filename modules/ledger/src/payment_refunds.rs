//! Hoàn tiền một thanh toán qua đối tác (`dyn Ledger::refund`, REST / MCP): giữ chỗ số hoàn, gọi đối tác,
//! ghi kết quả; lượt hoàn bị từ chối mở mục `refund_failed` ở hàng chờ đối soát. Tách khỏi `payments.rs`
//! theo trách nhiệm (chỉ di chuyển code).

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::payment::{RefundIntent, RefundStatus};
use autoxide_kernel::reference::{self, Reference, Submitted};
use autoxide_kernel::rows::{self, Bounds};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{AccountKey, Counterparty, LineAccount, NewEntry, NewLine, NewRefund, Refund};
use crate::entity::{exception, payment, refund};
use crate::events;
use crate::payments::{
    collected, describe_refund, payment_not_found, payment_state, payment_subject, refund_exceeds,
};
use crate::permissions::REFUNDS_CREATE;
use crate::service::{LedgerService, money, now};

impl LedgerService {
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
        match self.refund_found(tenant, &found, &input, &reference, &reason).await {
            // Từ chối hẳn ở bất cứ bước nào sau khi đã có thanh toán (chưa cấu hình đối tác, đối tác không hoàn,
            // sai tiền tệ, vượt số còn lại, đối tác từ chối): tiền có thể vẫn ở đối tác — mở `refund_failed`
            // (rà #591 C2). 409 / 429 / 5xx không phải từ chối: bên gọi gửi lại cùng mã.
            Err(error) if crate::api::refund_refused(&error) => {
                let details = json!({"payment_id": found.id, "reference": reference.as_str(),
                    "amount": input.amount, "reason": reason, "code": error.code()});
                self.open_refund_failed(tenant, found.id, reference.as_str(), &details).await?;
                Err(error)
            }
            other => other,
        }
    }

    /// Phần của [`LedgerService::refund`] sau khi đã tìm thấy thanh toán `found`.
    async fn refund_found(
        &self,
        tenant: &TenantContext,
        found: &payment::Model,
        input: &NewRefund,
        reference: &Reference,
        reason: &String,
    ) -> Result<Submitted<Refund>, Error> {
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
        let fingerprint = &fingerprint;
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
                Bounds { floor: 0, ceiling: collected(&row) },
            )
            .await?;
            if changed == 0 {
                let remaining = collected(&row).saturating_sub(row.refunded_minor);
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
            let answer = match gateway.refund(tenant.id(), &intent).await {
                Ok(answer) => answer,
                // Đối tác từ chối hẳn (4xx trừ 409 / 429): lượt hoàn `failed`, trả lại phần đã giữ chỗ — không
                // để dòng `pending` giữ `refunded_minor` mãi. Lỗi tạm (5xx, 409, 429) giữ `pending` để gửi
                // lại cùng mã.
                Err(error) if crate::api::refund_refused(&error) => {
                    self.fail_refund(tenant, row.id).await?;
                    return Err(error);
                }
                Err(error) => return Err(error),
            };
            refund::Entity::update_many()
                .scoped(tenant)
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
    pub(crate) async fn complete_refund(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<(), Error> {
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
                crate::reconcile::close_late_payment_if_refunded(
                    &transaction,
                    tenant,
                    outbox,
                    paid,
                )
                .await?;
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

    /// Đối tác từ chối hoàn: `failed`, trả lại số đã giữ chỗ, mở mục `refund_failed` ở hàng chờ đối soát.
    pub(crate) async fn fail_refund(&self, tenant: &TenantContext, id: Uuid) -> Result<(), Error> {
        let database = tenant.database();
        let outbox = &self.outbox;
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
                let (refund_id, payment_id) = (row.id, row.payment_id);
                let amount = money(row.amount_minor, &row.amount_currency)?;
                let (reason, reference) = (row.reason.clone(), row.reference.clone());
                let mut active: refund::ActiveModel = row.into();
                active.status = Set("failed".to_owned());
                active.updated_at = Set(now());
                refund::Entity::update(active).exec(&transaction).await?;
                // Tiền vẫn ở đối tác mà lượt hoàn hỏng: người vận hành phải thấy (UT-6).
                let details = json!({"payment_id": payment_id, "refund_id": refund_id,
                    "reference": reference, "amount": amount, "reason": reason});
                open_refund_failed(&transaction, tenant, outbox, payment_id, &reference, &details)
                    .await?;
                transaction.commit().await?;
                Ok(())
            })
            .await
    }

    /// [`open_refund_failed`] trong transaction riêng (lượt hoàn bị từ chối trước khi có dòng hoàn).
    async fn open_refund_failed(
        &self,
        tenant: &TenantContext,
        payment_id: Uuid,
        reference: &str,
        details: &Value,
    ) -> Result<(), Error> {
        let database = tenant.database();
        let outbox = &self.outbox;
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                open_refund_failed(&transaction, tenant, outbox, payment_id, reference, details)
                    .await?;
                transaction.commit().await?;
                Ok(())
            })
            .await
    }
}

/// Trong `transaction`: lượt hoàn mã `reference` của thanh toán `payment_id` bị từ chối — mở mục
/// `refund_failed` (một lần cho mỗi mã) để người vận hành hoàn tay. Không mở khi không còn gì để hoàn (đã hoàn
/// đủ), khi một lượt hoàn khác cùng mã đang chờ hay đã xong (mã dùng lại với nội dung khác), hay khi mục
/// `late_payment` của thanh toán còn mở (nó đã bảo hoàn tay — một mục cho một khoản tiền).
async fn open_refund_failed(
    transaction: &impl ConnectionTrait,
    tenant: &TenantContext,
    outbox: &autoxide_kernel::event::Outbox,
    payment_id: Uuid,
    reference: &str,
    details: &Value,
) -> Result<(), Error> {
    let Some(paid) =
        payment::Entity::find_by_id(payment_id).scoped(tenant).one(transaction).await?
    else {
        return Ok(());
    };
    if collected(&paid) <= paid.refunded_minor {
        return Ok(());
    }
    let live = refund::Entity::find()
        .filter(refund::Column::PaymentId.eq(payment_id))
        .filter(refund::Column::Reference.eq(reference))
        .filter(refund::Column::Status.ne("failed"))
        .scoped(tenant)
        .one(transaction)
        .await?
        .is_some();
    let late_open = exception::Entity::find()
        .filter(exception::Column::DedupeKey.eq(crate::reconcile::late_payment_key(payment_id)))
        .filter(exception::Column::Status.eq("open"))
        .scoped(tenant)
        .one(transaction)
        .await?
        .is_some();
    if live || late_open {
        return Ok(());
    }
    crate::reconcile::insert_exception(
        transaction,
        tenant,
        outbox,
        crate::reconcile::REFUND_FAILED,
        &format!("ledger.payment:{payment_id}"),
        details,
        &crate::reconcile::refund_failed_key(payment_id, reference),
    )
    .await
}
