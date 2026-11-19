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
    /// Nhận tại cửa hàng, khi đã sẵn sàng.
    pub pickup: Option<crate::entity::pickup::Model>,
}

/// Bộ lọc danh sách đơn của nhân viên.
#[derive(Debug, Clone, Default)]
pub(crate) struct OrderFilter {
    pub status: Option<String>,
    pub payment_status: Option<String>,
    pub org_id: Option<Uuid>,
    pub channel: Option<String>,
    pub number: Option<String>,
    pub pickup_org_id: Option<Uuid>,
    /// `ready`, `picked_up`, `expired`.
    pub pickup_status: Option<String>,
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
        let pickup = crate::entity::pickup::Entity::find()
            .filter(crate::entity::pickup::Column::OrderId.eq(row.id))
            .scoped(tenant)
            .one(read)
            .await?;
        Ok(OrderView { order: row, lines, payments, pickup })
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
        // Không có quyền đọc đơn ở đâu cả: 403 trước khi tìm đơn (không lộ đơn có hay không).
        principal.require(&READ)?;
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
        if let Some(org) = filter.pickup_org_id {
            condition = condition.add(order::Column::PickupOrgId.eq(org));
        }
        if let Some(status) = &filter.pickup_status {
            // Đơn có dòng `orders_pickups` mang trạng thái này (bảng của chính module).
            let ids = sea_orm::sea_query::Query::select()
                .column(crate::entity::pickup::Column::OrderId)
                .from(crate::entity::pickup::Entity)
                .and_where(crate::entity::pickup::Column::TenantId.eq(tenant.id().as_uuid()))
                .and_where(crate::entity::pickup::Column::Status.eq(status.as_str()))
                .to_owned();
            condition = condition.add(order::Column::Id.in_subquery(ids));
        }
        let select = order::Entity::find().filter(condition).scoped(tenant);
        fetch_page(select, order::Column::Id, |row| row.id, page, tenant.database().read()).await
    }
}

/// `Orders` cho module khác (bước 46–49): cùng dữ liệu với REST.
#[async_trait::async_trait]
impl crate::api::Orders for OrdersService {
    async fn find(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Option<crate::api::OrderSummary>, Error> {
        let Some(row) =
            order::Entity::find_by_id(id).scoped(tenant).one(tenant.database().read()).await?
        else {
            return Ok(None);
        };
        let view = self.order_view(tenant, row).await?;
        summary(view).map(Some)
    }

    async fn delivered_lines(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
        variant_id: Uuid,
    ) -> Result<Vec<Uuid>, Error> {
        use sea_orm::QuerySelect;
        let delivered = sea_orm::sea_query::Query::select()
            .column(order::Column::Id)
            .from(order::Entity)
            .and_where(order::Column::TenantId.eq(tenant.id().as_uuid()))
            .and_where(order::Column::BuyerUserId.eq(buyer))
            .and_where(order::Column::Status.is_in(["delivered", "completed"]))
            .to_owned();
        Ok(line::Entity::find()
            .select_only()
            .column(line::Column::Id)
            .filter(line::Column::VariantId.eq(variant_id))
            .filter(line::Column::OrderId.in_subquery(delivered))
            .scoped(tenant)
            .into_tuple()
            .all(tenant.database().read())
            .await?)
    }
}

fn summary(view: OrderView) -> Result<crate::api::OrderSummary, Error> {
    use autoxide_kernel::money::Money;
    let row = &view.order;
    let money = |minor: i64| Money::from_columns(minor, &row.currency);
    let lines = view
        .lines
        .iter()
        .map(|line| {
            // Kiểu `#[non_exhaustive]` của chính crate: dựng trực tiếp được.
            Ok(crate::api::OrderLineSummary {
                id: line.id,
                variant_id: line.variant_id,
                product_id: line.product_id,
                category_id: line.category_id,
                seller_org_id: OrgId::from_uuid(line.seller_org_id),
                quantity: line.quantity,
                net: money(line.total_minor)?,
                tax: money(line.tax_minor)?,
                returned_quantity: line.returned_quantity,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let status = crate::api::OrderStatus::parse(&row.status)
        .ok_or_else(|| Error::internal(format!("unknown order status `{}`", row.status)))?;
    let funds_flow = crate::api::FundsFlow::parse(&row.funds_flow)
        .ok_or_else(|| Error::internal(format!("unknown funds flow `{}`", row.funds_flow)))?;
    Ok(crate::api::OrderSummary {
        id: row.id,
        tenant_id: autoxide_kernel::tenancy::TenantId::from_uuid(row.tenant_id),
        org_id: OrgId::from_uuid(row.org_id),
        number: row.number.clone(),
        buyer_user_id: row.buyer_user_id,
        channel: row.channel.clone(),
        status,
        funds_flow,
        total: money(row.total_minor)?,
        placed_at: row.placed_at,
        lines,
        attribution: row.attribution.as_deref().and_then(|raw| serde_json::from_str(raw).ok()),
    })
}
