//! Dòng tồn, sổ biến động và điều chỉnh tồn có lý do (D-223).

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::reasons::{ReasonDefault, ReasonList};
use autoxide_kernel::reference::{self, Reference, Submitted};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::time::{Date, Timestamp};
use sea_orm::sea_query::{Expr, ExprTrait as _};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder,
};
use serde::Serialize;
use uuid::Uuid;

use crate::entity::{adjustment, level, movement};
use crate::errors;
use crate::permissions::{ADJUST, READ, SETTINGS_MANAGE};
use crate::service::{InventoryService, check_quantity, fail_if_any, now, readable, writable};
use crate::stock::{self, MovementKind, NewMovement, Touched};

/// Danh sách lý do điều chỉnh tồn (D-223); kiểm kê dùng cùng danh sách.
pub(crate) const ADJUSTMENT_REASONS: &str = "inventory.adjustment";

/// Số dòng tối đa của một lệnh điều chỉnh.
pub(crate) const MAX_ADJUSTMENT_LINES: usize = 500;

pub(crate) fn reason_lists() -> Vec<ReasonList> {
    vec![ReasonList::new(
        ADJUSTMENT_REASONS,
        vec![
            ReasonDefault::new("damaged", "Damaged"),
            ReasonDefault::new("lost", "Lost or stolen"),
            ReasonDefault::new("found", "Found"),
            ReasonDefault::new("data_entry_fix", "Data entry correction"),
            ReasonDefault::new("sample_use", "Used as a sample"),
            ReasonDefault::new("expired", "Expired"),
            ReasonDefault::new("count_variance", "Stock count difference"),
        ],
    )]
}

/// Bộ lọc danh sách dòng tồn.
#[derive(Debug, Clone, Default)]
pub(crate) struct LevelFilter {
    pub org_id: Option<Uuid>,
    pub variant_id: Option<Uuid>,
    /// Chỉ dòng có `available ≤ reorder_point`.
    pub below_reorder: bool,
}

/// Bộ lọc sổ biến động.
#[derive(Debug, Clone, Default)]
pub(crate) struct MovementFilter {
    pub org_id: Option<Uuid>,
    pub variant_id: Option<Uuid>,
    pub kind: Option<MovementKind>,
    pub from: Option<Timestamp>,
    pub to: Option<Timestamp>,
}

/// Một dòng của lệnh điều chỉnh.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct AdjustLine {
    pub variant_id: Uuid,
    /// Có dấu, khác 0.
    pub quantity: i64,
    pub reason_code: Option<String>,
    pub note: Option<String>,
    /// Giá vốn đơn vị của hàng thêm vào (vd nhập tồn đầu kỳ); chỉ khi `quantity > 0`.
    pub unit_cost: Option<Money>,
    /// Lô: bắt buộc khi thêm hàng của biến thể theo dõi lô; khi bớt thì trống = lô hết hạn trước.
    pub lot_code: Option<String>,
    /// Hạn dùng của lô khi thêm hàng.
    pub expires_on: Option<Date>,
    /// Đủ `|quantity|` số serial khi biến thể theo dõi serial.
    pub serials: Vec<String>,
}

/// Lệnh điều chỉnh tồn ở một kho.
#[derive(Debug, Clone)]
pub(crate) struct AdjustInput {
    pub org_id: Uuid,
    pub reference: String,
    pub lines: Vec<AdjustLine>,
}

/// Lệnh điều chỉnh đã ghi, kèm các dòng sổ biến động của nó.
#[derive(Debug, Clone)]
pub(crate) struct Adjustment {
    pub row: adjustment::Model,
    pub movements: Vec<movement::Model>,
}

/// Nội dung quyết định kết quả của lệnh điều chỉnh (dấu vân tay, D-184).
#[derive(Serialize)]
struct AdjustFingerprint<'a> {
    org_id: Uuid,
    lines: &'a [AdjustLine],
}

