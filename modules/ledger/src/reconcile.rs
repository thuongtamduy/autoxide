//! Đối soát sổ với sao kê của đối tác, và hàng chờ sai lệch cần người xử lý (bước 36 mục 5).
//!
//! Đối soát một ngày (`POST /reconciliations`, lịch `ledger.reconcile` mỗi ngày cho hôm qua): kéo
//! sao kê → `ledger_statement_lines` (unique theo mã dòng, chạy lại không chép trùng) → khớp theo
//! mã giao dịch + số tiền. Thanh toán còn `pending` mà sao kê đã có tiền (webhook bị lạc) thì ghi
//! như đã thu; lệch số / không biết giao dịch / thiếu trong sao kê → `ledger_exceptions`. Phí trong
//! sao kê → bút toán Nợ `expense.gateway_fee` / Có `clearing.gateway`.

use autoxide_kernel::time::{Date, Timestamp};
use std::collections::HashSet;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::payment::StatementKind;
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, TenantId};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect, Set};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{AccountKey, Counterparty, LineAccount, NewEntry, NewLine};
use crate::entity::{exception, payment, reconciliation, refund, statement_line};
use crate::events;
use crate::permissions::{READ, RECONCILE};
use crate::service::{LedgerService, now};

pub(crate) const RECONCILE_JOB: &str = "ledger.reconcile";

/// Mục chờ đang được một lệnh khớp / nhận số thực thu giữ trong lúc ghi thu.
const RESOLVING: &str = "resolving";

pub(crate) fn exception_not_found() -> Error {
    Error::not_found("ledger.exception_not_found", "There is no such item to resolve.")
}

/// Cách xử lý một mục chờ.
#[derive(Debug, Clone)]
pub(crate) enum Resolution {
    /// Ghi như đã thu cho thanh toán `payment_id`: chỉ khi số tiền thực thu đúng bằng số phải thu.
    Match(Uuid),
    /// Xác nhận riêng: ghi thanh toán `payment_id` đã thu đúng số tiền thực thu, kể cả khi lệch số
    /// phải thu (thiếu hay thừa) — event mang số thực thu.
    AcceptReceived(Uuid),
    Ignore,
    /// Đã hoàn ngoài hệ thống.
    RefundedOutside,
}

/// Loại mục chờ ghi được thành "đã thu" cho một thanh toán.
const MATCHABLE: [&str; 4] =
    ["amount_mismatch", "unmatched_credit", "unknown_in_statement", "missing_in_statement"];

fn amount_differs(received: Money, expected: Money) -> Error {
    Error::conflict(
        "ledger.amount_differs",
        "The partner collected {received} but the payment is for {expected}; resolve with \
         `accept_received` to record the amount collected.",
    )
    .with_param("received", received.minor().to_string())
    .with_param("expected", expected.minor().to_string())
}

/// Số tiền đối tác thực thu của mục chờ `item`, khi ghi nó cho thanh toán `payment` được: loại mục
/// ghi được, mục nói về đúng thanh toán đó (nếu có nêu), thanh toán chưa thu, cùng tiền tệ.
fn matchable(
    item: &exception::Model,
    details: &Value,
    payment: &payment::Model,
) -> Result<Money, Error> {
    if !MATCHABLE.contains(&item.kind.as_str()) {
        return Err(Error::validation(vec![FieldError::new(
            "payment_id",
            "mismatch",
            "this kind of item cannot be matched to a payment",
        )]));
    }
    if let Some(named) = details["payment_id"].as_str()
        && named != payment.id.to_string()
    {
        return Err(Error::validation(vec![FieldError::new(
            "payment_id",
            "mismatch",
            "must be the payment named in the item",
        )]));
    }
    // Cùng luật với `capture`: tiền về cho thanh toán đã hết hạn hay đối tác đã báo hỏng vẫn ghi được (thành
    // `late_payment`), không trả 409.
    if !matches!(payment.status.as_str(), "pending" | "expired" | "failed" | "needs_review") {
        return Err(crate::payments::payment_state(&payment.status));
    }
    // Số thực thu: `received_minor` (+ tiền tệ của `received` / `amount`), hay `amount` / `received`
    // dạng tiền; mục "thiếu trong sao kê" không có số → số phải thu.
    let stated = ["received", "amount"]
        .iter()
        .find_map(|field| serde_json::from_value::<Money>(details[field].clone()).ok());
    let received = match (details["received_minor"].as_i64(), stated) {
        (Some(minor), Some(money)) => Money::new(minor, money.currency()),
        (Some(minor), None) => Money::from_columns(minor, &payment.amount_currency)?,
        (None, Some(money)) => money,
        (None, None) => Money::from_columns(payment.amount_minor, &payment.amount_currency)?,
    };
    if received.currency().code() != payment.amount_currency {
        return Err(Error::validation(vec![FieldError::new(
            "payment_id",
            "mismatch",
            "must be a payment in the currency of the money collected",
        )]));
    }
    Ok(received)
}

