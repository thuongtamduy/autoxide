//! Giỏ bỏ dở (B-b9): người mua đã biết (đăng nhập) để giỏ không đổi quá
//! `AUTOXIDE_ORDERS_ABANDONED_AFTER_MINUTES` → giỏ `abandoned` và event `orders.cart_abandoned`
//! đúng một lần (bước 47 gửi nhắc). Sửa giỏ thì giỏ `active` lại. Nhân viên xem danh sách bằng
//! `GET /carts?status=abandoned`. Giỏ quá `AUTOXIDE_ORDERS_CART_TTL_DAYS` bị xoá.

use autoxide_kernel::access::Principal;
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext, TenantId};
use jiff::SignedDuration;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde_json::json;
use uuid::Uuid;

use crate::carts::{ABANDONED, ACTIVE};
use crate::entity::{cart, cart_line};
use crate::events::{CartAbandoned, Change};
use crate::jobs::BATCH;
use crate::permissions::READ;
use crate::service::OrdersService;

impl OrdersService {
    /// Một lô giỏ bỏ dở trong `database` (mọi khách); trả số giỏ đã đánh dấu.
    pub(crate) async fn mark_abandoned(&self, database: &Database) -> Result<u64, Error> {
        let before =
            crate::service::now() - SignedDuration::from_mins(self.config.abandoned_after_minutes);
        let with_lines = sea_orm::sea_query::Query::select()
            .column(cart_line::Column::CartId)
            .from(cart_line::Entity)
            .to_owned();
        // tenant-scope: job quét giỏ bỏ dở của mọi khách; xử lý từng giỏ theo `tenant_id` của nó.
        let due = cart::Entity::find()
            .filter(cart::Column::Status.eq(ACTIVE))
            .filter(cart::Column::BuyerUserId.is_not_null())
            .filter(cart::Column::UpdatedAt.lt(before))
            .filter(cart::Column::Id.in_subquery(with_lines))
            .order_by_asc(cart::Column::UpdatedAt)
            .limit(BATCH)
            .all(database.read())
            .await?;
        let mut marked = 0;
        for row in due {
            let tenant = self.databases.tenant(TenantId::from_uuid(row.tenant_id)).await?;
            if self.mark_one_abandoned(&tenant, &row).await? {
                marked += 1;
            }
        }
        Ok(marked)
    }

    async fn mark_one_abandoned(
        &self,
        tenant: &TenantContext,
        row: &cart::Model,
    ) -> Result<bool, Error> {
        let lines = cart_line::Entity::find()
            .filter(cart_line::Column::CartId.eq(row.id))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        let summary: Vec<_> = lines
            .iter()
            .map(|line| json!({"variant_id": line.variant_id, "quantity": line.quantity}))
            .collect();
        let event = CartAbandoned(Change::after(json!({
            "cart_id": row.id,
            "tenant_id": row.tenant_id,
            "org_id": row.org_id,
            "buyer_user_id": row.buyer_user_id,
            "currency": row.currency,
            "lines": summary,
            "updated_at": row.updated_at,
        })));
        let (database, outbox) = (tenant.database(), &self.outbox);
        let (id, updated_at, event) = (row.id, row.updated_at, &event);
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                // Chỉ khi giỏ chưa đổi từ lúc đọc (người mua sửa giỏ thì thôi).
                let changed = cart::Entity::update_many()
                    .col_expr(cart::Column::Status, Expr::value(ABANDONED))
                    .col_expr(cart::Column::AbandonedNotifiedAt, Expr::value(crate::service::now()))
                    .filter(cart::Column::Id.eq(id))
                    .filter(cart::Column::Status.eq(ACTIVE))
                    .filter(cart::Column::UpdatedAt.eq(updated_at))
                    .scoped(tenant)
                    .exec(&transaction)
                    .await?
                    .rows_affected;
                if changed == 0 {
                    return Ok(false);
                }
                outbox
                    .record(
                        &transaction,
                        tenant,
                        Actor::system(),
                        Some(format!("orders.cart:{id}")),
                        event,
                    )
                    .await?;
                transaction.commit().await?;
                Ok(true)
            })
            .await
    }

    /// Xoá một lô giỏ quá hạn trong `database` (mọi khách); trả số giỏ đã xoá.
    pub(crate) async fn purge_carts(&self, database: &Database) -> Result<u64, Error> {
        let due: Vec<(Uuid, Uuid)> = cart::Entity::find()
            .select_only()
            .column(cart::Column::TenantId)
            .column(cart::Column::Id)
            .filter(cart::Column::ExpiresAt.lt(crate::service::now()))
            .limit(BATCH * 5)
            .into_tuple()
            .all(database.read())
            .await?;
        let mut purged = 0;
        for (tenant_id, id) in due {
            let transaction = database.begin_write().await?;
            cart_line::Entity::delete_many()
                .filter(cart_line::Column::TenantId.eq(tenant_id))
                .filter(cart_line::Column::CartId.eq(id))
                .exec(&transaction)
                .await?;
            purged += cart::Entity::delete_many()
                .filter(cart::Column::TenantId.eq(tenant_id))
                .filter(cart::Column::Id.eq(id))
                .exec(&transaction)
                .await?
                .rows_affected;
            transaction.commit().await?;
        }
        Ok(purged)
    }

    /// Một trang giỏ theo trạng thái trong phạm vi `orders.read` của nhân viên, mới trước.
    pub(crate) async fn list_carts(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        status: &str,
        org: Option<Uuid>,
        page: &PageRequest,
    ) -> Result<Page<cart::Model>, Error> {
        let scope = principal.scope_condition(&READ, cart::Column::OrgId, None)?;
        let mut condition = Condition::all().add(scope).add(cart::Column::Status.eq(status));
        if let Some(org) = org {
            condition = condition.add(cart::Column::OrgId.eq(org));
        }
        let select = cart::Entity::find().filter(condition).scoped(tenant);
        fetch_page(select, cart::Column::Id, |row| row.id, page, tenant.database().read()).await
    }
}