impl InventoryService {
    pub(crate) async fn list_levels(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: &LevelFilter,
        page: &PageRequest,
    ) -> Result<Page<level::Model>, Error> {
        let scope = principal.scope_condition(&READ, level::Column::OrgId, None)?;
        let mut select = level::Entity::find().scoped(tenant).filter(scope);
        if let Some(org) = filter.org_id {
            select = select.filter(level::Column::OrgId.eq(org));
        }
        if let Some(variant) = filter.variant_id {
            select = select.filter(level::Column::VariantId.eq(variant));
        }
        if filter.below_reorder {
            select = select.filter(level::Column::ReorderPoint.is_not_null()).filter(
                Expr::col(level::Column::Available).lte(Expr::col(level::Column::ReorderPoint)),
            );
        }
        fetch_page(select, level::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    pub(crate) async fn get_level(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<level::Model, Error> {
        let read = tenant.database().read();
        let row = level::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| errors::level_not_found(id))?;
        readable(principal, read, &READ, row.org_id, || errors::level_not_found(id)).await?;
        Ok(row)
    }

    /// Đặt điểm đặt hàng lại (`None` = bỏ) của dòng tồn `id`.
    pub(crate) async fn set_reorder_point(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        reorder_point: Option<i64>,
    ) -> Result<level::Model, Error> {
        if reorder_point.is_some_and(|point| !(0..=crate::service::MAX_QUANTITY).contains(&point)) {
            return Err(Error::validation(vec![
                FieldError::new(
                    "reorder_point",
                    "range",
                    "must be a whole number from 0 to {max}, or null",
                )
                .with_param("max", crate::service::MAX_QUANTITY),
            ]));
        }
        let transaction = tenant.database().begin_write().await?;
        let row = level::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| errors::level_not_found(id))?;
        // Ngoài phạm vi: báo như không có (không lộ dòng tồn của kho khác).
        readable(principal, &transaction, &SETTINGS_MANAGE, row.org_id, || {
            errors::level_not_found(id)
        })
        .await?;
        writable(principal, &transaction, &SETTINGS_MANAGE, row.org_id).await?;
        level::Entity::update_many()
            .col_expr(level::Column::ReorderPoint, Expr::value(reorder_point))
            .col_expr(level::Column::UpdatedAt, Expr::value(now()))
            .filter(level::Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let row = level::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| errors::level_not_found(id))?;
        transaction.commit().await?;
        Ok(row)
    }

    pub(crate) async fn list_movements(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: &MovementFilter,
        page: &PageRequest,
    ) -> Result<Page<movement::Model>, Error> {
        let scope = principal.scope_condition(&READ, movement::Column::OrgId, None)?;
        let mut select = movement::Entity::find().scoped(tenant).filter(scope);
        if let Some(org) = filter.org_id {
            select = select.filter(movement::Column::OrgId.eq(org));
        }
        if let Some(variant) = filter.variant_id {
            select = select.filter(movement::Column::VariantId.eq(variant));
        }
        if let Some(kind) = filter.kind {
            select = select.filter(movement::Column::Kind.eq(kind.as_str()));
        }
        if let Some(from) = filter.from {
            select = select.filter(movement::Column::CreatedAt.gte(from));
        }
        if let Some(to) = filter.to {
            select = select.filter(movement::Column::CreatedAt.lt(to));
        }
        fetch_page(select, movement::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    pub(crate) async fn get_adjustment(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Adjustment, Error> {
        let read = tenant.database().read();
        let row = adjustment::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| errors::adjustment_not_found(id))?;
        readable(principal, read, &READ, row.org_id, || errors::adjustment_not_found(id)).await?;
        let movements = movements_of(read, tenant, "adjustment", row.id).await?;
        Ok(Adjustment { row, movements })
    }

    /// Điều chỉnh tồn ở một kho, mỗi dòng một lý do của danh sách `inventory.adjustment`. Chạy
    /// đúng một lần theo `reference` (gọi lại cùng nội dung → `Replayed`).
    pub(crate) async fn adjust(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: AdjustInput,
    ) -> Result<Submitted<Adjustment>, Error> {
        let mut problems = Vec::new();
        let reference = Reference::checked(&input.reference, "reference", &mut problems);
        if input.lines.is_empty() || input.lines.len() > MAX_ADJUSTMENT_LINES {
            problems.push(
                FieldError::new("lines", "length", "must hold 1 to {max} lines")
                    .with_param("max", MAX_ADJUSTMENT_LINES),
            );
        }
        let mut missing_reason = false;
        for (index, line) in input.lines.iter().enumerate() {
            check_quantity(&mut problems, &format!("lines[{index}].quantity"), line.quantity, true);
            if line.reason_code.as_deref().is_none_or(|code| code.trim().is_empty()) {
                missing_reason = true;
                problems.push(FieldError::new(
                    format!("lines[{index}].reason_code"),
                    "required",
                    "give a reason from the list `inventory.adjustment`",
                ));
            }
            if line.note.as_deref().is_some_and(|note| note.chars().count() > 500) {
                problems.push(FieldError::new(
                    format!("lines[{index}].note"),
                    "length",
                    "must be at most 500 characters",
                ));
            }
            if line.unit_cost.is_some() && line.quantity < 0 {
                problems.push(FieldError::new(
                    format!("lines[{index}].unit_cost"),
                    "not_allowed",
                    "only stock added (a positive quantity) has a unit cost",
                ));
            }
            if line.unit_cost.is_some_and(|cost| cost.is_negative()) {
                problems.push(FieldError::new(
                    format!("lines[{index}].unit_cost"),
                    "range",
                    "must not be negative",
                ));
            }
        }
        if missing_reason {
            let mut error = Error::new(
                ErrorKind::Validation,
                "inventory.reason_required",
                "Every stock adjustment needs a reason from the list `inventory.adjustment`.",
            );
            for problem in problems {
                error = error.with_field(problem);
            }
            return Err(error);
        }
        fail_if_any(problems)?;
        let Some(reference) = reference else {
            return Err(Error::internal("reference checked without a problem"));
        };
        let mut codes: Vec<&str> =
            input.lines.iter().filter_map(|line| line.reason_code.as_deref()).collect();
        codes.sort_unstable();
        codes.dedup();
        for code in codes {
            self.reasons.ensure(tenant, ADJUSTMENT_REASONS, code).await?;
        }
        let ids: Vec<Uuid> = input.lines.iter().map(|line| line.variant_id).collect();
        let variants = self.variants(tenant, &ids).await?;
        if let Some(line) = input
            .lines
            .iter()
            .find(|line| variants.get(&line.variant_id).is_some_and(|v| !v.track_inventory))
        {
            return Err(errors::not_tracked(line.variant_id));
        }
        let mut problems = Vec::new();
        for (index, line) in input.lines.iter().enumerate() {
            let Some(variant) = variants.get(&line.variant_id) else { continue };
            if variant.lot_tracked && line.quantity > 0 && line.lot_code.is_none() {
                problems.push(FieldError::new(
                    format!("lines[{index}].lot_code"),
                    "required",
                    "this variant tracks lots: give the lot of the units added",
                ));
            }
            if let Some(code) = &line.lot_code {
                crate::lots::check_lot_code(
                    &mut problems,
                    &format!("lines[{index}].lot_code"),
                    code,
                );
            }
            if variant.serial_tracked {
                crate::lots::check_serials(
                    &mut problems,
                    &format!("lines[{index}].serials"),
                    &line.serials,
                    line.quantity.abs(),
                );
            }
        }
        fail_if_any(problems)?;
        self.ensure_warehouse(tenant, input.org_id).await?;
        let fingerprint = reference::fingerprint(&AdjustFingerprint {
            org_id: input.org_id,
            lines: &input.lines,
        })?;
        let database = tenant.database();
        let (input, reference, fingerprint, variants) =
            (&input, &reference, &fingerprint, &variants);
        reference::submit_once(database, move || async move {
            let transaction = database.begin_write().await?;
            writable(principal, &transaction, &ADJUST, input.org_id).await?;
            let stored = adjustment::Entity::find()
                .filter(adjustment::Column::Reference.eq(reference.as_str()))
                .scoped(tenant)
                .one(&transaction)
                .await?
                .map(|row| {
                    let fingerprint = row.reference_fingerprint.clone();
                    (row, fingerprint)
                });
            if let Some(row) = reference::replayed(stored, reference, fingerprint)? {
                let movements = movements_of(&transaction, tenant, "adjustment", row.id).await?;
                return Ok(Submitted::Replayed(Adjustment { row, movements }));
            }
            let row = adjustment::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                org_id: Set(input.org_id),
                reference: Set(reference.as_str().to_owned()),
                reference_fingerprint: Set(fingerprint.clone()),
                kind: Set("adjustment".to_owned()),
                actor_id: Set(Some(principal.user_id())),
                created_at: Set(now()),
            };
            let row = adjustment::Entity::insert(row).exec_with_returning(&transaction).await?;
            // Khoá dòng tồn theo thứ tự ID tăng dần: hai lệnh cùng chạm A và B không chờ nhau mãi.
            let mut lines = Vec::with_capacity(input.lines.len());
            for line in &input.lines {
                let level =
                    stock::level_id(&transaction, tenant, input.org_id, line.variant_id).await?;
                lines.push((level, line));
            }
            lines.sort_by_key(|(level, _)| *level);
            let mut touched = Touched::default();
            let mut movements = Vec::with_capacity(lines.len());
            for (_, line) in lines {
                let new = NewMovement {
                    org: input.org_id,
                    variant: line.variant_id,
                    kind: MovementKind::Adjustment,
                    quantity: line.quantity,
                    touches_available: true,
                    unit_cost: line.unit_cost,
                    lot_id: None,
                    reason_code: line.reason_code.clone(),
                    note: line.note.clone().unwrap_or_default(),
                    source_type: "adjustment",
                    source_id: row.id,
                    actor: Some(principal.user_id()),
                };
                let variant = variants.get(&line.variant_id);
                let serial_tracked = variant.is_some_and(|v| v.serial_tracked);
                if line.quantity > 0 {
                    let lot = line.lot_code.as_deref().map(|code| (code, line.expires_on));
                    let serials: &[String] = if serial_tracked { &line.serials } else { &[] };
                    movements.push(
                        crate::lots::record_in(
                            &transaction,
                            tenant,
                            &mut touched,
                            new,
                            lot,
                            serials,
                        )
                        .await?,
                    );
                } else {
                    let tracking = crate::lots::Tracking {
                        lot_tracked: variant.is_some_and(|v| v.lot_tracked),
                        lot_code: line.lot_code.as_deref(),
                        serials: serial_tracked.then_some(line.serials.as_slice()),
                        serial_status: "written_off",
                        order: None,
                    };
                    let (rows, _) =
                        crate::lots::record_out(&transaction, tenant, &mut touched, new, &tracking)
                            .await?;
                    movements.extend(rows);
                }
            }
            stock::publish(&transaction, tenant, &self.outbox, &Actor::from(principal), touched)
                .await?;
            transaction.commit().await?;
            Ok(Submitted::Created(Adjustment { row, movements }))
        })
        .await
    }
}

/// Dòng sổ biến động của chứng từ `source_type:source_id`, theo thứ tự ghi.
pub(crate) async fn movements_of<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    source_type: &str,
    source_id: Uuid,
) -> Result<Vec<movement::Model>, Error> {
    Ok(movement::Entity::find()
        .filter(movement::Column::SourceType.eq(source_type))
        .filter(movement::Column::SourceId.eq(source_id))
        .scoped(tenant)
        .order_by_asc(movement::Column::Id)
        .all(connection)
        .await?)
}

/// Giá trị tồn của một kho theo một tiền tệ: Σ `on_hand` × giá vốn bình quân.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Valuation {
    pub org_id: Uuid,
    pub currency: String,
    pub units: i64,
    pub value_minor: i64,
}