/// Loại mục: tiền về sau hạn / sau khi đối tác báo hỏng.
pub(crate) const LATE_PAYMENT: &str = "late_payment";
/// Loại mục: đối tác (hay sổ) từ chối hoàn — tiền còn phải trả lại tay.
pub(crate) const REFUND_FAILED: &str = "refund_failed";

/// Ghi chú của mục `late_payment` do `ledger` đóng khi module nguồn giữ khoản tiền.
const CLOSED_BY_SOURCE: &str =
    "Closed by the ledger: the source module counted this payment (e.g. the order went on).";

/// Ghi chú của mục `late_payment` / `refund_failed` do `ledger` tự đóng.
const CLOSED_BY_REFUND: &str =
    "Closed by the ledger: the payment was refunded in full through the partner.";

/// Khoá chống trùng của mục `late_payment` của thanh toán `payment`.
pub(crate) fn late_payment_key(payment: Uuid) -> String {
    format!("late_payment:{payment}")
}

/// Khoá chống trùng của mục `refund_failed` của lượt hoàn `reference` (mã của bên gọi, duy nhất theo thanh
/// toán) — có cả khi lượt hoàn bị từ chối trước khi có dòng hoàn.
pub(crate) fn refund_failed_key(payment: Uuid, reference: &str) -> String {
    format!("refund_failed:{payment}:{reference}")
}

impl LedgerService {
    /// `dyn Ledger::keep_late_payment`: module nguồn đã tính khoản `payment` (vd đơn đi tiếp) — đóng mục
    /// `late_payment` của nó (`kept`) để không ai hoàn tay tiền đơn đang dùng. Gọi lại vô hại.
    pub(crate) async fn keep_late_payment(
        &self,
        tenant: &TenantContext,
        payment: Uuid,
    ) -> Result<(), Error> {
        // Handle `tenant.db()` của kernel (luật raw-database); lỗi tạm thì bên gọi (event) thử lại.
        let db = tenant.db();
        // Gọi cho mọi khoản thu xong: tra theo khoá chống trùng (index unique), không có mục mở thì thôi (rà
        // #591 N-b). Tra trong transaction ghi, không ở kết nối đọc: mục vừa mở ngay trước event, bản sao đọc
        // có thể chưa thấy — bỏ sót thì mục bảo hoàn tay tiền của đơn đang đi tiếp.
        let transaction = db.begin_write().await?;
        let Some(item) = exception::Entity::find()
            .filter(exception::Column::DedupeKey.eq(late_payment_key(payment)))
            .filter(exception::Column::Status.eq("open"))
            .scoped(tenant)
            .one(&transaction)
            .await?
        else {
            transaction.rollback().await?;
            return Ok(());
        };
        // Đã có lượt hoàn đang chờ hay hỏng: nguồn vừa đổi ý (vd đơn bị huỷ ngay sau khi tính tiền) — mục còn
        // phải mở để người vận hành thấy, không đóng `kept` (rà #591 N-a).
        let refunding = refund::Entity::find()
            .filter(refund::Column::PaymentId.eq(payment))
            .filter(refund::Column::Status.is_in(["pending", "failed"]))
            .scoped(tenant)
            .one(&transaction)
            .await?
            .is_some();
        if refunding {
            transaction.rollback().await?;
            return Ok(());
        }
        close_items(&transaction, tenant, &self.outbox, vec![item], "kept", CLOSED_BY_SOURCE)
            .await?;
        transaction.commit().await?;
        Ok(())
    }
}

