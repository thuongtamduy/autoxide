//! Vòng đời đơn (mục 4 của bước 45): nhân viên chuyển trạng thái, người mua tự huỷ theo chính sách
//! (`orders.buyer_cancel_until`), yêu cầu vận đơn (event `orders.fulfillment_requested` cho
//! `shipping`), hoàn tất khi hết hạn đổi trả, huỷ đơn cửa hàng không nhận kịp
//! (`orders.acceptance_hours`), hoãn hoàn tất (`OrderHolds`), hoàn tiền khi huỷ đơn đã thu. Mọi lần
//! đổi `status` đi qua `lifecycle::apply`.

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, TenantId};
use chrono::{DateTime, Duration, Utc};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set};
use uuid::Uuid;

use crate::api::OrderStatus;
use crate::entity::{completion_hold, order, payment};
use crate::errors;
use crate::jobs::BATCH;
use crate::lifecycle::{Applied, Cause, Transition};
use crate::payments::{self, Method};
use crate::permissions::MANAGE;
use crate::service::OrdersService;

/// Trạng thái nhân viên được chuyển tay (xác nhận do thanh toán, hoàn tất do job).
const STAFF_TARGETS: [OrderStatus; 5] = [
    OrderStatus::Preparing,
    OrderStatus::Shipping,
    OrderStatus::Delivered,
    OrderStatus::DeliveryFailed,
    OrderStatus::Cancelled,
];

impl OrdersService {
    /// Nhân viên chuyển đơn `id` sang `to` (`orders.manage` ở nút của đơn). `expected`: `updated_at`
    /// người gọi đã đọc (`If-Match`); khác → 412.
    pub(crate) async fn staff_transition(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        to: OrderStatus,
        reason: Option<String>,
        expected: Option<DateTime<Utc>>,
    ) -> Result<order::Model, Error> {
        principal.require(&MANAGE)?;
        let row = self.order(tenant, id).await?;
        let target = OrgTarget::new(OrgId::from_uuid(row.org_id), None);
        principal.require_at(tenant.database().read(), &MANAGE, target).await?;
        if !STAFF_TARGETS.contains(&to) {
            let from = OrderStatus::parse(&row.status).unwrap_or(OrderStatus::Cancelled);
            return Err(errors::invalid_transition(from, to));
        }
        let mut transition = Transition::new(to, Cause::Staff, Actor::user(principal.user_id()));
        if let Some(reason) = reason {
            transition = transition.reason(reason);
        }
        let check = move |current: &order::Model| -> Result<bool, Error> {
            match expected {
                Some(version) if version != current.updated_at => {
                    Err(autoxide_kernel::version::changed())
                }
                _ => Ok(true),
            }
        };
        let applied = self.transition_if(tenant, id, &transition, &check).await?;
        if matches!(applied, Applied::Changed) && to == OrderStatus::Cancelled {
            self.refund_cancelled(tenant, id).await?;
        }
        self.order(tenant, id).await
    }

    /// Người mua `buyer` huỷ đơn của mình theo chính sách `orders.buyer_cancel_until` của nút.
    pub(crate) async fn buyer_cancel(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
        id: Uuid,
        reason: Option<String>,
    ) -> Result<order::Model, Error> {
        let row = self.order(tenant, id).await?;
        if row.buyer_user_id != Some(buyer) {
            return Err(errors::order_not_found());
        }
        self.cancel_as_buyer(tenant, row, Actor::user(buyer), reason).await
    }

