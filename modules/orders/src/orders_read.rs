//! Đọc đơn: người mua (`/me/orders`, chỉ đơn của mình), nhân viên (`orders.read` theo phạm vi nút),
//! việc thanh toán của đơn (`payment_actions`).

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder};
use uuid::Uuid;

use crate::entity::{line, order, payment};
use crate::errors;
use crate::permissions::READ;
use crate::service::OrdersService;

/// Đơn kèm dòng và khoản thanh toán.
#[derive(Debug, Clone)]
pub(crate) struct OrderView {
    pub order: order::Model,
    pub lines: Vec<line::Model>,
    pub payments: Vec<payment::Model>,
}

/// Bộ lọc danh sách đơn của nhân viên.
#[derive(Debug, Clone, Default)]
pub(crate) struct OrderFilter {
    pub status: Option<String>,
    pub payment_status: Option<String>,
    pub org_id: Option<Uuid>,
    pub channel: Option<String>,
    pub number: Option<String>,
}

impl OrdersService {
    pub(crate) async fn order_view(
        &self,
        tenant: &TenantContext,
        row: order::Model,
    ) -> Result<OrderView, Error> {
        let read = tenant.database().read();
        let lines = line::Entity::find()
            .filter(line::Column::OrderId.eq(row.id))
            .scoped(tenant)
            .order_by_asc(line::Column::Id)
            .all(read)
            .await?;
        let payments = payment::Entity::find()
            .filter(payment::Column::OrderId.eq(row.id))
            .scoped(tenant)
            .order_by_asc(payment::Column::Id)
            .all(read)
            .await?;
        Ok(OrderView { order: row, lines, payments })
    }

    /// Đơn `id` của người mua `buyer` (đơn của người khác: 404).
    pub(crate) async fn my_order(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
        id: Uuid,
    ) -> Result<OrderView, Error> {
        let row = self.order(tenant, id).await?;
        if row.buyer_user_id != Some(buyer) {
            return Err(errors::order_not_found());
        }
        self.order_view(tenant, row).await
    }

    /// Một trang đơn của người mua `buyer`, mới trước (mặc định).
    pub(crate) async fn my_orders(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
        page: &PageRequest,
    ) -> Result<Page<order::Model>, Error> {
        let select =
            order::Entity::find().filter(order::Column::BuyerUserId.eq(buyer)).scoped(tenant);
        fetch_page(select, order::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Đơn `id` trong phạm vi `orders.read` của nhân viên.
    pub(crate) async fn staff_order(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<OrderView, Error> {
        let row = self.order(tenant, id).await?;
        let target = OrgTarget::new(OrgId::from_uuid(row.org_id), None);
        principal.require_at(tenant.database().read(), &READ, target).await?;
        self.order_view(tenant, row).await
    }

    /// Một trang đơn trong phạm vi `orders.read`.
    pub(crate) async fn list_orders(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: &OrderFilter,
        page: &PageRequest,
    ) -> Result<Page<order::Model>, Error> {
        let scope = principal.scope_condition(&READ, order::Column::OrgId, None)?;
        let mut condition = Condition::all().add(scope);
        if let Some(status) = &filter.status {
            condition = condition.add(order::Column::Status.eq(status.as_str()));
        }
        if let Some(status) = &filter.payment_status {
            condition = condition.add(order::Column::PaymentStatus.eq(status.as_str()));
        }
        if let Some(org) = filter.org_id {
            condition = condition.add(order::Column::OrgId.eq(org));
        }
        if let Some(channel) = &filter.channel {
            condition = condition.add(order::Column::Channel.eq(channel.as_str()));
        }
        if let Some(number) = &filter.number {
            condition = condition.add(order::Column::Number.eq(number.as_str()));
        }
        let select = order::Entity::find().filter(condition).scoped(tenant);
        fetch_page(select, order::Column::Id, |row| row.id, page, tenant.database().read()).await
    }
}