/// Trong `transaction` của lượt hoàn vừa xong: thanh toán `paid` đã hoàn đủ số thực thu (các lượt hoàn
/// `succeeded`) thì mục `late_payment` và các mục `refund_failed` (lượt hoàn trước bị từ chối) đang mở của
/// nó tự đóng (`resolved`, `refunded`, không người xử lý), phát `ledger.exception_resolved` — người vận
/// hành chỉ còn thấy khoản thật sự phải hoàn tay (UT-6).
pub(crate) async fn close_late_payment_if_refunded(
    transaction: &impl ConnectionTrait,
    tenant: &TenantContext,
    outbox: &autoxide_kernel::event::Outbox,
    paid: &payment::Model,
) -> Result<(), Error> {
    // Khoá dòng thanh toán trước khi cộng: hai lượt hoàn một phần xong cùng lúc chạy nối đuôi, lượt sau
    // thấy lượt trước đã `succeeded` (Postgres READ COMMITTED không tự bảo đảm điều đó).
    rows::lock_one(transaction, payment::Entity::find_by_id(paid.id).scoped(tenant)).await?;
    let refunded: i64 = refund::Entity::find()
        .filter(refund::Column::PaymentId.eq(paid.id))
        .filter(refund::Column::Status.eq("succeeded"))
        .scoped(tenant)
        .all(transaction)
        .await?
        .iter()
        .map(|row| row.amount_minor)
        .sum();
    if refunded < paid.captured_minor.unwrap_or(paid.amount_minor) {
        return Ok(());
    }
    close_payment_items(
        transaction,
        tenant,
        outbox,
        paid.id,
        &[LATE_PAYMENT, REFUND_FAILED],
        "refunded",
        CLOSED_BY_REFUND,
    )
    .await
}

/// Trong `transaction`: đóng (`resolved`, `resolution`, không người xử lý) các mục loại `kinds` đang mở của
/// thanh toán `payment` (subject `ledger.payment:<id>`), mỗi mục phát `ledger.exception_resolved` một lần.
pub(crate) async fn close_payment_items(
    transaction: &impl ConnectionTrait,
    tenant: &TenantContext,
    outbox: &autoxide_kernel::event::Outbox,
    payment: Uuid,
    kinds: &[&str],
    resolution: &str,
    note: &str,
) -> Result<(), Error> {
    let open = exception::Entity::find()
        .filter(exception::Column::Status.eq("open"))
        .filter(exception::Column::Subject.eq(format!("ledger.payment:{payment}")))
        .filter(exception::Column::Kind.is_in(kinds.iter().copied()))
        .scoped(tenant)
        .all(transaction)
        .await?;
    close_items(transaction, tenant, outbox, open, resolution, note).await
}

/// Trong `transaction`: đóng các mục `items` còn mở (`resolved`, `resolution`, không người xử lý), mỗi mục phát
/// `ledger.exception_resolved` một lần.
async fn close_items(
    transaction: &impl ConnectionTrait,
    tenant: &TenantContext,
    outbox: &autoxide_kernel::event::Outbox,
    items: Vec<exception::Model>,
    resolution: &str,
    note: &str,
) -> Result<(), Error> {
    for row in items {
        let closed = exception::Entity::update_many()
            .col_expr(exception::Column::Status, Expr::value("resolved"))
            .col_expr(exception::Column::Resolution, Expr::value(resolution))
            .col_expr(exception::Column::Note, Expr::value(note))
            .col_expr(exception::Column::UpdatedAt, Expr::value(now()))
            .filter(exception::Column::Id.eq(row.id))
            .filter(exception::Column::Status.eq("open"))
            .scoped(tenant)
            .exec(transaction)
            .await?;
        if closed.rows_affected == 1 {
            let event = events::ExceptionResolved(json!({
                "exception_id": row.id, "kind": row.kind, "subject": row.subject,
                "resolution": resolution,
            }));
            outbox
                .record(
                    transaction,
                    tenant,
                    Actor::system(),
                    Some(format!("ledger.exception:{}", row.id)),
                    &event,
                )
                .await?;
        }
    }
    Ok(())
}