    /// Huỷ phía người mua (có tài khoản hay vãng lai): cùng chính sách, cùng service.
    pub(crate) async fn cancel_as_buyer(
        &self,
        tenant: &TenantContext,
        row: order::Model,
        actor: Actor,
        reason: Option<String>,
    ) -> Result<order::Model, Error> {
        let policy = self.policy(tenant, OrgId::from_uuid(row.org_id)).await?;
        // Hàng đặt trước dời muộn hơn ngày đã hứa (D-204): huỷ miễn phí, không theo chính sách —
        // cả đơn khi chưa giao gì, không thì phần đặt trước chưa có hàng.
        let delayed: Vec<Uuid> = crate::entity::line::Entity::find()
            .filter(crate::entity::line::Column::OrderId.eq(row.id))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?
            .into_iter()
            .filter(crate::preorders::delayed)
            .map(|line| line.id)
            .collect();
        let status = OrderStatus::parse(&row.status).unwrap_or(OrderStatus::Cancelled);
        if !delayed.is_empty()
            && matches!(status, OrderStatus::Shipping | OrderStatus::DeliveryFailed)
        {
            self.cancel_preorder_lines(
                tenant,
                row.id,
                &delayed,
                "buyer_cancelled",
                crate::preorders::LineCancel::default(),
            )
            .await?;
            return self.order(tenant, row.id).await;
        }
        let free = !delayed.is_empty();
        let transition = Transition::new(OrderStatus::Cancelled, Cause::Buyer, actor)
            .reason(reason.unwrap_or_else(|| "buyer_cancelled".to_owned()));
        let check = move |current: &order::Model| -> Result<bool, Error> {
            let status = OrderStatus::parse(&current.status).unwrap_or(OrderStatus::Cancelled);
            if status == OrderStatus::Cancelled {
                return Ok(false);
            }
            let early = matches!(
                status,
                OrderStatus::PendingPayment | OrderStatus::Confirmed | OrderStatus::Preparing
            );
            if policy.buyer_may_cancel(status) || (free && early) {
                Ok(true)
            } else {
                Err(errors::cancel_not_allowed())
            }
        };
        let applied = self.transition_if(tenant, row.id, &transition, &check).await?;
        if matches!(applied, Applied::Changed) {
            // Trả trước một phần (D-232): huỷ sau mốc chốt thì giữ lại theo điều kiện đã chụp —
            // trừ khi được huỷ miễn phí vì người bán dời ngày.
            let keep = if free { 0 } else { self.retention_now(tenant, row.id).await? };
            self.refund_cancelled_keeping(tenant, row.id, keep).await?;
            self.post_retention(tenant, row.id, keep).await?;
        }
        self.order(tenant, row.id).await
    }

    /// Một lô đơn `delivered` quá hạn đổi trả của nút, không có hoãn → `completed`.
    pub(crate) async fn complete_delivered(&self, database: &Database) -> Result<u64, Error> {
        let due: Vec<(Uuid, Uuid, Uuid, Option<DateTime<Utc>>)> = order::Entity::find()
            .select_only()
            .column(order::Column::TenantId)
            .column(order::Column::Id)
            .column(order::Column::OrgId)
            .column(order::Column::DeliveredAt)
            .filter(order::Column::Status.eq(OrderStatus::Delivered.as_str()))
            .order_by_asc(order::Column::DeliveredAt)
            .limit(BATCH)
            .into_tuple()
            .all(database.read())
            .await?;
        let mut completed = 0;
        for (tenant_id, id, org, delivered_at) in due {
            let tenant = self.databases.tenant(TenantId::from_uuid(tenant_id)).await?;
            let policy = self.policy(&tenant, OrgId::from_uuid(org)).await?;
            let ends = delivered_at.map(|at| at + Duration::days(policy.return_window_days));
            if ends.is_none_or(|ends| ends > crate::service::now()) {
                continue;
            }
            let held = completion_hold::Entity::find()
                .filter(completion_hold::Column::OrderId.eq(id))
                .scoped(&tenant)
                .one(tenant.database().read())
                .await?
                .is_some();
            if held {
                continue;
            }
            let transition =
                Transition::new(OrderStatus::Completed, Cause::System, Actor::system());
            match self.transition(&tenant, id, &transition, Some(OrderStatus::Delivered)).await {
                Ok(Applied::Changed) => completed += 1,
                Ok(Applied::Already(_)) => {}
                Err(error) => {
                    tracing::warn!(order_id = %id, code = error.code(), "order not completed")
                }
            }
        }
        Ok(completed)
    }

