//! Giữ chỗ tồn theo đơn (bước 42 mục 2): giữ có hạn → xác nhận → xuất / trả lại. Dùng chung cho
//! trait `api::Inventory` (`orders` gọi thẳng), event của `orders`, REST và job hết hạn — một hàm
//! cho mỗi việc, chịu được gọi lại.
//!
//! Không bao giờ bán quá tồn: mỗi phần giữ là MỘT câu `UPDATE … SET available = available − q
//! WHERE … AND available >= q` (`rows::adjust`). Chọn kho đọc `available` trong transaction rồi để
//! câu đó làm trọng tài: đọc đã cũ (Postgres, bên khác vừa giữ) thì câu trả 0 dòng, cả transaction
//! bỏ, chạy lại với số đọc mới (tối đa [`PLAN_ATTEMPTS`] lần) — không "cộng bù" bằng tay.
//!
//! Mọi chuyển trạng thái khoá dòng chỗ giữ trước (`rows::lock_one`): xác nhận, trả, xuất, hết hạn
//! của cùng một đơn nối đuôi nhau; chỉ bên đổi được trạng thái mới cộng / trừ tồn.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::collections::{BTreeMap, HashMap};

use autoxide_catalog::api::{ProductKind, SaleMode, VariantSnapshot};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::reference::{self, Reference, Submitted};
use autoxide_kernel::rows::{self, Bounds};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, QueryFilter,
    QueryOrder,
};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::api::{
    FulfilRequest, ReleaseReason, Reservation, ReservationStatus, ReserveRequest, ReservedLine,
    RestockRequest,
};
use crate::entity::{fulfilment, level, reservation, reservation_line, restock, settings, source};
use crate::errors;
use crate::events::{self, Change};
use crate::service::{InventoryService, MAX_QUANTITY, check_quantity, fail_if_any, now};
use crate::stock::{self, MovementKind, NewMovement, Touched};

/// Số lần chọn kho lại khi số đọc đã cũ.
const PLAN_ATTEMPTS: usize = 4;

/// Mã nội bộ: số `available` đọc đã cũ, chọn kho lại (không bao giờ ra ngoài).
const STALE: &str = "inventory.stock_moved";

/// Một phần phải giữ: một biến thể thật của một dòng đơn.
#[derive(Debug, Clone)]
struct Need {
    line_key: String,
    variant: Uuid,
    bundle: Option<Uuid>,
    per_unit: i64,
    quantity: i64,
    /// Cho `available` âm (bán khi hết hàng, đặt trước).
    negative_ok: bool,
    preorder: bool,
    /// Giới hạn suất đặt trước (của `catalog`); `None` = không giới hạn.
    preorder_limit: Option<i64>,
}

/// Một phần đã chọn kho.
#[derive(Debug, Clone)]
struct Pick {
    need: usize,
    org: Uuid,
    quantity: i64,
    backordered: bool,
}

/// Nội dung quyết định kết quả của lệnh giữ (không gồm `expires_at`: gọi lại có thể lệch vài
/// giây).
#[derive(Serialize)]
struct ReserveFingerprint<'a> {
    selling_org_id: Uuid,
    fixed_org_id: Option<Uuid>,
    lines: Vec<(&'a str, Uuid, i64)>,
}

fn stale() -> Error {
    Error::conflict(STALE, "Stock changed while it was being reserved.")
}

/// Mã lỗi `inventory.reservation_not_found`.
pub(crate) fn not_found(order: Uuid) -> Error {
    Error::not_found(
        "inventory.reservation_not_found",
        "There is no stock reservation for order `{order_id}`.",
    )
    .with_param("order_id", order)
}

pub(crate) fn expired(order: Uuid) -> Error {
    Error::conflict(
        "inventory.reservation_expired",
        "The stock reservation of order `{order_id}` expired or was released; reserve again.",
    )
    .with_param("order_id", order)
}

fn status_of(row: &reservation::Model) -> Result<ReservationStatus, Error> {
    ReservationStatus::parse(&row.status)
        .ok_or_else(|| Error::internal(format!("unknown reservation status `{}`", row.status)))
}

/// Phần còn giữ (chưa xuất, chưa trả) của một dòng.
pub(crate) fn remaining(line: &reservation_line::Model) -> i64 {
    line.quantity - line.fulfilled_quantity - line.released_quantity
}

/// Đọc chỗ giữ của đơn `order` kèm các dòng.
pub(crate) async fn load<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    row: reservation::Model,
) -> Result<Reservation, Error> {
    let lines = reservation_line::Entity::find()
        .filter(reservation_line::Column::ReservationId.eq(row.id))
        .scoped(tenant)
        .order_by_asc(reservation_line::Column::Id)
        .all(connection)
        .await?;
    present(row, &lines)
}

fn present(
    row: reservation::Model,
    lines: &[reservation_line::Model],
) -> Result<Reservation, Error> {
    let status = status_of(&row)?;
    let lines = lines
        .iter()
        .map(|line| ReservedLine {
            line_key: line.line_key.clone(),
            variant_id: line.variant_id,
            bundle_variant_id: line.bundle_variant_id,
            warehouse_org_id: OrgId::from_uuid(line.org_id),
            quantity: line.quantity,
            fulfilled_quantity: line.fulfilled_quantity,
            released_quantity: line.released_quantity,
            backordered: line.backordered,
            preordered: line.preordered,
            allocated_quantity: line.allocated_quantity,
        })
        .collect();
    Ok(Reservation {
        id: row.id,
        order_id: row.order_id,
        selling_org_id: OrgId::from_uuid(row.selling_org_id),
        status,
        expires_at: row.expires_at,
        lines,
    })
}