/// Trong `transaction`: mở một mục chờ (một lần cho mỗi `dedupe_key`), phát `ledger.exception_opened`.
pub(crate) async fn insert_exception(
    transaction: &impl ConnectionTrait,
    tenant: &TenantContext,
    outbox: &autoxide_kernel::event::Outbox,
    kind: &str,
    subject: &str,
    details: &Value,
    dedupe_key: &str,
) -> Result<(), Error> {
    let id = Uuid::now_v7();
    let at = now();
    let inserted = exception::Entity::insert(exception::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant.id().as_uuid()),
        kind: Set(kind.to_owned()),
        subject: Set(subject.to_owned()),
        details: Set(details.to_string()),
        dedupe_key: Set(dedupe_key.to_owned()),
        status: Set("open".to_owned()),
        resolution: Set(None),
        note: Set(None),
        resolved_by: Set(None),
        created_at: Set(at),
        updated_at: Set(at),
    })
    .on_conflict(
        OnConflict::columns([exception::Column::TenantId, exception::Column::DedupeKey])
            .do_nothing()
            .to_owned(),
    )
    .exec_without_returning(transaction)
    .await?;
    if inserted == 1 {
        let event = events::ExceptionOpened(json!({
            "exception_id": id, "kind": kind, "subject": subject,
        }));
        outbox
            .record(
                transaction,
                tenant,
                Actor::system(),
                Some(format!("ledger.exception:{id}")),
                &event,
            )
            .await?;
    }
    Ok(())
}

