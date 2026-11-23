//! Hàng đặt trước (bước 42 mục 7, D-204).
//!
//! - Suất: biến thể `sale_mode = preorder` có `preorder_limit` (của `catalog`) thì tổng đang giữ / đã
//!   xác nhận không vượt giới hạn. Tăng `reserved` của `inventory_preorder_counters` bằng MỘT câu
//!   `UPDATE … WHERE reserved + q <= limit` (`rows::adjust` với trần là giới hạn) — không đọc rồi ghi,
//!   nên 100 người đặt cùng lúc giới hạn 10 thì đúng 10 người được (`inventory.preorder_sold_out`).
//!   Trả chỗ thì trừ lại.
//! - Hàng về (nhập, chuyển đến, trả về, điều chỉnh tăng): job `inventory.allocate_preorders` chia
//!   tồn mới cho các dòng đặt trước của dòng tồn đó **theo thứ tự đặt** (ID dòng UUIDv7), cộng
//!   `allocated_quantity` và phát `inventory.preorder_allocated` từng phần để `orders` chuyển đơn sang
//!   chuẩn bị hàng. Job tính lại từ trạng thái nên chạy lại (event nhập kho đến lại) không chia hai lần.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler, JobQueue};
use autoxide_kernel::rows::{self, Bounds};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::api::ReservationStatus;
use crate::entity::{level, preorder_counter, reservation, reservation_line};
use crate::events::{self, Change};
use crate::service::{InventoryService, now};

pub(crate) const ALLOCATE_PREORDERS: &str = "inventory.allocate_preorders";

/// Payload của job chia hàng về cho một dòng tồn.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct AllocatePreorders {
    pub level_id: Uuid,
}

impl Job for AllocatePreorders {
    const KIND: &'static str = ALLOCATE_PREORDERS;
}

/// Lỗi `inventory.preorder_sold_out`.
pub(crate) fn sold_out(variant: Uuid) -> Error {
    Error::conflict(
        "inventory.preorder_sold_out",
        "The pre-order places of variant `{variant_id}` are all taken.",
    )
    .with_param("variant_id", variant)
}

fn counter_filter(tenant: &TenantContext, variant: Uuid) -> Condition {
    Condition::all()
        .add(preorder_counter::Column::TenantId.eq(tenant.id().as_uuid()))
        .add(preorder_counter::Column::VariantId.eq(variant))
}

/// Lấy `quantity` suất đặt trước của biến thể `variant` (giới hạn `limit`, chép từ `catalog` mỗi
/// lần). Hết suất → `inventory.preorder_sold_out`.
pub(crate) async fn take<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    variant: Uuid,
    limit: i64,
    quantity: i64,
) -> Result<(), Error> {
    preorder_counter::Entity::insert(preorder_counter::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant.id().as_uuid()),
        variant_id: Set(variant),
        reserved: Set(0),
        preorder_limit: Set(limit),
        updated_at: Set(now()),
    })
    .on_conflict(
        OnConflict::columns([
            preorder_counter::Column::TenantId,
            preorder_counter::Column::VariantId,
        ])
        .do_nothing()
        .to_owned(),
    )
    .exec_without_returning(transaction)
    .await?;
    preorder_counter::Entity::update_many()
        .col_expr(preorder_counter::Column::PreorderLimit, Expr::value(limit))
        .col_expr(preorder_counter::Column::UpdatedAt, Expr::value(now()))
        .filter(counter_filter(tenant, variant))
        .exec(transaction)
        .await?;
    let changed = rows::adjust::<preorder_counter::Entity, _>(
        transaction,
        preorder_counter::Column::Reserved,
        counter_filter(tenant, variant),
        quantity,
        Bounds { floor: 0, ceiling: limit.max(0) },
    )
    .await?;
    if changed == 0 {
        return Err(sold_out(variant));
    }
    Ok(())
}

/// Trả `quantity` suất (huỷ, hết hạn). Biến thể không giới hạn suất thì không có bộ đếm: không làm gì.
pub(crate) async fn give_back<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    variant: Uuid,
    quantity: i64,
) -> Result<(), Error> {
    rows::adjust::<preorder_counter::Entity, _>(
        transaction,
        preorder_counter::Column::Reserved,
        counter_filter(tenant, variant),
        -quantity,
        Bounds::NON_NEGATIVE,
    )
    .await?;
    Ok(())
}