impl InventoryService {
    /// Giá trị tồn hiện tại theo kho và tiền tệ, trong phạm vi `inventory.read` của người gọi.
    pub(crate) async fn valuation(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Option<Uuid>,
    ) -> Result<Vec<Valuation>, Error> {
        use sea_orm::QuerySelect;
        let scope = principal.scope_condition(&READ, level::Column::OrgId, None)?;
        let mut select = level::Entity::find()
            .select_only()
            .column(level::Column::OrgId)
            .column(level::Column::AverageCostCurrency)
            // Postgres trả `numeric` cho `SUM(bigint)`: ép về BIGINT cho cả hai database.
            .column_as(Expr::cust("CAST(COALESCE(SUM(on_hand), 0) AS BIGINT)"), "units")
            .column_as(
                Expr::cust("CAST(COALESCE(SUM(on_hand * average_cost_minor), 0) AS BIGINT)"),
                "value_minor",
            )
            .filter(level::Column::AverageCostCurrency.is_not_null())
            .filter(level::Column::OnHand.gt(0))
            .filter(scope)
            .scoped(tenant)
            .group_by(level::Column::OrgId)
            .group_by(level::Column::AverageCostCurrency);
        if let Some(org) = org {
            select = select.filter(level::Column::OrgId.eq(org));
        }
        let rows: Vec<(Uuid, Option<String>, i64, i64)> =
            select.into_tuple().all(tenant.database().read()).await?;
        let mut found: Vec<Valuation> = rows
            .into_iter()
            .filter_map(|(org_id, currency, units, value_minor)| {
                currency.map(|currency| Valuation { org_id, currency, units, value_minor })
            })
            .collect();
        found.sort_by(|a, b| (a.org_id, &a.currency).cmp(&(b.org_id, &b.currency)));
        Ok(found)
    }
}
