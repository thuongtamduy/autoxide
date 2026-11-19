//! Đổi trả (mục 6 của bước 45, D-20261119-step45-returns): người mua yêu cầu trả trong hạn chính
//! sách với lý do thuộc `orders.allowed_return_reasons`; nhân viên duyệt / từ chối (lý do thuộc
//! `orders.return_rejected`), nhận hàng về (từng dòng: nhập lại kho hay hỏng → event
//! `orders.return_received` cho `inventory`), hoàn tiền (qua `refunds.rs`), đóng.
//!
//! | Từ | Sang | Ai |
//! |---|---|---|
//! | `requested` | `approved`, `rejected` | nhân viên |
//! | `requested`, `approved` | `received` | nhân viên (nhận tại cửa hàng thì bỏ qua bước duyệt) |
//! | `approved` | `rejected` | nhân viên (vd hàng gửi về không đúng) |
//! | `received` | `refunded` | nhân viên — tạo lượt hoàn theo tiền tính lúc nhận |
//! | `received`, `refunded` | `closed` | nhân viên (đổi hàng: nhận rồi đóng) |
//!
//! Yêu cầu còn mở hoãn hoàn tất đơn (`orders_completion_holds`, mã `return:<id>`).

use std::collections::HashMap;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::numbering;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use autoxide_promotions::api::Evaluation;
use chrono::{Datelike, Duration};
use sea_orm::sea_query::{Expr, ExprTrait};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, DatabaseTransaction, EntityTrait, QueryFilter, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::api::OrderStatus;
use crate::entity::{completion_hold, line, order, return_request};
use crate::errors;
use crate::events::{Change, ReturnReceived, ReturnRequested, ReturnStatusChanged};
use crate::permissions::RETURNS_MANAGE;
use crate::reasons::{RETURN_REASON, RETURN_REJECTED};
use crate::refunds::RefundRequest;
use crate::return_refund::{self, Breakdown, Earlier};
use crate::service::OrdersService;

pub(crate) const REQUESTED: &str = "requested";
pub(crate) const APPROVED: &str = "approved";
pub(crate) const REJECTED: &str = "rejected";
pub(crate) const RECEIVED: &str = "received";
pub(crate) const REFUNDED: &str = "refunded";
pub(crate) const CLOSED: &str = "closed";
/// Trạng thái còn mở: số đơn vị trong đó đã "đặt chỗ" trên dòng đơn.
const OPEN: [&str; 2] = [REQUESTED, APPROVED];
/// Ảnh kèm tối đa (D-202).
pub(crate) const MAX_PHOTOS: usize = 6;
const MAX_LINES: usize = 100;
pub(crate) const METHODS: [&str; 3] = ["pickup", "drop_off", "in_store"];
pub(crate) const RESOLUTIONS: [&str; 2] = ["refund", "exchange"];
pub(crate) const CONDITIONS: [&str; 2] = ["restock", "damaged"];

/// Một dòng của yêu cầu trả (JSON trong cột `lines`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ReturnLine {
    pub line_id: Uuid,
    pub variant_id: Uuid,
    pub quantity: i64,
    /// Dòng quà thêm vào vì chương trình mất điều kiện (`return_gift`).
    #[serde(default)]
    pub gift: bool,
    /// Số đã nhận về và tình trạng (`restock` / `damaged`), khi đã nhận.
    #[serde(default)]
    pub received: Option<i64>,
    #[serde(default)]
    pub condition: Option<String>,
}

/// Yêu cầu trả của người mua.
#[derive(Debug, Clone)]
pub(crate) struct ReturnInput {
    pub lines: Vec<(Uuid, i64)>,
    pub reason_code: String,
    pub note: Option<String>,
    pub resolution: String,
    pub return_method: String,
    pub media_ids: Vec<Uuid>,
}

/// Một dòng nhận về.
#[derive(Debug, Clone)]
pub(crate) struct Received {
    pub line_id: Uuid,
    pub quantity: i64,
    pub condition: String,
}

/// Việc nhân viên làm với yêu cầu trả.
#[derive(Debug, Clone)]
pub(crate) enum ReturnAction {
    Approve,
    Reject { reason: String },
    Receive { lines: Vec<Received>, warehouse_org_id: Option<Uuid> },
    Refund,
    Close,
}

