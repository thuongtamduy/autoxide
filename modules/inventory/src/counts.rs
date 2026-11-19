//! Kiểm kê (bước 42 mục 3): mở phiếu ở một kho → nhập số đếm từng biến thể (REST hay CSV) → áp
//! dụng: chênh lệch thành dòng sổ `count` có lý do (danh sách `inventory.adjustment`, D-223).
//!
//! Kiểm kê khi đang bán: lúc nhập số đếm của một dòng, lưu `on_hand` lúc đó (`expected_at_count`);
//! khi áp dụng, chênh lệch = `counted − expected_at_count` cộng vào `on_hand` HIỆN TẠI — hàng bán xen
//! giữa lúc đếm và lúc áp dụng không bị tính lại, và không bao giờ ghi đè `on_hand` bằng số đếm.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, QueryFilter,
    QueryOrder,
};
use serde_json::json;
use uuid::Uuid;

use crate::entity::{count, count_line, level};
use crate::errors;
use crate::events::{self, Change};
use crate::levels::ADJUSTMENT_REASONS;
use crate::permissions::{ADJUST, COUNT, READ};
use crate::service::{InventoryService, MAX_QUANTITY, fail_if_any, now, readable, writable};
use crate::stock::{self, MovementKind, NewMovement, Touched};
use crate::transfers::{document_number, invalid_transition};

/// Số dòng tối đa mỗi lần nhập số đếm (REST hay CSV).
pub(crate) const MAX_LINES: usize = 5000;

/// Lý do mặc định của chênh lệch kiểm kê.
const DEFAULT_REASON: &str = "count_variance";

mod status {
    pub(crate) const OPEN: &str = "open";
    pub(crate) const APPLIED: &str = "applied";
    pub(crate) const CANCELLED: &str = "cancelled";
}

/// Một số đếm.
#[derive(Debug, Clone)]
pub(crate) struct CountedLine {
    pub variant_id: Uuid,
    pub counted: i64,
    pub reason_code: Option<String>,
}

/// Phiếu kiểm kê kèm dòng.
#[derive(Debug, Clone)]
pub(crate) struct Count {
    pub row: count::Model,
    pub lines: Vec<count_line::Model>,
}

fn not_found(id: Uuid) -> Error {
    Error::not_found("inventory.count_not_found", "There is no stock count `{id}` in your scope.")
        .with_param("id", id)
}

async fn load<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    row: count::Model,
) -> Result<Count, Error> {
    let lines = count_line::Entity::find()
        .filter(count_line::Column::CountId.eq(row.id))
        .scoped(tenant)
        .order_by_asc(count_line::Column::Id)
        .all(connection)
        .await?;
    Ok(Count { row, lines })
}