/// Xếp job chia hàng về cho dòng tồn `level` khi nó có dòng đặt trước đang chờ (gọi trong transaction
/// của biến động làm tăng `on_hand`). Job tính lại từ trạng thái nên xếp thừa cũng không sao.
pub(crate) async fn schedule<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    level: Uuid,
) -> Result<(), Error> {
    // `ExprTrait` chỉ nhập ở đây: nhập cả file thì `i64::max` / `min` bị trùng tên với nó.
    use sea_orm::sea_query::ExprTrait;
    let waiting = reservation_line::Entity::find()
        .filter(reservation_line::Column::LevelId.eq(level))
        .filter(reservation_line::Column::Preordered.eq(true))
        .filter(
            Expr::col(reservation_line::Column::AllocatedQuantity)
                .lt(Expr::col(reservation_line::Column::Quantity)
                    .sub(Expr::col(reservation_line::Column::ReleasedQuantity))),
        )
        .scoped(tenant)
        .one(transaction)
        .await?;
    if waiting.is_some() {
        JobQueue::new("inventory")
            .enqueue(
                transaction,
                tenant,
                &AllocatePreorders { level_id: level },
                EnqueueOptions::default(),
            )
            .await?;
    }
    Ok(())
}

impl InventoryService {
    /// Chia hàng hiện có của dòng tồn `level_id` cho các dòng đặt trước đang chờ, theo thứ tự đặt.
    /// Trả số đơn vị vừa chia.
    pub(crate) async fn allocate_preorders(
        &self,
        tenant: &TenantContext,
        level_id: Uuid,
    ) -> Result<i64, Error> {
        let database = tenant.database();
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let Some(level) = rows::lock_one(
                    &transaction,
                    level::Entity::find_by_id(level_id).scoped(tenant),
                )
                .await?
                else {
                    return Ok(0);
                };
                let open: Vec<Uuid> = reservation::Entity::find()
                    .filter(reservation::Column::Status.is_in([
                        ReservationStatus::Held.as_str(),
                        ReservationStatus::Confirmed.as_str(),
                    ]))
                    .scoped(tenant)
                    .all(&transaction)
                    .await?
                    .into_iter()
                    .map(|row| row.id)
                    .collect();
                let lines = reservation_line::Entity::find()
                    .filter(reservation_line::Column::LevelId.eq(level_id))
                    .filter(reservation_line::Column::ReservationId.is_in(open))
                    .scoped(tenant)
                    .order_by_asc(reservation_line::Column::Id)
                    .all(&transaction)
                    .await?;
                let remaining = |line: &reservation_line::Model| {
                    line.quantity - line.fulfilled_quantity - line.released_quantity
                };
                // Hàng trong kho đã dành cho dòng thường và cho phần đặt trước đã chia mà chưa xuất.
                let committed: i64 = lines
                    .iter()
                    .map(|line| {
                        if line.preordered {
                            (line.allocated_quantity - line.fulfilled_quantity).max(0)
                        } else {
                            remaining(line)
                        }
                    })
                    .sum();
                let mut free = level.on_hand - committed;
                let mut given = 0;
                for line in lines.iter().filter(|line| line.preordered) {
                    if free <= 0 {
                        break;
                    }
                    let open_allocated = (line.allocated_quantity - line.fulfilled_quantity).max(0);
                    let need = remaining(line) - open_allocated;
                    if need <= 0 {
                        continue;
                    }
                    let part = need.min(free);
                    free -= part;
                    given += part;
                    reservation_line::Entity::update_many()
                        .col_expr(
                            reservation_line::Column::AllocatedQuantity,
                            Expr::value(line.allocated_quantity + part),
                        )
                        .filter(reservation_line::Column::Id.eq(line.id))
                        .scoped(tenant)
                        .exec(&transaction)
                        .await?;
                    let order = reservation::Entity::find_by_id(line.reservation_id)
                        .scoped(tenant)
                        .one(&transaction)
                        .await?
                        .map(|row| row.order_id);
                    let after = json!({
                        "reservation_id": line.reservation_id,
                        "order_id": order,
                        "line_key": line.line_key,
                        "variant_id": line.variant_id,
                        "warehouse_org_id": line.org_id,
                        "quantity": part,
                        "allocated_quantity": line.allocated_quantity + part,
                    });
                    self.outbox
                        .record(
                            &transaction,
                            tenant,
                            Actor::system(),
                            Some(format!("inventory.reservation:{}", line.reservation_id)),
                            &events::PreorderAllocated(Change::after(after)),
                        )
                        .await?;
                }
                transaction.commit().await?;
                Ok(given)
            })
            .await
    }
}

pub(crate) struct Allocate(pub Arc<InventoryService>);

#[async_trait::async_trait]
impl JobHandler for Allocate {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let tenant =
            job.tenant.as_ref().ok_or_else(|| Error::internal("pre-order job without a tenant"))?;
        let payload: AllocatePreorders =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        self.0.allocate_preorders(tenant, payload.level_id).await.map(|_| ())
    }
}