impl ReturnAction {
    fn target(&self) -> &'static str {
        match self {
            ReturnAction::Approve => APPROVED,
            ReturnAction::Reject { .. } => REJECTED,
            ReturnAction::Receive { .. } => RECEIVED,
            ReturnAction::Refund => REFUNDED,
            ReturnAction::Close => CLOSED,
        }
    }

    fn allowed_from(&self) -> &'static [&'static str] {
        match self {
            ReturnAction::Approve => &[REQUESTED],
            ReturnAction::Reject { .. } | ReturnAction::Receive { .. } => &[REQUESTED, APPROVED],
            ReturnAction::Refund => &[RECEIVED],
            ReturnAction::Close => &[RECEIVED, REFUNDED],
        }
    }
}

/// Lọc danh sách yêu cầu trả của nhân viên.
#[derive(Debug, Clone, Default)]
pub(crate) struct ReturnFilter {
    pub status: Option<String>,
    pub org_id: Option<Uuid>,
    pub order_id: Option<Uuid>,
}

pub(crate) fn lines_of(row: &return_request::Model) -> Result<Vec<ReturnLine>, Error> {
    serde_json::from_str(&row.lines).map_err(Error::internal)
}

pub(crate) fn media_of(row: &return_request::Model) -> Vec<Uuid> {
    serde_json::from_str(&row.media_ids).unwrap_or_default()
}

pub(crate) fn evaluation_of(row: &order::Model) -> Option<Evaluation> {
    row.promotions.as_deref().and_then(|raw| serde_json::from_str(raw).ok())
}

fn breakdown_of(row: &return_request::Model) -> Option<Breakdown> {
    row.breakdown.as_deref().and_then(|raw| serde_json::from_str(raw).ok())
}

fn hold_reference(id: Uuid) -> String {
    format!("return:{id}")
}

async fn order_lines(
    transaction: &DatabaseTransaction,
    tenant: &TenantContext,
    order_id: Uuid,
) -> Result<Vec<line::Model>, Error> {
    Ok(line::Entity::find()
        .filter(line::Column::OrderId.eq(order_id))
        .scoped(tenant)
        .all(transaction)
        .await?)
}

async fn order_returns(
    transaction: &DatabaseTransaction,
    tenant: &TenantContext,
    order_id: Uuid,
) -> Result<Vec<return_request::Model>, Error> {
    Ok(return_request::Entity::find()
        .filter(return_request::Column::OrderId.eq(order_id))
        .scoped(tenant)
        .all(transaction)
        .await?)
}

/// Số đơn vị của mỗi dòng đơn đang nằm trong yêu cầu còn mở.
fn open_quantities(returns: &[return_request::Model]) -> Result<HashMap<Uuid, i64>, Error> {
    let mut open = HashMap::new();
    for row in returns.iter().filter(|row| OPEN.contains(&row.status.as_str())) {
        for line in lines_of(row)? {
            *open.entry(line.line_id).or_insert(0) += line.quantity;
        }
    }
    Ok(open)
}