impl LedgerService {
    /// Mở một mục chờ (một lần cho mỗi `dedupe_key`), phát `ledger.exception_opened`.
    pub(crate) async fn open_exception(
        &self,
        tenant: &TenantContext,
        kind: &str,
        subject: &str,
        details: Value,
        dedupe_key: &str,
    ) -> Result<(), Error> {
        let database = tenant.database();
        let outbox = &self.outbox;
        let details = &details;
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                insert_exception(&transaction, tenant, outbox, kind, subject, details, dedupe_key)
                    .await?;
                transaction.commit().await?;
                Ok(())
            })
            .await
    }

    pub(crate) async fn list_exceptions(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        status: Option<String>,
        page: &PageRequest,
    ) -> Result<Page<exception::Model>, Error> {
        // Mục chờ thuộc cả khách: cần `ledger.read` phủ nút gốc (phạm vi cả khách).
        principal.require(&READ)?;
        let root = autoxide_kernel::tenancy::root_of(tenant.id());
        principal
            .require_at(
                tenant.database().read(),
                &READ,
                autoxide_kernel::access::OrgTarget::new(root, None),
            )
            .await?;
        let mut select = exception::Entity::find().scoped(tenant);
        if let Some(status) = status {
            select = select.filter(exception::Column::Status.eq(status));
        }
        fetch_page(select, exception::Column::Id, |row| row.id, page, tenant.database().read())
            .await
    }

    /// Xử lý mục chờ `id`. Đã xử lý → trả lại như cũ (gọi lại an toàn).
    pub(crate) async fn resolve_exception(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        resolution: Resolution,
        note: Option<String>,
    ) -> Result<exception::Model, Error> {
        principal.require(&RECONCILE)?;
        if note.as_ref().is_some_and(|note| note.chars().count() > 500) {
            return Err(Error::validation(vec![FieldError::new(
                "note",
                "length",
                "must be at most 500 characters long",
            )]));
        }
        let database = tenant.database();
        let current = exception::Entity::find_by_id(id)
            .scoped(tenant)
            .one(database.read())
            .await?
            .ok_or_else(exception_not_found)?;
        if current.status != "open" {
            return Ok(current);
        }
        let claimed = matches!(resolution, Resolution::Match(_) | Resolution::AcceptReceived(_));
        let (label, status) = match &resolution {
            Resolution::Match(payment_id) | Resolution::AcceptReceived(payment_id) => {
                let accept = matches!(resolution, Resolution::AcceptReceived(_));
                let found = Self::find_payment(database.read(), tenant, *payment_id)
                    .await?
                    .ok_or_else(crate::payments::payment_not_found)?;
                let details: Value = serde_json::from_str(&current.details).unwrap_or_default();
                let received = matchable(&current, &details, &found)?;
                let expected = Money::from_columns(found.amount_minor, &found.amount_currency)?;
                if received != expected && !accept {
                    // Lệch số: thanh toán giữ `needs_review` tới khi có xác nhận riêng.
                    return Err(amount_differs(received, expected));
                }
                let via = if accept { crate::payments::VIA_ACCEPTED } else { "manual" };
                // Nhận mục chờ TRƯỚC khi ghi thu (rà soát bảo mật 2026-11): hai lệnh khớp cùng lúc
                // vào hai thanh toán khác nhau từng cùng thấy `open` và cùng ghi thu — một khoản
                // tiền trả cho hai đơn. Cập nhật có điều kiện: chỉ một bên đổi được `open` →
                // `resolving`; bên kia trả mục như hiện có.
                let claimed = exception::Entity::update_many()
                    .col_expr(exception::Column::Status, Expr::value(RESOLVING))
                    .col_expr(exception::Column::UpdatedAt, Expr::value(now()))
                    .filter(exception::Column::Id.eq(id))
                    .filter(exception::Column::Status.eq("open"))
                    .scoped(tenant)
                    .exec(database.write())
                    .await?
                    .rows_affected
                    == 1;
                if !claimed {
                    return exception::Entity::find_by_id(id)
                        .scoped(tenant)
                        .one(database.write())
                        .await?
                        .ok_or_else(exception_not_found);
                }
                if let Err(error) = self.capture(tenant, found.id, received, via).await {
                    // Ghi thu hỏng: trả mục về `open` để xử lý lại.
                    exception::Entity::update_many()
                        .col_expr(exception::Column::Status, Expr::value("open"))
                        .filter(exception::Column::Id.eq(id))
                        .filter(exception::Column::Status.eq(RESOLVING))
                        .scoped(tenant)
                        .exec(database.write())
                        .await?;
                    return Err(error);
                }
                (if accept { "accept_received" } else { "match" }, "resolved")
            }
            Resolution::Ignore => ("ignore", "ignored"),
            Resolution::RefundedOutside => ("refunded_outside", "resolved"),
        };
        let outbox = &self.outbox;
        let note = &note;
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let row =
                    rows::lock_one(&transaction, exception::Entity::find_by_id(id).scoped(tenant))
                        .await?
                        .ok_or_else(exception_not_found)?;
                // `resolving`: chính lệnh này đã nhận mục ở trên (chỉ khớp / nhận số thực thu).
                if row.status != "open" && !(claimed && row.status == RESOLVING) {
                    return Ok(row);
                }
                let mut active: exception::ActiveModel = row.into();
                active.status = Set(status.to_owned());
                active.resolution = Set(Some(label.to_owned()));
                active.note = Set(note.clone());
                active.resolved_by = Set(Some(principal.user_id()));
                active.updated_at = Set(now());
                let saved = exception::Entity::update(active).exec(&transaction).await?;
                let event = events::ExceptionResolved(json!({
                    "exception_id": saved.id, "kind": saved.kind, "subject": saved.subject,
                    "resolution": label,
                }));
                outbox
                    .record(
                        &transaction,
                        tenant,
                        Actor::from(principal),
                        Some(format!("ledger.exception:{}", saved.id)),
                        &event,
                    )
                    .await?;
                transaction.commit().await?;
                Ok(saved)
            })
            .await
    }

    pub(crate) async fn list_reconciliations(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<reconciliation::Model>, Error> {
        principal.require(&RECONCILE)?;
        let select = reconciliation::Entity::find().scoped(tenant);
        fetch_page(select, reconciliation::Column::Id, |row| row.id, page, tenant.database().read())
            .await
    }

    /// Đối soát qua REST / MCP.
    pub(crate) async fn reconcile_manual(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        provider: Option<String>,
        on: Date,
    ) -> Result<reconciliation::Model, Error> {
        principal.require(&RECONCILE)?;
        let gateway = self.gateway_for(tenant, provider.as_deref()).await?;
        self.reconcile(tenant, gateway.provider(), on, Some(principal.user_id())).await
    }

    /// Đối soát ngày `on` với sao kê của đối tác `provider`.
    pub(crate) async fn reconcile(
        &self,
        tenant: &TenantContext,
        provider: &str,
        on: Date,
        by: Option<Uuid>,
    ) -> Result<reconciliation::Model, Error> {
        let gateway = self.gateway(Some(provider))?;
        if !gateway.capabilities().statements {
            return Err(autoxide_kernel::payment::not_supported(provider));
        }
        let database = tenant.database();
        let run = Uuid::now_v7();
        // Ngoài transaction: kéo đủ các trang sao kê.
        let mut lines = Vec::new();
        let mut cursor = None;
        loop {
            let page = gateway.statement(tenant.id(), on, on, cursor).await?;
            lines.extend(page.lines);
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        let (mut matched, mut problems) = (0i64, 0i64);
        let mut seen_refs = HashSet::new();
        for line in &lines {
            statement_line::Entity::insert(statement_line::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                provider: Set(provider.to_owned()),
                provider_line_id: Set(line.line_id.clone()),
                provider_ref: Set(line.provider_ref.clone()),
                kind: Set(line.kind.as_str().to_owned()),
                amount_minor: Set(line.amount.minor()),
                amount_currency: Set(line.amount.currency().code().to_owned()),
                fee_minor: Set(line.fee.minor()),
                booked_on: Set(line.booked_on),
                match_status: Set("unmatched".to_owned()),
                payment_id: Set(None),
                reconciliation_id: Set(run),
            })
            .on_conflict(
                OnConflict::columns([
                    statement_line::Column::TenantId,
                    statement_line::Column::Provider,
                    statement_line::Column::ProviderLineId,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(database.write())
            .await?;
            if line.kind == StatementKind::Payment {
                seen_refs.insert(line.provider_ref.clone());
            }
            let (status, payment_id) = self.match_line(tenant, provider, line).await?;
            if status == "matched" {
                matched += 1;
            } else {
                problems += 1;
            }
            statement_line::Entity::update_many()
                .col_expr(statement_line::Column::MatchStatus, status.into())
                .col_expr(statement_line::Column::PaymentId, payment_id.into())
                .filter(statement_line::Column::TenantId.eq(tenant.id().as_uuid()))
                .filter(statement_line::Column::Provider.eq(provider))
                .filter(statement_line::Column::ProviderLineId.eq(line.line_id.as_str()))
                .exec(database.write())
                .await?;
        }
        // Đã thu trong ngày theo sổ mà sao kê không có.
        let start = Some(on.start_of_day_utc());
        let end = on.succ().map(Date::start_of_day_utc);
        if let (Some(start), Some(end)) = (start, end) {
            let captured = payment::Entity::find()
                .scoped(tenant)
                .filter(payment::Column::Provider.eq(provider))
                .filter(payment::Column::Status.eq("succeeded"))
                .filter(payment::Column::CapturedAt.gte(start))
                .filter(payment::Column::CapturedAt.lt(end))
                .all(database.read())
                .await?;
            for row in captured {
                let known =
                    row.provider_ref.as_ref().is_some_and(|found| seen_refs.contains(found));
                if !known {
                    problems += 1;
                    self.open_exception(
                        tenant,
                        "missing_in_statement",
                        &format!("ledger.payment:{}", row.id),
                        json!({"payment_id": row.id, "provider": provider, "date": on}),
                        &format!("missing_in_statement:{}", row.id),
                    )
                    .await?;
                }
            }
        }
        let saved = reconciliation::Entity::insert(reconciliation::ActiveModel {
            id: Set(run),
            tenant_id: Set(tenant.id().as_uuid()),
            provider: Set(provider.to_owned()),
            on_date: Set(on),
            status: Set("completed".to_owned()),
            lines: Set(i64::try_from(lines.len()).unwrap_or(i64::MAX)),
            matched: Set(matched),
            exceptions: Set(problems),
            error: Set(None),
            created_by: Set(by),
            created_at: Set(now()),
        })
        .exec_with_returning(database.write())
        .await?;
        Ok(saved)
    }

    /// Khớp một dòng sao kê: (`matched` / `exception`, thanh toán liên quan).
    async fn match_line(
        &self,
        tenant: &TenantContext,
        provider: &str,
        line: &autoxide_kernel::payment::StatementLine,
    ) -> Result<(&'static str, Option<Uuid>), Error> {
        let database = tenant.database();
        let subject_line = format!("ledger.statement_line:{provider}:{}", line.line_id);
        match line.kind {
            StatementKind::Payment => {
                let found = payment::Entity::find()
                    .scoped(tenant)
                    .filter(payment::Column::Provider.eq(provider))
                    .filter(payment::Column::ProviderRef.eq(line.provider_ref.as_str()))
                    .one(database.write())
                    .await?;
                let Some(found) = found else {
                    self.open_exception(
                        tenant,
                        "unknown_in_statement",
                        &subject_line,
                        json!({"provider": provider, "provider_ref": line.provider_ref, "amount": line.amount}),
                        &format!("unknown_in_statement:{provider}:{}", line.line_id),
                    )
                    .await?;
                    return Ok(("exception", None));
                };
                self.charge_fee(tenant, provider, &found, line).await?;
                match found.status.as_str() {
                    "succeeded" if found.amount_minor == line.amount.minor() => {
                        Ok(("matched", Some(found.id)))
                    }
                    "pending" | "expired" | "failed" | "needs_review" | "succeeded" => {
                        // Webhook bị lạc: sao kê là bằng chứng đã thu.
                        self.capture(tenant, found.id, line.amount, "reconcile").await?;
                        let after = Self::find_payment(database.write(), tenant, found.id).await?;
                        let ok = after.is_some_and(|row| {
                            row.status == "succeeded" && row.amount_minor == line.amount.minor()
                        });
                        if !ok && found.status == "succeeded" {
                            self.open_exception(
                                tenant,
                                "amount_mismatch",
                                &format!("ledger.payment:{}", found.id),
                                json!({"payment_id": found.id, "received": line.amount, "received_minor": line.amount.minor()}),
                                &format!("amount_mismatch:{}", found.id),
                            )
                            .await?;
                        }
                        Ok((if ok { "matched" } else { "exception" }, Some(found.id)))
                    }
                    _ => Ok(("exception", Some(found.id))),
                }
            }
            StatementKind::Refund => {
                let found = refund::Entity::find()
                    .scoped(tenant)
                    .filter(refund::Column::ProviderRef.eq(line.provider_ref.as_str()))
                    .one(database.read())
                    .await?;
                Ok(match found {
                    Some(found) => ("matched", Some(found.payment_id)),
                    None => ("exception", None),
                })
            }
            StatementKind::Fee | StatementKind::Payout => Ok(("matched", None)),
        }
    }

    /// Phí đối tác trên dòng sao kê → bút toán phí (theo mã dòng, chạy lại không ghi trùng).
    async fn charge_fee(
        &self,
        tenant: &TenantContext,
        provider: &str,
        paid: &payment::Model,
        line: &autoxide_kernel::payment::StatementLine,
    ) -> Result<(), Error> {
        if !line.fee.is_positive() {
            return Ok(());
        }
        let book = OrgId::from_uuid(paid.org_id);
        let clearing = AccountKey::new(book, "clearing.gateway", line.fee.currency())
            .with_counterparty(Counterparty::new("partner", provider.to_owned()));
        let fee = AccountKey::new(book, "expense.gateway_fee", line.fee.currency());
        let entry = NewEntry::new(
            book,
            "ledger",
            "gateway_fee",
            format!("fee:{provider}:{}", line.line_id),
            "Payment partner fee",
            vec![
                NewLine::debit(LineAccount::Key(fee), line.fee),
                NewLine::credit(LineAccount::Key(clearing), line.fee),
            ],
        )
        .source_id(paid.id.to_string())
        .on(line.booked_on);
        self.post(tenant, Actor::system(), entry).await.map(|_| ())
    }

    /// Lịch `ledger.reconcile`: đối soát hôm qua cho mọi khách có thanh toán trên `database`, với
    /// mọi cổng có sao kê. Lỗi của một khách không chặn khách khác.
    pub(crate) async fn reconcile_database(
        &self,
        database: &autoxide_kernel::database::Database,
    ) -> Result<u64, Error> {
        let Some(yesterday) = Timestamp::now().date_utc().pred() else {
            return Ok(0);
        };
        let pairs: Vec<(Uuid, String)> = payment::Entity::find()
            .select_only()
            .column(payment::Column::TenantId)
            .column(payment::Column::Provider)
            .distinct()
            .into_tuple()
            .all(database.read())
            .await?;
        let mut done = 0;
        for (tenant, provider) in pairs {
            let Ok(gateway) = self.gateway(Some(&provider)) else { continue };
            if !gateway.capabilities().statements {
                continue;
            }
            let tenant = self.databases.tenant(TenantId::from_uuid(tenant)).await?;
            let already = reconciliation::Entity::find()
                .scoped(&tenant)
                .filter(reconciliation::Column::Provider.eq(provider.as_str()))
                .filter(reconciliation::Column::OnDate.eq(yesterday))
                .filter(reconciliation::Column::Status.eq("completed"))
                .one(database.read())
                .await?;
            if already.is_some() {
                continue;
            }
            match self.reconcile(&tenant, &provider, yesterday, None).await {
                Ok(_) => done += 1,
                Err(error) => {
                    tracing::warn!(tenant_id = %tenant.id(), provider, error = %error, "reconciliation failed");
                }
            }
        }
        Ok(done)
    }
}

/// Lịch `ledger.reconcile` (mỗi ngày, mọi database).
pub(crate) struct ReconcileJob(pub Arc<LedgerService>);

#[async_trait::async_trait]
impl JobHandler for ReconcileJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.reconcile_database(&job.database).await.map(|_| ())
    }
}
