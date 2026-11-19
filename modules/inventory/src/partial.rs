//! Trả RIÊNG một phần chỗ giữ của một đơn còn hiệu lực — `orders` huỷ phần đặt trước chưa có hàng
//! (người bán ngừng bán đặt trước, người mua huỷ phần đó) mà đơn vẫn giao phần hàng sẵn
//! (event `orders.preorder_cancelled`, D-20261119-step42-preorder-cancel).
//!
//! Mỗi lần huỷ trả `quantity` đơn vị của dòng `line_key` (đơn vị của dòng đơn; combo nhân `per_unit`
//! cho từng thành phần): cộng lại `available`, cộng `released_quantity`, dòng đặt trước trả suất. Event
//! chỉ mang phần huỷ của lần đó (không cộng dồn) nên chống trùng bằng `inventory_line_releases`:
//! khoá duy nhất theo `reference` (ID event), chèn trong cùng transaction — event đến lại không trả
//! hai lần.

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::rows::Bounds;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde_json::json;
use uuid::Uuid;

use crate::api::ReservationStatus;
use crate::entity::{line_release, reservation_line};
use crate::reservations::{find_by_order, remaining};
use crate::service::{InventoryService, now};
use crate::stock::{self, Touched};

/// Một dòng bị huỷ: `quantity` đơn vị của dòng đơn `line_key`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CancelledLine {
    pub line_key: String,
    pub quantity: i64,
}

impl InventoryService {
    /// Trả phần `lines` của chỗ giữ của đơn `order`, đúng một lần cho mỗi `reference`. Trả số đơn vị
    /// tồn đã trả lại. Đơn không có chỗ giữ, hay chỗ giữ đã đóng → 0 (không lỗi).
    pub(crate) async fn release_lines(
        &self,
        tenant: &TenantContext,
        order: Uuid,
        reference: &str,
        lines: &[CancelledLine],
        actor: &Actor,
    ) -> Result<i64, Error> {
        let database = tenant.database();
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                // Khoá chỗ giữ trước: hai lần giao cùng event (hay với `release` cả đơn) xếp hàng.
                let Some(row) = find_by_order(&transaction, tenant, order, true).await? else {
                    return Ok(0);
                };
                let open = matches!(
                    ReservationStatus::parse(&row.status),
                    Some(ReservationStatus::Held | ReservationStatus::Confirmed)
                );
                if !open {
                    return Ok(0);
                }
                let seen = line_release::Entity::find()
                    .filter(line_release::Column::Reference.eq(reference))
                    .scoped(tenant)
                    .one(&transaction)
                    .await?;
                if seen.is_some() {
                    return Ok(0);
                }
                let mut rows = reservation_line::Entity::find()
                    .filter(reservation_line::Column::ReservationId.eq(row.id))
                    .scoped(tenant)
                    .all(&transaction)
                    .await?;
                // Thứ tự cố định (theo dòng tồn rồi ID) như `release`: không khoá chéo nhau.
                rows.sort_by_key(|line| (line.level_id, line.id));
                let mut touched = Touched::default();
                let mut returned = 0;
                let mut applied = Vec::new();
                for cancelled in lines.iter().filter(|line| line.quantity > 0) {
                    // Mỗi thành phần (biến thể) của dòng cần `quantity × per_unit`.
                    let mut need: std::collections::BTreeMap<Uuid, i64> =
                        std::collections::BTreeMap::new();
                    for line in rows.iter().filter(|line| line.line_key == cancelled.line_key) {
                        need.entry(line.variant_id)
                            .or_insert(cancelled.quantity.saturating_mul(line.per_unit));
                    }
                    for line in rows.iter_mut().filter(|line| line.line_key == cancelled.line_key) {
                        let Some(left) = need.get_mut(&line.variant_id) else { continue };
                        let take = remaining(line).min(*left);
                        if take <= 0 {
                            continue;
                        }
                        *left -= take;
                        if line.preordered {
                            crate::preorders::give_back(
                                &transaction,
                                tenant,
                                line.variant_id,
                                take,
                            )
                            .await?;
                        }
                        if !stock::move_available(
                            &transaction,
                            tenant,
                            &mut touched,
                            line.level_id,
                            take,
                            Bounds::ANY,
                        )
                        .await?
                        {
                            return Err(Error::internal("available stock would overflow"));
                        }
                        line.released_quantity += take;
                        reservation_line::Entity::update_many()
                            .col_expr(
                                reservation_line::Column::ReleasedQuantity,
                                Expr::value(line.released_quantity),
                            )
                            .filter(reservation_line::Column::Id.eq(line.id))
                            .scoped(tenant)
                            .exec(&transaction)
                            .await?;
                        returned += take;
                    }
                    applied.push(
                        json!({"line_key": cancelled.line_key, "quantity": cancelled.quantity}),
                    );
                }
                line_release::Entity::insert(line_release::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    reservation_id: Set(row.id),
                    reference: Set(reference.to_owned()),
                    lines: Set(serde_json::Value::Array(applied).to_string()),
                    created_at: Set(now()),
                })
                .exec_without_returning(&transaction)
                .await?;
                stock::publish(&transaction, tenant, &self.outbox, actor, touched).await?;
                transaction.commit().await?;
                Ok(returned)
            })
            .await
    }
}

pub(crate) mod migration {
    //! Bảng `inventory_line_releases`.

    use autoxide_kernel::migration::prelude::*;

    use crate::migration::{drop_all, index, table};

    pub(crate) struct CreateLineReleases;

    impl MigrationName for CreateLineReleases {
        fn name(&self) -> &str {
            "m0016_create_line_releases"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for CreateLineReleases {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            let mut releases = table("inventory_line_releases");
            releases
                .col(schema::uuid("reservation_id"))
                .col(schema::string_len("reference", 200))
                .col(schema::text("lines"))
                .col(schema::timestamp_with_time_zone("created_at"));
            manager.create_table(releases).await?;
            index(
                manager,
                "inventory_line_releases",
                "inventory_line_releases_reference",
                &["tenant_id", "reference"],
                true,
            )
            .await?;
            index(
                manager,
                "inventory_line_releases",
                "inventory_line_releases_reservation_idx",
                &["tenant_id", "reservation_id"],
                false,
            )
            .await
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            drop_all(manager, &["inventory_line_releases"]).await
        }
    }
}