impl OrdersService {
    /// Người mua `buyer` yêu cầu trả hàng của đơn `order_id`. Mọi lỗi của đầu vào báo một lần.
    pub(crate) async fn request_return(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
        order_id: Uuid,
        input: ReturnInput,
    ) -> Result<return_request::Model, Error> {
        let row = self.order(tenant, order_id).await?;
        if row.buyer_user_id != Some(buyer) {
            return Err(errors::order_not_found());
        }
        let status = OrderStatus::parse(&row.status).unwrap_or(OrderStatus::Cancelled);
        let policy = self.policy(tenant, OrgId::from_uuid(row.org_id)).await?;
        if status == OrderStatus::Completed {
            return Err(errors::return_window_closed(policy.return_window_days));
        }
        if status != OrderStatus::Delivered {
            return Err(errors::return_not_allowed(status));
        }
        let delivered_at = row.delivered_at.unwrap_or(row.updated_at);
        let deadline_at = delivered_at + Duration::days(policy.return_window_days);
        let now = crate::service::now();
        if now > deadline_at {
            return Err(errors::return_window_closed(policy.return_window_days));
        }
        let mut problems = Vec::new();
        let reason_allowed = policy.allowed_return_reasons.contains(&input.reason_code)
            && self.reasons.ensure(tenant, RETURN_REASON, &input.reason_code).await.is_ok();
        if !reason_allowed {
            problems.push(
                FieldError::new(
                    "reason_code",
                    "not_allowed",
                    "is not a return reason this shop accepts; allowed: {allowed}",
                )
                .with_param("allowed", policy.allowed_return_reasons.join(", ")),
            );
        }
        if !RESOLUTIONS.contains(&input.resolution.as_str()) {
            problems.push(FieldError::new(
                "resolution",
                "invalid",
                "must be `refund` or `exchange`",
            ));
        }
        if !METHODS.contains(&input.return_method.as_str()) {
            problems.push(FieldError::new(
                "return_method",
                "invalid",
                "must be `pickup`, `drop_off` or `in_store`",
            ));
        }
        if input.media_ids.len() > MAX_PHOTOS {
            problems.push(
                FieldError::new("media_ids", "length", "must have at most {max} photos")
                    .with_param("max", MAX_PHOTOS),
            );
        }
        if input.lines.is_empty() || input.lines.len() > MAX_LINES {
            problems.push(
                FieldError::new("lines", "length", "must have 1 to {max} lines")
                    .with_param("max", MAX_LINES),
            );
        }
        let (database, outbox) = (tenant.database(), &self.outbox);
        let (input, policy_days) = (&input, policy.return_window_days);
        let problems = &problems;
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let current = crate::lifecycle::lock(&transaction, tenant, order_id).await?;
                if current.status != OrderStatus::Delivered.as_str() {
                    let status =
                        OrderStatus::parse(&current.status).unwrap_or(OrderStatus::Cancelled);
                    return Err(errors::return_not_allowed(status));
                }
                let lines = order_lines(&transaction, tenant, order_id).await?;
                let open = open_quantities(&order_returns(&transaction, tenant, order_id).await?)?;
                let mut problems = problems.clone();
                let mut wanted: HashMap<Uuid, i64> = HashMap::new();
                for (index, (line_id, quantity)) in input.lines.iter().enumerate() {
                    let field = format!("lines[{index}].quantity");
                    let Some(line) = lines.iter().find(|line| line.id == *line_id) else {
                        problems.push(FieldError::new(
                            format!("lines[{index}].line_id"),
                            "not_found",
                            "is not a line of this order",
                        ));
                        continue;
                    };
                    if !line.requires_shipping || line.gift_of.is_some() {
                        problems.push(
                            FieldError::new(
                                format!("lines[{index}].line_id"),
                                "not_returnable",
                                "{sku} cannot be returned (digital items, services and gifts \
                                 are not returned by themselves)",
                            )
                            .with_param("sku", line.sku.clone()),
                        );
                        continue;
                    }
                    let left = line.quantity
                        - line.returned_quantity
                        - open.get(&line.id).copied().unwrap_or(0)
                        - wanted.get(&line.id).copied().unwrap_or(0);
                    if *quantity < 1 || *quantity > left {
                        problems.push(
                            FieldError::new(field, "range", "must be from 1 to {left} for {sku}")
                                .with_param("left", Ord::max(left, 0))
                                .with_param("sku", line.sku.clone()),
                        );
                        continue;
                    }
                    *wanted.entry(line.id).or_insert(0) += quantity;
                }
                if !problems.is_empty() {
                    return Err(Error::validation(problems));
                }
                // Tính "mất điều kiện" trên mọi đơn vị sẽ rời đơn (kể cả yêu cầu khác còn mở).
                let mut leaving = open.clone();
                for (line_id, quantity) in &wanted {
                    *leaving.entry(*line_id).or_insert(0) += quantity;
                }
                let evaluation = evaluation_of(&current);
                let mut return_lines: Vec<ReturnLine> = Vec::new();
                for line in &lines {
                    if let Some(quantity) = wanted.get(&line.id) {
                        return_lines.push(ReturnLine {
                            line_id: line.id,
                            variant_id: line.variant_id,
                            quantity: *quantity,
                            gift: false,
                            received: None,
                            condition: None,
                        });
                    }
                }
                for (gift_id, quantity) in
                    return_refund::gifts_to_return(&lines, &leaving, evaluation.as_ref())?
                {
                    if let Some(gift) = lines.iter().find(|line| line.id == gift_id) {
                        return_lines.push(ReturnLine {
                            line_id: gift.id,
                            variant_id: gift.variant_id,
                            quantity,
                            gift: true,
                            received: None,
                            condition: None,
                        });
                    }
                }
                let id = Uuid::now_v7();
                let year = now.year();
                let sequence =
                    numbering::next(&transaction, tenant.id(), &format!("orders.return:{year}"))
                        .await?;
                let saved = return_request::ActiveModel {
                    id: Set(id),
                    tenant_id: Set(tenant.id().as_uuid()),
                    order_id: Set(order_id),
                    org_id: Set(current.org_id),
                    buyer_user_id: Set(current.buyer_user_id),
                    number: Set(format!("R{year}-{sequence:06}")),
                    status: Set(REQUESTED.to_owned()),
                    reason_code: Set(input.reason_code.clone()),
                    buyer_note: Set(input.note.clone()),
                    resolution: Set(input.resolution.clone()),
                    return_method: Set(input.return_method.clone()),
                    media_ids: Set(json!(input.media_ids).to_string()),
                    lines: Set(serde_json::to_string(&return_lines).map_err(Error::internal)?),
                    refund_minor: Set(None),
                    breakdown: Set(None),
                    currency: Set(current.currency.clone()),
                    reject_reason: Set(None),
                    warehouse_org_id: Set(None),
                    requested_at: Set(now),
                    deadline_at: Set(deadline_at),
                    decided_by: Set(None),
                    decided_at: Set(None),
                    received_at: Set(None),
                    created_at: Set(now),
                    updated_at: Set(now),
                }
                .insert(&transaction)
                .await?;
                completion_hold::ActiveModel {
                    tenant_id: Set(tenant.id().as_uuid()),
                    order_id: Set(order_id),
                    reference: Set(hold_reference(id)),
                    created_at: Set(now),
                }
                .insert(&transaction)
                .await?;
                let event = ReturnRequested(Change::after(json!({
                    "return_id": id, "order_id": order_id, "tenant_id": current.tenant_id,
                    "org_id": current.org_id, "channel": current.channel,
                    "number": saved.number, "order_number": current.number,
                    "reason_code": saved.reason_code, "resolution": saved.resolution,
                    "return_method": saved.return_method, "photo_count": input.media_ids.len(),
                    "window_days": policy_days, "lines": return_lines,
                })));
                outbox
                    .record(&transaction, tenant, Actor::user(buyer), subject(order_id), &event)
                    .await?;
                transaction.commit().await?;
                Ok(saved)
            })
            .await
    }

    /// Yêu cầu trả `id` của người mua `buyer`.
    pub(crate) async fn my_return(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
        id: Uuid,
    ) -> Result<return_request::Model, Error> {
        return_request::Entity::find_by_id(id)
            .filter(return_request::Column::BuyerUserId.eq(buyer))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(errors::return_not_found)
    }

    /// Yêu cầu trả `id` cho nhân viên có `orders.returns.manage` ở nút bán của đơn.
    pub(crate) async fn staff_return(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<return_request::Model, Error> {
        principal.require(&RETURNS_MANAGE)?;
        let row = return_request::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(errors::return_not_found)?;
        let target = OrgTarget::new(OrgId::from_uuid(row.org_id), None);
        principal.require_at(tenant.database().read(), &RETURNS_MANAGE, target).await?;
        Ok(row)
    }

    /// Danh sách yêu cầu trả trong phạm vi quyền.
    pub(crate) async fn list_returns(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: &ReturnFilter,
        page: &PageRequest,
    ) -> Result<Page<return_request::Model>, Error> {
        let scope =
            principal.scope_condition(&RETURNS_MANAGE, return_request::Column::OrgId, None)?;
        let mut condition = Condition::all().add(scope);
        if let Some(status) = &filter.status {
            condition = condition.add(return_request::Column::Status.eq(status.as_str()));
        }
        if let Some(org) = filter.org_id {
            condition = condition.add(return_request::Column::OrgId.eq(org));
        }
        if let Some(order) = filter.order_id {
            condition = condition.add(return_request::Column::OrderId.eq(order));
        }
        let select = return_request::Entity::find().filter(condition).scoped(tenant);
        fetch_page(select, return_request::Column::Id, |row| row.id, page, tenant.database().read())
            .await
    }

    /// Nhân viên làm `action` với yêu cầu trả `id`.
    pub(crate) async fn transition_return(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        action: ReturnAction,
    ) -> Result<return_request::Model, Error> {
        let row = self.staff_return(tenant, principal, id).await?;
        if row.status == action.target() {
            // Gửi lại cùng bước (agent retry): kết quả hiện có.
            return Ok(row);
        }
        let mut problems = Vec::new();
        match &action {
            ReturnAction::Reject { reason } => {
                if self.reasons.ensure(tenant, RETURN_REJECTED, reason).await.is_err() {
                    problems.push(FieldError::new(
                        "reason",
                        "not_allowed",
                        "must be an active code of the `orders.return_rejected` reason list",
                    ));
                }
            }
            ReturnAction::Receive { lines, .. } => {
                for (index, line) in lines.iter().enumerate() {
                    if !CONDITIONS.contains(&line.condition.as_str()) {
                        problems.push(FieldError::new(
                            format!("lines[{index}].condition"),
                            "invalid",
                            "must be `restock` or `damaged`",
                        ));
                    }
                }
            }
            _ => {}
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        if matches!(action, ReturnAction::Refund) {
            // Hoàn trước (mã `orders:return:<id>`: gọi lại không hoàn hai lần), rồi mới đổi trạng thái.
            if !action.allowed_from().contains(&row.status.as_str()) {
                return Err(errors::return_invalid_transition(&row.status, REFUNDED));
            }
            let amount = row.refund_minor.unwrap_or(0);
            if row.resolution == "refund" && amount > 0 {
                let request = RefundRequest {
                    order_id: row.order_id,
                    amount: autoxide_kernel::money::Money::from_columns(amount, &row.currency)?,
                    reason: format!("return:{}", row.reason_code),
                    reference: format!("orders:return:{id}"),
                    return_id: Some(id),
                    payment_id: None,
                    created_by: Some(principal.user_id()),
                };
                self.refund(tenant, request).await?;
            }
        }
        let actor = Actor::from(principal);
        let decided_by = Some(principal.user_id());
        let (database, outbox, action) = (tenant.database(), &self.outbox, &action);
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let current = crate::lifecycle::lock(&transaction, tenant, row.order_id).await?;
                let stored = return_request::Entity::find_by_id(id)
                    .scoped(tenant)
                    .one(&transaction)
                    .await?
                    .ok_or_else(errors::return_not_found)?;
                let to = action.target();
                if stored.status == to {
                    return Ok(stored);
                }
                if !action.allowed_from().contains(&stored.status.as_str()) {
                    return Err(errors::return_invalid_transition(&stored.status, to));
                }
                let now = crate::service::now();
                let before = stored.status.clone();
                let mut updated: return_request::ActiveModel = stored.clone().into();
                updated.status = Set(to.to_owned());
                updated.updated_at = Set(now);
                match action {
                    ReturnAction::Approve => {
                        updated.decided_by = Set(decided_by);
                        updated.decided_at = Set(Some(now));
                    }
                    ReturnAction::Reject { reason } => {
                        updated.decided_by = Set(decided_by);
                        updated.decided_at = Set(Some(now));
                        updated.reject_reason = Set(Some(reason.clone()));
                    }
                    ReturnAction::Receive { lines, warehouse_org_id } => {
                        let received = self
                            .receive(&transaction, tenant, &current, &stored, lines, *warehouse_org_id)
                            .await?;
                        updated.lines = Set(serde_json::to_string(&received.lines).map_err(Error::internal)?);
                        updated.refund_minor = Set(Some(received.breakdown.total));
                        updated.breakdown =
                            Set(Some(serde_json::to_string(&received.breakdown).map_err(Error::internal)?));
                        updated.warehouse_org_id = Set(Some(received.warehouse_org_id));
                        updated.received_at = Set(Some(now));
                        if stored.decided_at.is_none() {
                            updated.decided_by = Set(decided_by);
                            updated.decided_at = Set(Some(now));
                        }
                        if received.restocked.iter().any(|line| line["quantity"].as_i64() > Some(0)) {
                            let event = ReturnReceived(Change::after(json!({
                                "return_id": id, "order_id": current.id,
                                "tenant_id": current.tenant_id, "org_id": current.org_id,
                                "channel": current.channel,
                                "warehouse_org_id": received.warehouse_org_id,
                                "lines": received.restocked,
                            })));
                            outbox.record(&transaction, tenant, actor, subject(current.id), &event).await?;
                        }
                    }
                    ReturnAction::Refund | ReturnAction::Close => {}
                }
                let saved = updated.update(&transaction).await?;
                if matches!(to, REJECTED | REFUNDED | CLOSED) {
                    completion_hold::Entity::delete_many()
                        .filter(completion_hold::Column::OrderId.eq(current.id))
                        .filter(completion_hold::Column::Reference.eq(hold_reference(id)))
                        .scoped(tenant)
                        .exec(&transaction)
                        .await?;
                }
                let event = ReturnStatusChanged(Change {
                    before: Some(json!({ "status": before })),
                    after: Some(json!({
                        "return_id": id, "order_id": current.id, "tenant_id": current.tenant_id,
                        "org_id": current.org_id, "channel": current.channel,
                        "number": saved.number, "status": to,
                        "reject_reason": saved.reject_reason,
                        "refund": saved.refund_minor.map(|minor| autoxide_kernel::money::Money::from_columns(minor, &saved.currency)).transpose()?,
                    })),
                });
                outbox.record(&transaction, tenant, actor, subject(current.id), &event).await?;
                transaction.commit().await?;
                Ok(saved)
            })
            .await
    }

    /// Nhận hàng về (trong transaction có khoá dòng đơn): cộng `returned_quantity` của dòng, tính
    /// tiền hoàn, dựng dòng nhập lại kho cho `inventory`.
    async fn receive(
        &self,
        transaction: &DatabaseTransaction,
        tenant: &TenantContext,
        current: &order::Model,
        stored: &return_request::Model,
        received: &[Received],
        warehouse: Option<Uuid>,
    ) -> Result<ReceivedReturn, Error> {
        let mut lines = lines_of(stored)?;
        let mut problems = Vec::new();
        let mut taken: HashMap<Uuid, i64> = HashMap::new();
        for (index, item) in received.iter().enumerate() {
            let Some(line) = lines.iter().find(|line| line.line_id == item.line_id) else {
                problems.push(FieldError::new(
                    format!("lines[{index}].line_id"),
                    "not_found",
                    "is not a line of this return",
                ));
                continue;
            };
            let total = taken.get(&item.line_id).copied().unwrap_or(0) + item.quantity;
            if item.quantity < 0 || total > line.quantity {
                problems.push(
                    FieldError::new(
                        format!("lines[{index}].quantity"),
                        "range",
                        "must be from 0 to {max}",
                    )
                    .with_param("max", line.quantity),
                );
                continue;
            }
            taken.insert(item.line_id, total);
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let order_lines = order_lines(transaction, tenant, current.id).await?;
        let mut earlier = Earlier::default();
        for other in order_returns(transaction, tenant, current.id).await? {
            if other.id != stored.id
                && let Some(breakdown) = breakdown_of(&other)
            {
                earlier.add(&breakdown);
            }
        }
        let evaluation = evaluation_of(current);
        let seller_fault = return_refund::SELLER_FAULT.contains(&stored.reason_code.as_str());
        let breakdown = if stored.resolution == "refund" {
            return_refund::refund_for(
                current,
                &order_lines,
                &taken,
                seller_fault,
                evaluation.as_ref(),
                &earlier,
            )?
        } else {
            Breakdown::default()
        };
        let mut restocked = Vec::new();
        for line in &mut lines {
            let quantity = taken.get(&line.line_id).copied().unwrap_or(0);
            let condition = received
                .iter()
                .find(|item| item.line_id == line.line_id)
                .map_or_else(|| "restock".to_owned(), |item| item.condition.clone());
            line.received = Some(quantity);
            line.condition = Some(condition.clone());
            if quantity == 0 {
                continue;
            }
            // Cộng có chặn trên: không bao giờ trả quá số đã mua, kể cả khi hai lần nhận chạy cùng lúc.
            let changed = line::Entity::update_many()
                .col_expr(
                    line::Column::ReturnedQuantity,
                    Expr::col(line::Column::ReturnedQuantity).add(quantity),
                )
                .filter(line::Column::Id.eq(line.line_id))
                .filter(
                    Expr::col(line::Column::ReturnedQuantity)
                        .add(quantity)
                        .lte(Expr::col(line::Column::Quantity)),
                )
                .scoped(tenant)
                .exec(transaction)
                .await?;
            if changed.rows_affected == 0 {
                return Err(Error::validation(vec![FieldError::new(
                    "lines",
                    "range",
                    "more units than were bought would be returned",
                )]));
            }
            restocked.push(json!({
                "variant_id": line.variant_id, "quantity": quantity,
                "sellable": condition == "restock", "serials": [],
            }));
        }
        let warehouse_org_id = warehouse
            .or_else(|| {
                order_lines
                    .iter()
                    .find(|line| taken.contains_key(&line.id))
                    .and_then(|line| line.warehouse_org_id)
            })
            .unwrap_or(current.org_id);
        Ok(ReceivedReturn { lines, breakdown, restocked, warehouse_org_id })
    }
}

struct ReceivedReturn {
    lines: Vec<ReturnLine>,
    breakdown: Breakdown,
    restocked: Vec<serde_json::Value>,
    warehouse_org_id: Uuid,
}

fn subject(order_id: Uuid) -> Option<String> {
    Some(format!("orders.order:{order_id}"))
}