/// Dữ liệu chung của event về chỗ giữ.
fn snapshot(reservation: &Reservation) -> serde_json::Value {
    json!({
        "id": reservation.id,
        "order_id": reservation.order_id,
        "selling_org_id": reservation.selling_org_id,
        "status": reservation.status.as_str(),
        "expires_at": reservation.expires_at,
        "lines": reservation.lines,
    })
}

fn subject(id: Uuid) -> Option<String> {
    Some(format!("inventory.reservation:{id}"))
}

pub(crate) async fn find_by_order<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    order: Uuid,
    lock: bool,
) -> Result<Option<reservation::Model>, Error> {
    let select =
        reservation::Entity::find().filter(reservation::Column::OrderId.eq(order)).scoped(tenant);
    Ok(if lock {
        rows::lock_one(connection, select).await?
    } else {
        select.one(connection).await?
    })
}

/// Đổi trạng thái chỗ giữ `row` có điều kiện (`WHERE status = <cũ>`); `false`: bên khác đã đổi.
async fn set_status(
    transaction: &DatabaseTransaction,
    tenant: &TenantContext,
    row: &reservation::Model,
    to: ReservationStatus,
    reason: Option<ReleaseReason>,
) -> Result<bool, Error> {
    let mut update = reservation::Entity::update_many()
        .col_expr(reservation::Column::Status, Expr::value(to.as_str()))
        .col_expr(reservation::Column::UpdatedAt, Expr::value(now()));
    if to != ReservationStatus::Held {
        update = update.col_expr(reservation::Column::ExpiresAt, Expr::value(None::<Timestamp>));
    }
    if let Some(reason) = reason {
        update = update.col_expr(reservation::Column::ReleaseReason, Expr::value(reason.as_str()));
    }
    let changed = update
        .filter(reservation::Column::Id.eq(row.id))
        .filter(reservation::Column::Status.eq(row.status.clone()))
        .scoped(tenant)
        .exec(transaction)
        .await?
        .rows_affected;
    Ok(changed == 1)
}

impl InventoryService {
    /// Kho ứng viên của lệnh giữ, theo thứ tự thử; cờ "cho chia kho" của nút bán.
    pub(crate) async fn candidates(
        &self,
        tenant: &TenantContext,
        selling: Uuid,
        fixed: Option<Uuid>,
        province: Option<&str>,
    ) -> Result<(Vec<Uuid>, bool), Error> {
        let read = tenant.database().read();
        let own = settings::Entity::find()
            .filter(settings::Column::OrgId.eq(selling))
            .scoped(tenant)
            .one(read)
            .await?;
        let (prefer_nearby, allow_split) =
            own.as_ref().map_or((false, false), |row| (row.prefer_nearby, row.allow_split));
        if let Some(fixed) = fixed {
            self.check_stock_unit(tenant, fixed).await?;
            return Ok((vec![fixed], false));
        }
        let sources = source::Entity::find()
            .filter(source::Column::SellingOrgId.eq(selling))
            .filter(source::Column::Active.eq(true))
            .scoped(tenant)
            .order_by_asc(source::Column::Priority)
            .order_by_asc(source::Column::Id)
            .all(read)
            .await?;
        let mut orgs: Vec<Uuid> = sources.iter().map(|row| row.warehouse_org_id).collect();
        if orgs.is_empty() {
            if self.check_stock_unit(tenant, selling).await.is_err() {
                return Err(errors::no_source(selling));
            }
            orgs.push(selling);
        }
        if prefer_nearby && let Some(province) = province {
            let rows = settings::Entity::find()
                .filter(settings::Column::OrgId.is_in(orgs.clone()))
                .scoped(tenant)
                .all(read)
                .await?;
            let near: Vec<Uuid> = rows
                .iter()
                .filter(|row| row.province_code.as_deref() == Some(province))
                .map(|row| row.org_id)
                .collect();
            // Sắp ổn định: cùng tỉnh trước, giữ thứ tự ưu tiên trong mỗi nhóm.
            orgs.sort_by_key(|org| !near.contains(org));
        }
        Ok((orgs, allow_split))
    }