impl InventoryService {
    pub(crate) async fn open_count(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Uuid,
        note: Option<String>,
    ) -> Result<Count, Error> {
        if note.as_deref().is_some_and(|note| note.chars().count() > 500) {
            return Err(Error::validation(vec![FieldError::new(
                "note",
                "length",
                "must be at most 500 characters",
            )]));
        }
        self.check_stock_unit(tenant, org).await?;
        let transaction = tenant.database().begin_write().await?;
        writable(principal, &transaction, &COUNT, org).await?;
        let stamp = now();
        let row = count::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            number: Set(document_number(&transaction, tenant, "KK").await?),
            org_id: Set(org),
            status: Set(status::OPEN.to_owned()),
            note: Set(note.unwrap_or_default()),
            applied_at: Set(None),
            created_by: Set(Some(principal.user_id())),
            created_at: Set(stamp),
            updated_at: Set(stamp),
        };
        let row = count::Entity::insert(row).exec_with_returning(&transaction).await?;
        transaction.commit().await?;
        Ok(Count { row, lines: Vec::new() })
    }

    pub(crate) async fn get_count(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Count, Error> {
        let read = tenant.database().read();
        let row = count::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| not_found(id))?;
        if readable(principal, read, &READ, row.org_id, || not_found(id)).await.is_err() {
            readable(principal, read, &COUNT, row.org_id, || not_found(id)).await?;
        }
        load(read, tenant, row).await
    }

    pub(crate) async fn list_counts(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Option<Uuid>,
        status: Option<&str>,
        page: &PageRequest,
    ) -> Result<Page<Count>, Error> {
        let scope = principal.scope_condition(&COUNT, count::Column::OrgId, None)?;
        let mut select = count::Entity::find().scoped(tenant).filter(scope);
        if let Some(org) = org {
            select = select.filter(count::Column::OrgId.eq(org));
        }
        if let Some(status) = status {
            select = select.filter(count::Column::Status.eq(status));
        }
        let read = tenant.database().read();
        let found = fetch_page(select, count::Column::Id, |row| row.id, page, read).await?;
        let mut data = Vec::with_capacity(found.data.len());
        for row in found.data {
            data.push(load(read, tenant, row).await?);
        }
        Ok(Page { data, pagination: found.pagination })
    }

    async fn lock_count(
        &self,
        transaction: &DatabaseTransaction,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        permission: &autoxide_kernel::access::Permission,
    ) -> Result<count::Model, Error> {
        let row = rows::lock_one(transaction, count::Entity::find_by_id(id).scoped(tenant))
            .await?
            .ok_or_else(|| not_found(id))?;
        readable(principal, transaction, &COUNT, row.org_id, || not_found(id)).await?;
        writable(principal, transaction, permission, row.org_id).await?;
        Ok(row)
    }

    /// Nhập (hay sửa) số đếm của các biến thể; mỗi dòng lưu `on_hand` lúc này làm số kỳ vọng.
    pub(crate) async fn record_counts(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        lines: Vec<CountedLine>,
    ) -> Result<Count, Error> {
        let mut problems = Vec::new();
        if lines.is_empty() || lines.len() > MAX_LINES {
            problems.push(
                FieldError::new("lines", "length", "must hold 1 to {max} lines")
                    .with_param("max", MAX_LINES),
            );
        }
        for (index, line) in lines.iter().enumerate() {
            if !(0..=MAX_QUANTITY).contains(&line.counted) {
                problems.push(
                    FieldError::new(
                        format!("lines[{index}].counted"),
                        "range",
                        "must be a whole number from 0 to {max}",
                    )
                    .with_param("max", MAX_QUANTITY),
                );
            }
        }
        fail_if_any(problems)?;
        let mut codes: Vec<&str> =
            lines.iter().filter_map(|line| line.reason_code.as_deref()).collect();
        codes.sort_unstable();
        codes.dedup();
        for code in codes {
            self.reasons.ensure(tenant, ADJUSTMENT_REASONS, code).await?;
        }
        let ids: Vec<Uuid> = lines.iter().map(|line| line.variant_id).collect();
        let variants = self.variants(tenant, &ids).await?;
        if let Some(line) = lines
            .iter()
            .find(|line| variants.get(&line.variant_id).is_some_and(|v| !v.track_inventory))
        {
            return Err(errors::not_tracked(line.variant_id));
        }
        let database = tenant.database();
        let lines = &lines;
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let row = self.lock_count(&transaction, tenant, principal, id, &COUNT).await?;
                if row.status != status::OPEN {
                    return Err(invalid_transition(&row.status, status::OPEN));
                }
                let stamp = now();
                for line in lines {
                    let expected = level::Entity::find()
                        .filter(level::Column::OrgId.eq(row.org_id))
                        .filter(level::Column::VariantId.eq(line.variant_id))
                        .scoped(tenant)
                        .one(&transaction)
                        .await?
                        .map_or(0, |level| level.on_hand);
                    count_line::Entity::insert(count_line::ActiveModel {
                        id: Set(Uuid::now_v7()),
                        tenant_id: Set(tenant.id().as_uuid()),
                        count_id: Set(row.id),
                        variant_id: Set(line.variant_id),
                        counted: Set(line.counted),
                        expected_at_count: Set(expected),
                        counted_at: Set(stamp),
                        reason_code: Set(line.reason_code.clone()),
                    })
                    .on_conflict(
                        OnConflict::columns([
                            count_line::Column::TenantId,
                            count_line::Column::CountId,
                            count_line::Column::VariantId,
                        ])
                        .update_columns([
                            count_line::Column::Counted,
                            count_line::Column::ExpectedAtCount,
                            count_line::Column::CountedAt,
                            count_line::Column::ReasonCode,
                        ])
                        .to_owned(),
                    )
                    .exec_without_returning(&transaction)
                    .await?;
                }
                count::Entity::update_many()
                    .col_expr(count::Column::UpdatedAt, Expr::value(stamp))
                    .filter(count::Column::Id.eq(id))
                    .scoped(tenant)
                    .exec(&transaction)
                    .await?;
                let count = load(&transaction, tenant, row).await?;
                transaction.commit().await?;
                Ok(count)
            })
            .await
    }

    /// Nhập số đếm từ CSV: cột `sku` (hay mã vạch), `counted`, `reason_code` (tuỳ chọn); dòng đầu là
    /// tiêu đề. Lỗi gom theo dòng, báo một lần.
    pub(crate) async fn import_counts(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        csv: &str,
    ) -> Result<Count, Error> {
        let rows = autoxide_kernel::csv::read(csv, MAX_LINES + 1).map_err(|error| {
            Error::validation(vec![
                FieldError::new("csv", "format", "is not a valid CSV file: {problem}")
                    .with_param("problem", error.to_string()),
            ])
        })?;
        let Some((header, body)) = rows.split_first() else {
            return Err(Error::validation(vec![FieldError::new("csv", "required", "has no rows")]));
        };
        let column =
            |name: &str| header.iter().position(|cell| cell.trim().eq_ignore_ascii_case(name));
        let (Some(code_at), Some(counted_at)) = (column("sku"), column("counted")) else {
            return Err(Error::validation(vec![FieldError::new(
                "csv",
                "header",
                "needs the columns `sku` and `counted` (and optionally `reason_code`)",
            )]));
        };
        let reason_at = column("reason_code");
        let mut problems = Vec::new();
        let mut lines = Vec::with_capacity(body.len());
        for (index, row) in body.iter().enumerate() {
            let line_number = index + 2;
            let cell = |at: usize| row.get(at).map(|cell| cell.trim()).unwrap_or_default();
            let Ok(counted) = cell(counted_at).parse::<i64>() else {
                problems.push(FieldError::new(
                    format!("csv[{line_number}].counted"),
                    "format",
                    "must be a whole number",
                ));
                continue;
            };
            match self.catalog.variant_by_code(tenant, cell(code_at), None).await? {
                Some(variant) => lines.push(CountedLine {
                    variant_id: variant.variant_id,
                    counted,
                    reason_code: reason_at
                        .map(|at| cell(at).to_owned())
                        .filter(|code| !code.is_empty()),
                }),
                None => problems.push(FieldError::new(
                    format!("csv[{line_number}].sku"),
                    "not_found",
                    "no variant has this SKU or barcode",
                )),
            }
        }
        fail_if_any(problems)?;
        self.record_counts(tenant, principal, id, lines).await
    }

    /// Áp dụng phiếu: mỗi dòng lệch thành một dòng sổ `count` (lý do của dòng, mặc định
    /// `count_variance`) cộng vào `on_hand` hiện tại. Cần `inventory.adjust` ở kho.
    pub(crate) async fn apply_count(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Count, Error> {
        self.reasons.ensure(tenant, ADJUSTMENT_REASONS, DEFAULT_REASON).await?;
        let database = tenant.database();
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let row = self.lock_count(&transaction, tenant, principal, id, &ADJUST).await?;
                if row.status != status::OPEN {
                    return Err(invalid_transition(&row.status, status::APPLIED));
                }
                let lines = load(&transaction, tenant, row.clone()).await?.lines;
                let mut keyed = Vec::new();
                for line in &lines {
                    let difference = line.counted - line.expected_at_count;
                    if difference != 0 {
                        keyed.push((
                            stock::level_id(&transaction, tenant, row.org_id, line.variant_id)
                                .await?,
                            line,
                            difference,
                        ));
                    }
                }
                keyed.sort_by_key(|(level, _, _)| *level);
                let mut touched = Touched::default();
                for (_, line, difference) in &keyed {
                    stock::record(
                        &transaction,
                        tenant,
                        &mut touched,
                        NewMovement {
                            org: row.org_id,
                            variant: line.variant_id,
                            kind: MovementKind::Count,
                            quantity: *difference,
                            touches_available: true,
                            unit_cost: None,
                            lot_id: None,
                            reason_code: Some(
                                line.reason_code
                                    .clone()
                                    .unwrap_or_else(|| DEFAULT_REASON.to_owned()),
                            ),
                            note: row.number.clone(),
                            source_type: "count",
                            source_id: row.id,
                            actor: Some(principal.user_id()),
                        },
                    )
                    .await?;
                }
                let stamp = now();
                count::Entity::update_many()
                    .col_expr(count::Column::Status, Expr::value(status::APPLIED))
                    .col_expr(count::Column::AppliedAt, Expr::value(Some(stamp)))
                    .col_expr(count::Column::UpdatedAt, Expr::value(stamp))
                    .filter(count::Column::Id.eq(id))
                    .scoped(tenant)
                    .exec(&transaction)
                    .await?;
                let row = count::Entity::find_by_id(id)
                    .scoped(tenant)
                    .one(&transaction)
                    .await?
                    .ok_or_else(|| not_found(id))?;
                let count = load(&transaction, tenant, row).await?;
                let actor = Actor::from(principal);
                let after = json!({
                    "id": count.row.id,
                    "number": count.row.number,
                    "org_id": count.row.org_id,
                    "differences": keyed.iter().map(|(_, line, difference)| json!({
                        "variant_id": line.variant_id,
                        "difference": difference,
                    })).collect::<Vec<_>>(),
                });
                self.outbox
                    .record(
                        &transaction,
                        tenant,
                        actor,
                        Some(format!("inventory.count:{id}")),
                        &events::CountApplied(Change::after(after)),
                    )
                    .await?;
                stock::publish(&transaction, tenant, &self.outbox, &actor, touched).await?;
                transaction.commit().await?;
                Ok(count)
            })
            .await
    }

    pub(crate) async fn cancel_count(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Count, Error> {
        let transaction = tenant.database().begin_write().await?;
        let row = self.lock_count(&transaction, tenant, principal, id, &COUNT).await?;
        if row.status != status::OPEN {
            return Err(invalid_transition(&row.status, status::CANCELLED));
        }
        count::Entity::update_many()
            .col_expr(count::Column::Status, Expr::value(status::CANCELLED))
            .col_expr(count::Column::UpdatedAt, Expr::value(now()))
            .filter(count::Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let row = count::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| not_found(id))?;
        let count = load(&transaction, tenant, row).await?;
        transaction.commit().await?;
        Ok(count)
    }
}