    /// Một lô đơn `confirmed` quá `orders.acceptance_hours` của nút mà cửa hàng chưa nhận →
    /// huỷ (`seller_not_accepted`), hoàn tiền tự động (D-209).
    pub(crate) async fn expire_unaccepted(&self, database: &Database) -> Result<u64, Error> {
        let due: Vec<(Uuid, Uuid, Uuid, Option<DateTime<Utc>>)> = order::Entity::find()
            .select_only()
            .column(order::Column::TenantId)
            .column(order::Column::Id)
            .column(order::Column::OrgId)
            .column(order::Column::ConfirmedAt)
            .filter(order::Column::Status.eq(OrderStatus::Confirmed.as_str()))
            .filter(order::Column::Channel.eq("web"))
            .order_by_asc(order::Column::ConfirmedAt)
            .limit(BATCH)
            .into_tuple()
            .all(database.read())
            .await?;
        let mut cancelled = 0;
        for (tenant_id, id, org, confirmed_at) in due {
            let tenant = self.databases.tenant(TenantId::from_uuid(tenant_id)).await?;
            let policy = self.policy(&tenant, OrgId::from_uuid(org)).await?;
            if policy.acceptance_hours == 0 {
                continue;
            }
            let ends = confirmed_at.map(|at| at + Duration::hours(policy.acceptance_hours));
            if ends.is_none_or(|ends| ends > crate::service::now()) {
                continue;
            }
            let transition =
                Transition::new(OrderStatus::Cancelled, Cause::System, Actor::system())
                    .reason("seller_not_accepted");
            match self.transition(&tenant, id, &transition, Some(OrderStatus::Confirmed)).await {
                Ok(Applied::Changed) => {
                    cancelled += 1;
                    self.refund_cancelled(&tenant, id).await?;
                }
                Ok(Applied::Already(_)) => {}
                Err(error) => {
                    tracing::warn!(order_id = %id, code = error.code(), "unaccepted order not cancelled")
                }
            }
        }
        Ok(cancelled)
    }

    /// Việc thanh toán còn hạn của đơn chờ thanh toán (gửi lại link); lượt thu chưa tạo được lúc
    /// đặt thì tạo lại.
    pub(crate) async fn payment_link(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Vec<payment::Model>, Error> {
        principal.require(&MANAGE)?;
        let row = self.order(tenant, id).await?;
        let target = OrgTarget::new(OrgId::from_uuid(row.org_id), None);
        principal.require_at(tenant.database().read(), &MANAGE, target).await?;
        // Phần còn lại của đơn trả trước một phần (D-232): mở lại link khi link trước đã hết hạn.
        let plan = self.advance_plan(tenant.database().read(), tenant, id).await?;
        if plan.is_some_and(|plan| plan.status == crate::advance::BALANCE_REQUESTED) {
            return Ok(vec![self.reopen_balance(tenant, id).await?]);
        }
        if row.status != OrderStatus::PendingPayment.as_str() {
            return Err(errors::invalid_transition(
                OrderStatus::parse(&row.status).unwrap_or(OrderStatus::Cancelled),
                OrderStatus::Confirmed,
            ));
        }
        if row.payment_due_at.is_some_and(|due| due <= crate::service::now()) {
            return Err(errors::payment_expired());
        }
        if Method::parse(&row.payment_method) != Method::Cod {
            self.start_payment(tenant, id, None).await?;
        }
        Ok(payment::Entity::find()
            .filter(payment::Column::OrderId.eq(id))
            .filter(payment::Column::Status.eq(payments::PENDING))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?)
    }
}

/// Hoãn hoàn tất đơn (tranh chấp của bước 46, yêu cầu trả hàng).
#[async_trait::async_trait]
impl crate::api::OrderHolds for OrdersService {
    async fn hold_completion(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        reference: &str,
    ) -> Result<(), Error> {
        self.order(tenant, id).await?;
        let row = completion_hold::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            order_id: Set(id),
            reference: Set(reference.to_owned()),
            created_at: Set(crate::service::now()),
        };
        completion_hold::Entity::insert(row)
            .on_conflict(
                OnConflict::columns([
                    completion_hold::Column::TenantId,
                    completion_hold::Column::OrderId,
                    completion_hold::Column::Reference,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(tenant.database().write())
            .await?;
        Ok(())
    }

    async fn release_completion(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        reference: &str,
    ) -> Result<(), Error> {
        completion_hold::Entity::delete_many()
            .filter(completion_hold::Column::OrderId.eq(id))
            .filter(completion_hold::Column::Reference.eq(reference))
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        Ok(())
    }
}