    /// Phần phải giữ của lệnh: tách combo thành thành phần, bỏ biến thể không theo dõi tồn. Gọi
    /// `Catalog` TRƯỚC transaction.
    async fn needs(
        &self,
        tenant: &TenantContext,
        request: &ReserveRequest,
    ) -> Result<Vec<Need>, Error> {
        let ids: Vec<Uuid> = request.lines.iter().map(|line| line.variant_id).collect();
        let variants = self.variants(tenant, &ids).await?;
        let bundle_ids: Vec<Uuid> = variants
            .values()
            .filter(|variant| variant.kind == ProductKind::Bundle)
            .map(|variant| variant.variant_id)
            .collect();
        let components = if bundle_ids.is_empty() {
            HashMap::new()
        } else {
            self.catalog.bundle_components(tenant, &bundle_ids).await?
        };
        let part_ids: Vec<Uuid> =
            components.values().flatten().map(|part| part.variant_id).collect();
        let parts = if part_ids.is_empty() {
            HashMap::new()
        } else {
            self.variants(tenant, &part_ids).await?
        };
        let mut needs = Vec::new();
        let push = |needs: &mut Vec<Need>,
                    line_key: &str,
                    variant: &VariantSnapshot,
                    bundle: Option<Uuid>,
                    per_unit: i64,
                    count: i64|
         -> Result<(), Error> {
            if !variant.track_inventory {
                return Ok(());
            }
            let quantity = count
                .checked_mul(per_unit)
                .filter(|quantity| *quantity <= MAX_QUANTITY)
                .ok_or_else(|| {
                    Error::validation(vec![
                        FieldError::new("lines", "range", "a line needs more than {max} units")
                            .with_param("max", MAX_QUANTITY),
                    ])
                })?;
            needs.push(Need {
                line_key: line_key.to_owned(),
                variant: variant.variant_id,
                bundle,
                per_unit,
                quantity,
                negative_ok: matches!(variant.sale_mode, SaleMode::Backorder | SaleMode::Preorder),
                preorder: variant.sale_mode == SaleMode::Preorder,
                preorder_limit: variant
                    .preorder_limit
                    .filter(|_| variant.sale_mode == SaleMode::Preorder),
            });
            Ok(())
        };
        for line in &request.lines {
            let Some(variant) = variants.get(&line.variant_id) else { continue };
            if variant.kind == ProductKind::Bundle {
                for part in components.get(&line.variant_id).into_iter().flatten() {
                    if let Some(snapshot) = parts.get(&part.variant_id) {
                        push(
                            &mut needs,
                            &line.line_key,
                            snapshot,
                            Some(line.variant_id),
                            part.quantity,
                            line.quantity,
                        )?;
                    }
                }
            } else {
                push(&mut needs, &line.line_key, variant, None, 1, line.quantity)?;
            }
        }
        Ok(needs)
    }

    fn check_reserve(&self, request: &ReserveRequest) -> Result<(), Error> {
        let mut problems = Vec::new();
        let max = self.config.max_lines_per_reservation;
        if request.lines.is_empty() || request.lines.len() > max {
            problems.push(
                FieldError::new("lines", "length", "must hold 1 to {max} lines")
                    .with_param("max", max),
            );
        }
        let mut keys = std::collections::BTreeSet::new();
        for (index, line) in request.lines.iter().enumerate() {
            check_quantity(
                &mut problems,
                &format!("lines[{index}].quantity"),
                line.quantity,
                false,
            );
            if !(1..=100).contains(&line.line_key.chars().count())
                || !keys.insert(line.line_key.as_str())
            {
                problems.push(FieldError::new(
                    format!("lines[{index}].line_key"),
                    "invalid",
                    "must be 1 to 100 characters and unique in the order",
                ));
            }
        }
        if request.expires_at <= now() {
            problems.push(FieldError::new("expires_at", "range", "must be in the future"));
        }
        fail_if_any(problems)?;
        if request.expires_at > now() + SignedDuration::from_hours(self.config.max_hold_hours) {
            return Err(Error::new(
                ErrorKind::Validation,
                "inventory.hold_too_long",
                "Stock can be held for at most {max} hours.",
            )
            .with_param("max", self.config.max_hold_hours));
        }
        Ok(())
    }

    /// Giữ hàng cho cả đơn (xem `api::Inventory::reserve`).
    pub(crate) async fn reserve(
        &self,
        tenant: &TenantContext,
        request: &ReserveRequest,
    ) -> Result<Submitted<Reservation>, Error> {
        self.check_reserve(request)?;
        let needs = self.needs(tenant, request).await?;
        let selling = request.selling_org_id.as_uuid();
        let fixed = request.fixed_org_id.map(|org| org.as_uuid());
        let (candidates, allow_split) =
            self.candidates(tenant, selling, fixed, request.ship_to_province.as_deref()).await?;
        let fingerprint = reference::fingerprint(&ReserveFingerprint {
            selling_org_id: selling,
            fixed_org_id: fixed,
            lines: request
                .lines
                .iter()
                .map(|line| (line.line_key.as_str(), line.variant_id, line.quantity))
                .collect(),
        })?;
        let order_reference =
            Reference::checked(&request.order_id.to_string(), "order_id", &mut Vec::new())
                .ok_or_else(|| Error::internal("an order id is always a valid reference"))?;
        let database = tenant.database();
        let (needs, candidates, fingerprint, order_reference) =
            (&needs, &candidates, &fingerprint, &order_reference);
        for _ in 0..PLAN_ATTEMPTS {
            let outcome = reference::submit_once(database, move || async move {
                let transaction = database.begin_write().await?;
                let stored = find_by_order(&transaction, tenant, request.order_id, false)
                    .await?
                    .map(|row| {
                        let print = row.fingerprint.clone();
                        (row, print)
                    });
                if let Some(row) = reference::replayed(stored, order_reference, fingerprint)? {
                    return Ok(Submitted::Replayed(load(&transaction, tenant, row).await?));
                }
                let reservation = self
                    .hold(
                        &transaction,
                        tenant,
                        request,
                        needs,
                        candidates,
                        allow_split,
                        fingerprint,
                    )
                    .await?;
                transaction.commit().await?;
                Ok(Submitted::Created(reservation))
            })
            .await;
            match outcome {
                Err(error) if error.code() == STALE => continue,
                outcome => return outcome,
            }
        }
        Err(errors::insufficient_stock("the order (stock is changing fast; try again)"))
    }

    /// Phần trong transaction của [`reserve`](Self::reserve): đọc tồn, chọn kho, trừ `available`.
    #[allow(clippy::too_many_arguments)]
    async fn hold(
        &self,
        transaction: &DatabaseTransaction,
        tenant: &TenantContext,
        request: &ReserveRequest,
        needs: &[Need],
        candidates: &[Uuid],
        allow_split: bool,
        fingerprint: &str,
    ) -> Result<Reservation, Error> {
        let variant_ids: Vec<Uuid> = needs.iter().map(|need| need.variant).collect();
        let levels = level::Entity::find()
            .filter(level::Column::OrgId.is_in(candidates.to_vec()))
            .filter(level::Column::VariantId.is_in(variant_ids))
            .scoped(tenant)
            .all(transaction)
            .await?;
        let mut available: HashMap<(Uuid, Uuid), i64> =
            levels.iter().map(|row| ((row.org_id, row.variant_id), row.available)).collect();
        let picks = plan(needs, candidates, allow_split, &mut available)?;
        // Tạo dòng tồn thiếu (bán khi hết hàng ở kho chưa có tồn), rồi trừ theo ID dòng tăng dần.
        let mut ordered = Vec::with_capacity(picks.len());
        for pick in &picks {
            let need = &needs[pick.need];
            let level = stock::level_id(transaction, tenant, pick.org, need.variant).await?;
            ordered.push((level, pick));
        }
        ordered.sort_by_key(|(level, _)| *level);
        // Suất đặt trước: theo thứ tự ID biến thể, trước khi trừ tồn (một câu `UPDATE` có trần).
        let mut quotas: std::collections::BTreeMap<Uuid, (i64, i64)> =
            std::collections::BTreeMap::new();
        for need in needs {
            if let Some(limit) = need.preorder_limit {
                let entry = quotas.entry(need.variant).or_insert((limit, 0));
                entry.1 += need.quantity;
            }
        }
        for (variant, (limit, quantity)) in quotas {
            crate::preorders::take(transaction, tenant, variant, limit, quantity).await?;
        }
        let mut touched = Touched::default();
        for (level, pick) in &ordered {
            let bounds =
                if needs[pick.need].negative_ok { Bounds::ANY } else { Bounds::NON_NEGATIVE };
            if !stock::move_available(
                transaction,
                tenant,
                &mut touched,
                *level,
                -pick.quantity,
                bounds,
            )
            .await?
            {
                return Err(stale());
            }
        }
        let stamp = now();
        let row = reservation::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            order_id: Set(request.order_id),
            selling_org_id: Set(request.selling_org_id.as_uuid()),
            status: Set(ReservationStatus::Held.as_str().to_owned()),
            expires_at: Set(Some(request.expires_at)),
            fingerprint: Set(fingerprint.to_owned()),
            release_reason: Set(None),
            created_at: Set(stamp),
            updated_at: Set(stamp),
        };
        let row = reservation::Entity::insert(row).exec_with_returning(transaction).await?;
        let mut lines = Vec::with_capacity(picks.len());
        // Dòng lưu theo thứ tự dòng đơn (không theo thứ tự khoá).
        let mut by_need: Vec<&(Uuid, &Pick)> = ordered.iter().collect();
        by_need.sort_by_key(|(_, pick)| pick.need);
        for (level, pick) in by_need {
            let need = &needs[pick.need];
            let line = reservation_line::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                reservation_id: Set(row.id),
                line_key: Set(need.line_key.clone()),
                variant_id: Set(need.variant),
                bundle_variant_id: Set(need.bundle),
                per_unit: Set(need.per_unit),
                level_id: Set(*level),
                org_id: Set(pick.org),
                quantity: Set(pick.quantity),
                fulfilled_quantity: Set(0),
                released_quantity: Set(0),
                backordered: Set(pick.backordered),
                preordered: Set(need.preorder),
                allocated_quantity: Set(0),
            };
            lines.push(
                reservation_line::Entity::insert(line).exec_with_returning(transaction).await?,
            );
        }
        let reservation = present(row, &lines)?;
        self.outbox
            .record(
                transaction,
                tenant,
                Actor::system(),
                subject(reservation.id),
                &events::ReservationHeld(Change::after(snapshot(&reservation))),
            )
            .await?;
        stock::publish(transaction, tenant, &self.outbox, &Actor::system(), touched).await?;
        Ok(reservation)
    }

    /// Giữ → xác nhận (xem `api::Inventory::confirm`).
    pub(crate) async fn confirm(
        &self,
        tenant: &TenantContext,
        order: Uuid,
    ) -> Result<Reservation, Error> {
        let database = tenant.database();
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let row = find_by_order(&transaction, tenant, order, true)
                    .await?
                    .ok_or_else(|| not_found(order))?;
                match status_of(&row)? {
                    ReservationStatus::Held => {}
                    ReservationStatus::Confirmed | ReservationStatus::Fulfilled => {
                        return load(&transaction, tenant, row).await;
                    }
                    ReservationStatus::Released | ReservationStatus::Expired => {
                        return Err(expired(order));
                    }
                }
                if !set_status(&transaction, tenant, &row, ReservationStatus::Confirmed, None)
                    .await?
                {
                    return Err(Error::internal("a locked reservation changed under its lock"));
                }
                let row = find_by_order(&transaction, tenant, order, false)
                    .await?
                    .ok_or_else(|| not_found(order))?;
                let reservation = load(&transaction, tenant, row).await?;
                self.outbox
                    .record(
                        &transaction,
                        tenant,
                        Actor::system(),
                        subject(reservation.id),
                        &events::ReservationConfirmed(Change::after(snapshot(&reservation))),
                    )
                    .await?;
                transaction.commit().await?;
                Ok(reservation)
            })
            .await
    }

    /// Trả phần chưa xuất (xem `api::Inventory::release`). `actor`: người gỡ bằng tay, hay hệ thống.
    pub(crate) async fn release(
        &self,
        tenant: &TenantContext,
        order: Uuid,
        reason: ReleaseReason,
        actor: &Actor,
    ) -> Result<Option<Reservation>, Error> {
        let database = tenant.database();
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let Some(row) = find_by_order(&transaction, tenant, order, true).await? else {
                    return Ok(None);
                };
                let status = status_of(&row)?;
                let open = matches!(status, ReservationStatus::Held | ReservationStatus::Confirmed);
                // Hết hạn chỉ áp cho chỗ còn `held` và đã quá hạn (job và `confirm` đua nhau: bên
                // khoá trước thắng, bên sau thấy trạng thái mới).
                let due = row.expires_at.is_some_and(|at| at <= now());
                if !open
                    || (reason == ReleaseReason::Expired
                        && (status != ReservationStatus::Held || !due))
                {
                    return Ok(Some(load(&transaction, tenant, row).await?));
                }
                let reservation =
                    self.release_locked(&transaction, tenant, row, reason, actor).await?;
                transaction.commit().await?;
                Ok(Some(reservation))
            })
            .await
    }

    async fn release_locked(
        &self,
        transaction: &DatabaseTransaction,
        tenant: &TenantContext,
        row: reservation::Model,
        reason: ReleaseReason,
        actor: &Actor,
    ) -> Result<Reservation, Error> {
        let mut lines = reservation_line::Entity::find()
            .filter(reservation_line::Column::ReservationId.eq(row.id))
            .scoped(tenant)
            .all(transaction)
            .await?;
        lines.sort_by_key(|line| line.level_id);
        let mut touched = Touched::default();
        let mut shipped = false;
        for line in &lines {
            shipped |= line.fulfilled_quantity > 0;
            let left = remaining(line);
            if left <= 0 {
                continue;
            }
            if line.preordered {
                crate::preorders::give_back(transaction, tenant, line.variant_id, left).await?;
            }
            if !stock::move_available(
                transaction,
                tenant,
                &mut touched,
                line.level_id,
                left,
                Bounds::ANY,
            )
            .await?
            {
                return Err(Error::internal("available stock would overflow"));
            }
            reservation_line::Entity::update_many()
                .col_expr(
                    reservation_line::Column::ReleasedQuantity,
                    Expr::value(line.released_quantity + left),
                )
                .filter(reservation_line::Column::Id.eq(line.id))
                .scoped(tenant)
                .exec(transaction)
                .await?;
        }
        let to = if reason == ReleaseReason::Expired {
            ReservationStatus::Expired
        } else if shipped {
            ReservationStatus::Fulfilled
        } else {
            ReservationStatus::Released
        };
        if !set_status(transaction, tenant, &row, to, Some(reason)).await? {
            return Err(Error::internal("a locked reservation changed under its lock"));
        }
        let row = reservation::Entity::find_by_id(row.id)
            .scoped(tenant)
            .one(transaction)
            .await?
            .ok_or_else(|| Error::internal("reservation vanished under its lock"))?;
        let reservation = load(transaction, tenant, row).await?;
        let mut after = snapshot(&reservation);
        after["reason"] = json!(reason.as_str());
        let change = Change::after(after);
        if to == ReservationStatus::Expired {
            self.outbox
                .record(
                    transaction,
                    tenant,
                    *actor,
                    subject(reservation.id),
                    &events::ReservationExpired(change),
                )
                .await?;
        } else {
            self.outbox
                .record(
                    transaction,
                    tenant,
                    *actor,
                    subject(reservation.id),
                    &events::ReservationReleased(change),
                )
                .await?;
        }
        stock::publish(transaction, tenant, &self.outbox, actor, touched).await?;
        Ok(reservation)
    }

    /// Xuất kho theo chỗ đã giữ (xem `api::Inventory::fulfil`).
    pub(crate) async fn fulfil(
        &self,
        tenant: &TenantContext,
        request: &FulfilRequest,
    ) -> Result<Reservation, Error> {
        self.fulfil_with(tenant, request, None).await
    }

    /// [`Self::fulfil`]; `delivery`: xuất theo event giao hàng — dòng tính trong transaction (`delivery::plan`),
    /// được lấy lô quá hạn, chỗ giữ đã xuất hết là xong.
    pub(crate) async fn fulfil_with(
        &self,
        tenant: &TenantContext,
        request: &FulfilRequest,
        delivery: Option<&[crate::delivery::DeliveredLine]>,
    ) -> Result<Reservation, Error> {
        let allow_expired = delivery.is_some();
        let mut problems = Vec::new();
        let key = Reference::checked(&request.fulfilment_key, "fulfilment_key", &mut problems);
        for (index, line) in request.lines.iter().enumerate() {
            check_quantity(
                &mut problems,
                &format!("lines[{index}].quantity"),
                line.quantity,
                false,
            );
        }
        fail_if_any(problems)?;
        let Some(key) = key else { return Err(Error::internal("key checked without a problem")) };
        let fingerprint = match delivery {
            Some(wanted) => reference::fingerprint(&wanted)?,
            None => reference::fingerprint(&request.lines)?,
        };
        // Cờ lô / serial của các biến thể trong chỗ giữ: hỏi `Catalog` TRƯỚC transaction.
        let variant_ids: Vec<Uuid> =
            match find_by_order(tenant.database().read(), tenant, request.order_id, false).await? {
                Some(row) => reservation_line::Entity::find()
                    .filter(reservation_line::Column::ReservationId.eq(row.id))
                    .scoped(tenant)
                    .all(tenant.database().read())
                    .await?
                    .into_iter()
                    .map(|line| line.variant_id)
                    .collect(),
                None => Vec::new(),
            };
        let variants = if variant_ids.is_empty() {
            HashMap::new()
        } else {
            self.variants(tenant, &variant_ids).await?
        };
        let database = tenant.database();
        let (key, fingerprint, variants) = (&key, &fingerprint, &variants);
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let order = request.order_id;
                let row = find_by_order(&transaction, tenant, order, true)
                    .await?
                    .ok_or_else(|| not_found(order))?;
                let done = fulfilment::Entity::find()
                    .filter(fulfilment::Column::ReservationId.eq(row.id))
                    .filter(fulfilment::Column::FulfilmentKey.eq(key.as_str()))
                    .scoped(tenant)
                    .one(&transaction)
                    .await?
                    .map(|done| ((), done.fingerprint));
                if reference::replayed(done, key, fingerprint)?.is_some() {
                    return load(&transaction, tenant, row).await;
                }
                let status = status_of(&row)?;
                // Event giao hàng tới khi chỗ giữ đã xuất hết (tới lại, hay sau lần giao khác): không còn gì để xuất.
                if delivery.is_some() && status == ReservationStatus::Fulfilled {
                    tracing::debug!(order_id = %order, "reservation already fulfilled; nothing to issue");
                    return load(&transaction, tenant, row).await;
                }
                // Nhân viên đã trả tay chỗ giữ (đối soát tay hàng đã giao, xem docs/INVENTORY.md): xong, không xuất.
                if delivery.is_some()
                    && status == ReservationStatus::Released
                    && row.release_reason.as_deref() == Some("manual")
                {
                    // Thường là đối soát tay đã làm (trả chỗ giữ rồi ghi điều chỉnh); nhưng nếu nhân viên trả
                    // chỗ giữ mà quên ghi điều chỉnh thì hàng đã đi mà tồn không giảm — cảnh báo và đếm để
                    // người vận hành kiểm (UT-1 tiếp; cách đối soát ở docs/INVENTORY.md).
                    metrics::counter!("inventory_delivery_after_manual_release_total").increment(1);
                    tracing::warn!(
                        order_id = %order,
                        reservation_id = %row.id,
                        "goods delivered after staff released the reservation; nothing issued, check that the sale was recorded with an adjustment"
                    );
                    return load(&transaction, tenant, row).await;
                }
                // Chỗ giữ HỆ THỐNG tự trả (hết hạn giữ, thanh toán lỗi) mà hàng vẫn tới tay người mua (vd chuyển
                // khoản tới muộn): hàng vật lý đã đi, nên event giao hàng vẫn xuất phần đã trả lại, trừ thẳng tồn
                // (`available` cùng giảm). Không áp khi: nhân viên trả tay (`manual` — đã tự đối soát, xuất nữa là
                // trừ hai lần), đơn huỷ; event cũ không nêu tổng đã giao (không biết phần nào đã huỷ); `fulfil` tay.
                let lapsed = delivery.is_some_and(crate::delivery::knows_totals)
                    && (status == ReservationStatus::Expired
                        || (status == ReservationStatus::Released
                            && matches!(
                                row.release_reason.as_deref(),
                                Some("payment_failed" | "expired")
                            )));
                if !lapsed && !matches!(status, ReservationStatus::Held | ReservationStatus::Confirmed) {
                    return Err(expired(order));
                }
                let lines = reservation_line::Entity::find()
                    .filter(reservation_line::Column::ReservationId.eq(row.id))
                    .scoped(tenant)
                    .order_by_asc(reservation_line::Column::Id)
                    .all(&transaction)
                    .await?;
                let plan = match delivery {
                    Some(wanted) => {
                        let plan = crate::delivery::plan(
                            &transaction,
                            tenant,
                            order,
                            &lines,
                            variants,
                            wanted,
                            lapsed,
                        )
                        .await?;
                        if plan.is_empty() {
                            return load(&transaction, tenant, row).await;
                        }
                        if lapsed {
                            tracing::warn!(
                                order_id = %order,
                                status = status.as_str(),
                                "issuing delivered goods whose reservation had lapsed; stock is taken from available"
                            );
                        }
                        plan
                    }
                    None => crate::fulfil_plan::manual_plan(&lines, request, variants)?,
                };
                let mut ordered: Vec<&crate::fulfil_plan::Take> = plan.iter().collect();
                ordered.sort_by_key(|take| lines[take.index].level_id);
                let mut touched = Touched::default();
                // Một dòng giữ có thể có nhiều phần (`manual_plan` theo từng dòng yêu cầu): cộng dồn.
                let mut fulfilled: HashMap<usize, i64> = HashMap::new();
                for take in ordered {
                    let line = &lines[take.index];
                    let quantity = take.quantity;
                    let wanted =
                        request.lines.iter().find(|wanted| wanted.line_key == line.line_key);
                    let variant = variants.get(&line.variant_id);
                    let tracking = crate::lots::Tracking {
                        lot_tracked: variant.is_some_and(|v| v.lot_tracked),
                        lot_code: wanted.and_then(|wanted| wanted.lot_code.as_deref()),
                        serials: take.serials.as_deref(),
                        serial_status: "sold",
                        order: Some(order),
                        allow_expired,
                    };
                    let new = NewMovement {
                        org: line.org_id,
                        variant: line.variant_id,
                        kind: MovementKind::Sale,
                        quantity: -quantity,
                        touches_available: lapsed,
                        unit_cost: None,
                        lot_id: None,
                        reason_code: None,
                        note: String::new(),
                        source_type: "reservation",
                        source_id: row.id,
                        actor: None,
                    };
                    crate::lots::record_out(&transaction, tenant, &mut touched, new, &tracking)
                        .await?;
                    let done = fulfilled.entry(take.index).or_default();
                    *done += quantity;
                    let mut update = reservation_line::Entity::update_many().col_expr(
                        reservation_line::Column::FulfilledQuantity,
                        Expr::value(line.fulfilled_quantity + *done),
                    );
                    if lapsed {
                        update = update.col_expr(
                            reservation_line::Column::ReleasedQuantity,
                            Expr::value(line.released_quantity - *done),
                        );
                    }
                    update
                        .filter(reservation_line::Column::Id.eq(line.id))
                        .scoped(tenant)
                        .exec(&transaction)
                        .await?;
                }
                fulfilment::Entity::insert(fulfilment::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    reservation_id: Set(row.id),
                    fulfilment_key: Set(key.as_str().to_owned()),
                    fingerprint: Set(fingerprint.clone()),
                    created_at: Set(now()),
                })
                .exec_without_returning(&transaction)
                .await?;
                let finished = lines.iter().enumerate().all(|(index, line)| {
                    remaining(line) - fulfilled.get(&index).copied().unwrap_or(0) == 0
                });
                let to = if finished {
                    ReservationStatus::Fulfilled
                } else {
                    ReservationStatus::Confirmed
                };
                // Chỗ giữ đã hết hạn / đã trả giữ nguyên trạng thái (lịch sử); dòng `sale` vẫn ghi ở trên.
                if !lapsed
                    && to.as_str() != row.status
                    && !set_status(&transaction, tenant, &row, to, None).await?
                {
                    return Err(Error::internal("a locked reservation changed under its lock"));
                }
                let row = reservation::Entity::find_by_id(row.id)
                    .scoped(tenant)
                    .one(&transaction)
                    .await?
                    .ok_or_else(|| not_found(order))?;
                let reservation = load(&transaction, tenant, row).await?;
                if status == ReservationStatus::Held {
                    self.outbox
                        .record(
                            &transaction,
                            tenant,
                            Actor::system(),
                            subject(reservation.id),
                            &events::ReservationConfirmed(Change::after(snapshot(&reservation))),
                        )
                        .await?;
                }
                stock::publish(&transaction, tenant, &self.outbox, &Actor::system(), touched)
                    .await?;
                transaction.commit().await?;
                Ok(reservation)
            })
            .await
    }

    /// Nhập lại hàng trả về (xem `api::Inventory::restock`).
    pub(crate) async fn restock(
        &self,
        tenant: &TenantContext,
        request: &RestockRequest,
    ) -> Result<(), Error> {
        let mut problems = Vec::new();
        let reference = Reference::checked(&request.reference, "reference", &mut problems);
        if request.lines.is_empty() || request.lines.len() > crate::levels::MAX_ADJUSTMENT_LINES {
            problems.push(
                FieldError::new("lines", "length", "must hold 1 to {max} lines")
                    .with_param("max", crate::levels::MAX_ADJUSTMENT_LINES),
            );
        }
        for (index, line) in request.lines.iter().enumerate() {
            check_quantity(
                &mut problems,
                &format!("lines[{index}].quantity"),
                line.quantity,
                false,
            );
        }
        fail_if_any(problems)?;
        let Some(reference) = reference else {
            return Err(Error::internal("reference checked without a problem"));
        };
        let ids: Vec<Uuid> = request.lines.iter().map(|line| line.variant_id).collect();
        let variants = self.variants(tenant, &ids).await?;
        let org = request.warehouse_org_id.as_uuid();
        self.check_stock_unit(tenant, org).await?;
        let fingerprint = reference::fingerprint(request)?;
        let lines_json = serde_json::to_string(&request.lines).map_err(Error::internal)?;
        let database = tenant.database();
        let (reference, fingerprint, lines_json, variants) =
            (&reference, &fingerprint, &lines_json, &variants);
        reference::submit_once(database, move || async move {
            let transaction = database.begin_write().await?;
            let stored = restock::Entity::find()
                .filter(restock::Column::Reference.eq(reference.as_str()))
                .scoped(tenant)
                .one(&transaction)
                .await?
                .map(|row| ((), row.reference_fingerprint));
            if reference::replayed(stored, reference, fingerprint)?.is_some() {
                return Ok(Submitted::Replayed(()));
            }
            let row = restock::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                reference: Set(reference.as_str().to_owned()),
                reference_fingerprint: Set(fingerprint.clone()),
                order_id: Set(request.order_id),
                org_id: Set(org),
                lines: Set(lines_json.clone()),
                created_at: Set(now()),
            };
            let row = restock::Entity::insert(row).exec_with_returning(&transaction).await?;
            let mut touched = Touched::default();
            let mut sellable: Vec<(Uuid, &crate::api::RestockLine)> = Vec::new();
            for line in request.lines.iter().filter(|line| line.sellable) {
                if variants.get(&line.variant_id).is_some_and(|variant| variant.track_inventory) {
                    sellable.push((
                        stock::level_id(&transaction, tenant, org, line.variant_id).await?,
                        line,
                    ));
                }
            }
            sellable.sort_by_key(|(level, _)| *level);
            for (_, line) in sellable {
                let new = NewMovement {
                    org,
                    variant: line.variant_id,
                    kind: MovementKind::Return,
                    quantity: line.quantity,
                    touches_available: true,
                    unit_cost: None,
                    lot_id: None,
                    reason_code: None,
                    note: String::new(),
                    source_type: "restock",
                    source_id: row.id,
                    actor: None,
                };
                let variant = variants.get(&line.variant_id);
                let lot = line
                    .lot_code
                    .as_deref()
                    .filter(|_| variant.is_some_and(|v| v.lot_tracked))
                    .map(|code| (code, None));
                let serials: &[String] =
                    if variant.is_some_and(|v| v.serial_tracked) { &line.serials } else { &[] };
                crate::lots::record_in(&transaction, tenant, &mut touched, new, lot, serials)
                    .await?;
            }
            stock::publish(&transaction, tenant, &self.outbox, &Actor::system(), touched).await?;
            transaction.commit().await?;
            Ok(Submitted::Created(()))
        })
        .await
        .map(|_| ())
    }
}

/// Chọn kho cho mọi phần. Một kho đủ mọi phần thắng (theo thứ tự ứng viên); không có thì chia
/// tham lam khi được chia; không thì `inventory.insufficient_stock` nêu từng dòng thiếu.
fn plan(
    needs: &[Need],
    candidates: &[Uuid],
    allow_split: bool,
    available: &mut HashMap<(Uuid, Uuid), i64>,
) -> Result<Vec<Pick>, Error> {
    let free = |available: &HashMap<(Uuid, Uuid), i64>, org: Uuid, variant: Uuid| {
        available.get(&(org, variant)).copied().unwrap_or(0)
    };
    for org in candidates {
        let mut wanted: BTreeMap<Uuid, i64> = BTreeMap::new();
        for need in needs.iter().filter(|need| !need.negative_ok) {
            *wanted.entry(need.variant).or_default() += need.quantity;
        }
        if wanted.iter().all(|(variant, quantity)| free(available, *org, *variant) >= *quantity) {
            let mut picks = Vec::with_capacity(needs.len());
            for (index, need) in needs.iter().enumerate() {
                let left = free(available, *org, need.variant);
                picks.push(Pick {
                    need: index,
                    org: *org,
                    quantity: need.quantity,
                    backordered: left < need.quantity,
                });
                available.insert((*org, need.variant), left - need.quantity);
            }
            return Ok(picks);
        }
    }
    let mut picks = Vec::new();
    let mut short = Vec::new();
    for (index, need) in needs.iter().enumerate() {
        let mut left = need.quantity;
        if allow_split {
            for org in candidates {
                let free_here = free(available, *org, need.variant).max(0).min(left);
                if free_here > 0 {
                    picks.push(Pick {
                        need: index,
                        org: *org,
                        quantity: free_here,
                        backordered: false,
                    });
                    available.insert(
                        (*org, need.variant),
                        free(available, *org, need.variant) - free_here,
                    );
                    left -= free_here;
                }
                if left == 0 {
                    break;
                }
            }
        }
        if left > 0 && need.negative_ok {
            let org = candidates[0];
            picks.push(Pick { need: index, org, quantity: left, backordered: true });
            available.insert((org, need.variant), free(available, org, need.variant) - left);
            left = 0;
        }
        if left > 0 {
            let total: i64 =
                candidates.iter().map(|org| free(available, *org, need.variant).max(0)).sum();
            short.push(format!("`{}` (available {})", need.line_key, total));
        }
    }
    if short.is_empty() && allow_split {
        return Ok(picks);
    }
    if short.is_empty() {
        // Không chia được nhưng mọi phần "đủ" nhờ cho âm: đã xử lý ở vòng một kho; tới đây là
        // thiếu ở dòng không cho âm.
        let lines: Vec<String> = needs
            .iter()
            .filter(|need| !need.negative_ok)
            .map(|need| {
                let best = candidates
                    .iter()
                    .map(|org| free(available, *org, need.variant))
                    .max()
                    .unwrap_or(0);
                format!("`{}` (available {})", need.line_key, best.max(0))
            })
            .collect();
        return Err(errors::insufficient_stock(&lines.join(", ")));
    }
    Err(errors::insufficient_stock(&short.join(", ")))
}
