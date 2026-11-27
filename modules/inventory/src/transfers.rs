//! Chuyển kho (bước 42 mục 3): nháp → đang đi đường (`ship`: trừ kho đi, cộng `incoming` kho đến)
//! → nhận một phần / đủ (`receive`: cộng kho đến theo giá vốn lúc gửi) → đóng (`close`: phần chưa
//! tới coi như mất trên đường) hay huỷ (chỉ khi còn nháp). Mọi chuyển trạng thái khoá dòng phiếu
//! trước (`rows::lock_one`), nên gửi / nhận hai lần cùng lúc chỉ một lần có hiệu lực.

use autoxide_kernel::time::Date;
use std::collections::BTreeMap;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::Money;
use autoxide_kernel::numbering;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::rows::{self, Bounds};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait, DatabaseTransaction, EntityTrait,
    QueryFilter, QueryOrder,
};
use serde_json::json;
use uuid::Uuid;

use crate::entity::{level, transfer, transfer_line};
use crate::errors;
use crate::events::{self, Change};
use crate::permissions::{READ, TRANSFER};
use crate::service::{
    InventoryService, TRANSFERS_FEATURE, check_quantity, fail_if_any, now, readable, writable,
};
use crate::stock::{self, MovementKind, NewMovement, Touched};

/// Số dòng tối đa của một phiếu.
pub(crate) const MAX_LINES: usize = 500;

/// Trạng thái của phiếu chuyển kho.
pub(crate) mod status {
    pub(crate) const DRAFT: &str = "draft";
    pub(crate) const IN_TRANSIT: &str = "in_transit";
    pub(crate) const PARTIALLY_RECEIVED: &str = "partially_received";
    pub(crate) const RECEIVED: &str = "received";
    pub(crate) const CLOSED: &str = "closed";
    pub(crate) const CANCELLED: &str = "cancelled";
}

/// Một dòng của phiếu mới.
#[derive(Debug, Clone)]
pub(crate) struct TransferLineInput {
    pub variant_id: Uuid,
    pub quantity: i64,
    /// Bắt buộc, đủ `quantity` số, khi biến thể theo dõi serial.
    pub serials: Vec<String>,
}

/// Phiếu chuyển mới.
#[derive(Debug, Clone)]
pub(crate) struct NewTransfer {
    pub from_org_id: Uuid,
    pub to_org_id: Uuid,
    pub reference: Option<String>,
    pub note: Option<String>,
    pub lines: Vec<TransferLineInput>,
}

/// Phiếu kèm dòng.
#[derive(Debug, Clone)]
pub(crate) struct Transfer {
    pub row: transfer::Model,
    pub lines: Vec<transfer_line::Model>,
}

/// Một dòng nhận: biến thể và số nhận lần này.
#[derive(Debug, Clone)]
pub(crate) struct ReceiveLine {
    pub variant_id: Uuid,
    pub quantity: i64,
}

fn not_found(id: Uuid) -> Error {
    Error::not_found(
        "inventory.transfer_not_found",
        "There is no stock transfer `{id}` in your scope.",
    )
    .with_param("id", id)
}

fn over_receipt() -> Error {
    Error::conflict(
        "inventory.over_receipt",
        "More units would be received than were ordered or are in transit.",
    )
}

/// Lỗi `inventory.invalid_transition` cho phiếu (chuyển kho, kiểm kê, đơn đặt hàng…).
pub(crate) fn invalid_transition(from: &str, to: &str) -> Error {
    Error::conflict("inventory.invalid_transition", "A document cannot go from `{from}` to `{to}`.")
        .with_param("from", from.to_owned())
        .with_param("to", to.to_owned())
}

fn subject(id: Uuid) -> Option<String> {
    Some(format!("inventory.transfer:{id}"))
}

fn snapshot(transfer: &Transfer) -> serde_json::Value {
    json!({
        "id": transfer.row.id,
        "number": transfer.row.number,
        "from_org_id": transfer.row.from_org_id,
        "to_org_id": transfer.row.to_org_id,
        "status": transfer.row.status,
        "lines": transfer.lines.iter().map(|line| json!({
            "variant_id": line.variant_id,
            "quantity": line.quantity,
            "received_quantity": line.received_quantity,
        })).collect::<Vec<_>>(),
    })
}

async fn load<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    row: transfer::Model,
) -> Result<Transfer, Error> {
    let lines = transfer_line::Entity::find()
        .filter(transfer_line::Column::TransferId.eq(row.id))
        .scoped(tenant)
        .order_by_asc(transfer_line::Column::Id)
        .all(connection)
        .await?;
    Ok(Transfer { row, lines })
}

async fn set_status(
    transaction: &DatabaseTransaction,
    tenant: &TenantContext,
    id: Uuid,
    to: &str,
    shipped: bool,
) -> Result<(), Error> {
    let mut update = transfer::Entity::update_many()
        .col_expr(transfer::Column::Status, Expr::value(to))
        .col_expr(transfer::Column::UpdatedAt, Expr::value(now()));
    if shipped {
        update = update.col_expr(transfer::Column::ShippedAt, Expr::value(Some(now())));
    }
    update.filter(transfer::Column::Id.eq(id)).scoped(tenant).exec(transaction).await?;
    Ok(())
}

/// Cấp số chứng từ `<tiền tố>-<năm>-<số 6 chữ số>` theo khách và năm, trong transaction của chứng từ.
pub(crate) async fn document_number<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    prefix: &str,
) -> Result<String, Error> {
    let year = now().date_utc().year();
    let next =
        numbering::next(transaction, tenant.id(), &format!("inventory.{prefix}:{year}")).await?;
    Ok(format!("{prefix}-{year}-{next:06}"))
}

impl InventoryService {
    pub(crate) async fn create_transfer(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewTransfer,
    ) -> Result<Transfer, Error> {
        let mut problems = Vec::new();
        if input.from_org_id == input.to_org_id {
            problems.push(FieldError::new("to_org_id", "same", "must differ from `from_org_id`"));
        }
        if input.lines.is_empty() || input.lines.len() > MAX_LINES {
            problems.push(
                FieldError::new("lines", "length", "must hold 1 to {max} lines")
                    .with_param("max", MAX_LINES),
            );
        }
        let mut seen = std::collections::BTreeSet::new();
        for (index, line) in input.lines.iter().enumerate() {
            check_quantity(
                &mut problems,
                &format!("lines[{index}].quantity"),
                line.quantity,
                false,
            );
            if !seen.insert(line.variant_id) {
                problems.push(FieldError::new(
                    format!("lines[{index}].variant_id"),
                    "duplicate",
                    "lists the same variant twice",
                ));
            }
        }
        if input.reference.as_deref().is_some_and(|text| text.chars().count() > 100) {
            problems.push(FieldError::new("reference", "length", "must be at most 100 characters"));
        }
        if input.note.as_deref().is_some_and(|note| note.chars().count() > 500) {
            problems.push(FieldError::new("note", "length", "must be at most 500 characters"));
        }
        fail_if_any(problems)?;
        self.require_feature(tenant, &TRANSFERS_FEATURE).await?;
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
            if variants.get(&line.variant_id).is_some_and(|v| v.serial_tracked) {
                crate::lots::check_serials(
                    &mut problems,
                    &format!("lines[{index}].serials"),
                    &line.serials,
                    line.quantity,
                );
            }
        }
        fail_if_any(problems)?;
        self.check_stock_unit(tenant, input.from_org_id).await?;
        self.ensure_warehouse(tenant, input.to_org_id).await?;
        let transaction = tenant.database().begin_write().await?;
        writable(principal, &transaction, &TRANSFER, input.from_org_id).await?;
        let stamp = now();
        let row = transfer::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            number: Set(document_number(&transaction, tenant, "CK").await?),
            from_org_id: Set(input.from_org_id),
            to_org_id: Set(input.to_org_id),
            status: Set(status::DRAFT.to_owned()),
            shipped_at: Set(None),
            reference: Set(input.reference),
            note: Set(input.note.unwrap_or_default()),
            created_by: Set(Some(principal.user_id())),
            created_at: Set(stamp),
            updated_at: Set(stamp),
        };
        let row = transfer::Entity::insert(row).exec_with_returning(&transaction).await?;
        for line in &input.lines {
            transfer_line::Entity::insert(transfer_line::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                transfer_id: Set(row.id),
                variant_id: Set(line.variant_id),
                quantity: Set(line.quantity),
                received_quantity: Set(0),
                unit_cost_minor: Set(None),
                unit_cost_currency: Set(None),
                lots: Set("[]".to_owned()),
                serials: Set(serde_json::to_string(&line.serials).map_err(Error::internal)?),
            })
            .exec_without_returning(&transaction)
            .await?;
        }
        let transfer = load(&transaction, tenant, row).await?;
        transaction.commit().await?;
        Ok(transfer)
    }

    pub(crate) async fn get_transfer(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Transfer, Error> {
        let read = tenant.database().read();
        let row = transfer::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| not_found(id))?;
        let from = readable(principal, read, &READ, row.from_org_id, || not_found(id)).await;
        if from.is_err() {
            readable(principal, read, &READ, row.to_org_id, || not_found(id)).await?;
        }
        load(read, tenant, row).await
    }

    pub(crate) async fn list_transfers(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        status: Option<&str>,
        org: Option<Uuid>,
        page: &PageRequest,
    ) -> Result<Page<Transfer>, Error> {
        let from = principal.scope_condition(&READ, transfer::Column::FromOrgId, None)?;
        let to = principal.scope_condition(&READ, transfer::Column::ToOrgId, None)?;
        let mut select =
            transfer::Entity::find().scoped(tenant).filter(Condition::any().add(from).add(to));
        if let Some(status) = status {
            select = select.filter(transfer::Column::Status.eq(status));
        }
        if let Some(org) = org {
            select = select.filter(
                Condition::any()
                    .add(transfer::Column::FromOrgId.eq(org))
                    .add(transfer::Column::ToOrgId.eq(org)),
            );
        }
        let read = tenant.database().read();
        let found = fetch_page(select, transfer::Column::Id, |row| row.id, page, read).await?;
        let mut data = Vec::with_capacity(found.data.len());
        for row in found.data {
            data.push(load(read, tenant, row).await?);
        }
        Ok(Page { data, pagination: found.pagination })
    }

    /// Khoá phiếu `id` trong transaction và kiểm quyền `TRANSFER` ở nút `side` của nó.
    async fn lock_transfer(
        &self,
        transaction: &DatabaseTransaction,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        destination: bool,
    ) -> Result<transfer::Model, Error> {
        let row = rows::lock_one(transaction, transfer::Entity::find_by_id(id).scoped(tenant))
            .await?
            .ok_or_else(|| not_found(id))?;
        let org = if destination { row.to_org_id } else { row.from_org_id };
        let other = if destination { row.from_org_id } else { row.to_org_id };
        if readable(principal, transaction, &TRANSFER, org, || not_found(id)).await.is_err() {
            // Người ở đầu kia thấy phiếu (403) nhưng không làm được việc của đầu này.
            readable(principal, transaction, &READ, other, || not_found(id)).await?;
        }
        writable(principal, transaction, &TRANSFER, org).await?;
        Ok(row)
    }

    /// Gửi phiếu nháp: trừ kho đi (`transfer_out`, không đụng hàng đã giữ cho đơn), ghi giá vốn
    /// lúc gửi vào dòng, cộng `incoming` ở kho đến.
    pub(crate) async fn ship_transfer(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Transfer, Error> {
        let variants = self.transfer_variants(tenant, id).await?;
        let variants = &variants;
        let database = tenant.database();
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let row = self.lock_transfer(&transaction, tenant, principal, id, false).await?;
                if row.status != status::DRAFT {
                    return Err(invalid_transition(&row.status, status::IN_TRANSIT));
                }
                let mut lines = load(&transaction, tenant, row.clone()).await?.lines;
                let mut keyed = Vec::with_capacity(lines.len());
                for line in &lines {
                    let from =
                        stock::level_id(&transaction, tenant, row.from_org_id, line.variant_id)
                            .await?;
                    let to = stock::level_id(&transaction, tenant, row.to_org_id, line.variant_id)
                        .await?;
                    keyed.push((from, to, line.id));
                }
                keyed.sort();
                let mut touched = Touched::default();
                let mut short = Vec::new();
                for (from, to, line_id) in &keyed {
                    let Some(line) = lines.iter_mut().find(|line| line.id == *line_id) else {
                        continue;
                    };
                    if !stock::move_available(
                        &transaction,
                        tenant,
                        &mut touched,
                        *from,
                        -line.quantity,
                        Bounds::NON_NEGATIVE,
                    )
                    .await?
                    {
                        short.push(format!("variant `{}`", line.variant_id));
                        continue;
                    }
                    let cost = level::Entity::find_by_id(*from)
                        .scoped(tenant)
                        .one(&transaction)
                        .await?
                        .and_then(|level| {
                            level.average_cost_currency.as_deref().and_then(|currency| {
                                Money::from_columns(level.average_cost_minor, currency).ok()
                            })
                        });
                    let flags = variants.get(&line.variant_id);
                    let serial_list: Vec<String> =
                        serde_json::from_str(&line.serials).unwrap_or_default();
                    let tracking = crate::lots::Tracking {
                        lot_tracked: flags.is_some_and(|v| v.lot_tracked),
                        lot_code: None,
                        serials: flags
                            .is_some_and(|v| v.serial_tracked)
                            .then_some(serial_list.as_slice()),
                        serial_status: "in_transit",
                        order: None,
                        allow_expired: false,
                    };
                    let (_, picks) = crate::lots::record_out(
                        &transaction,
                        tenant,
                        &mut touched,
                        NewMovement {
                            org: row.from_org_id,
                            variant: line.variant_id,
                            kind: MovementKind::TransferOut,
                            quantity: -line.quantity,
                            touches_available: false,
                            unit_cost: None,
                            lot_id: None,
                            reason_code: None,
                            note: row.number.clone(),
                            source_type: "transfer",
                            source_id: row.id,
                            actor: Some(principal.user_id()),
                        },
                        &tracking,
                    )
                    .await?;
                    let lots_json = pieces_json(&transaction, tenant, &picks).await?;
                    stock::move_incoming(&transaction, tenant, *to, line.quantity).await?;
                    transfer_line::Entity::update_many()
                        .col_expr(transfer_line::Column::Lots, Expr::value(lots_json))
                        .col_expr(
                            transfer_line::Column::UnitCostMinor,
                            Expr::value(cost.map(Money::minor)),
                        )
                        .col_expr(
                            transfer_line::Column::UnitCostCurrency,
                            Expr::value(cost.map(|cost| cost.currency().code().to_owned())),
                        )
                        .filter(transfer_line::Column::Id.eq(line.id))
                        .scoped(tenant)
                        .exec(&transaction)
                        .await?;
                }
                if !short.is_empty() {
                    return Err(errors::insufficient_stock(&short.join(", ")));
                }
                set_status(&transaction, tenant, row.id, status::IN_TRANSIT, true).await?;
                let row = transfer::Entity::find_by_id(id)
                    .scoped(tenant)
                    .one(&transaction)
                    .await?
                    .ok_or_else(|| not_found(id))?;
                let transfer = load(&transaction, tenant, row).await?;
                let actor = Actor::from(principal);
                self.outbox
                    .record(
                        &transaction,
                        tenant,
                        actor,
                        subject(id),
                        &events::TransferShipped(Change::after(snapshot(&transfer))),
                    )
                    .await?;
                stock::publish(&transaction, tenant, &self.outbox, &actor, touched).await?;
                transaction.commit().await?;
                Ok(transfer)
            })
            .await
    }

    /// Nhận hàng của phiếu đang đi đường (một phần được). `lines` trống: nhận hết phần còn lại.
    pub(crate) async fn receive_transfer(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        lines: Vec<ReceiveLine>,
    ) -> Result<Transfer, Error> {
        let mut problems = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            check_quantity(
                &mut problems,
                &format!("lines[{index}].quantity"),
                line.quantity,
                false,
            );
        }
        fail_if_any(problems)?;
        let database = tenant.database();
        let lines = &lines;
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let row = self.lock_transfer(&transaction, tenant, principal, id, true).await?;
                if row.status != status::IN_TRANSIT && row.status != status::PARTIALLY_RECEIVED {
                    return Err(invalid_transition(&row.status, status::RECEIVED));
                }
                let current = load(&transaction, tenant, row.clone()).await?.lines;
                // Số nhận lần này theo dòng phiếu.
                let mut wanted: BTreeMap<Uuid, i64> = BTreeMap::new();
                if lines.is_empty() {
                    for line in &current {
                        wanted.insert(line.id, line.quantity - line.received_quantity);
                    }
                } else {
                    for input in lines {
                        let line = current
                            .iter()
                            .find(|line| line.variant_id == input.variant_id)
                            .ok_or_else(over_receipt)?;
                        *wanted.entry(line.id).or_default() += input.quantity;
                    }
                }
                let mut keyed = Vec::new();
                for line in &current {
                    let quantity = wanted.get(&line.id).copied().unwrap_or(0);
                    if quantity <= 0 {
                        continue;
                    }
                    if quantity > line.quantity - line.received_quantity {
                        return Err(over_receipt());
                    }
                    let to = stock::level_id(&transaction, tenant, row.to_org_id, line.variant_id)
                        .await?;
                    keyed.push((to, line, quantity));
                }
                keyed.sort_by_key(|(level, _, _)| *level);
                let mut touched = Touched::default();
                for (to, line, quantity) in &keyed {
                    let cost = match (line.unit_cost_minor, line.unit_cost_currency.as_deref()) {
                        (Some(minor), Some(currency)) => {
                            Some(Money::from_columns(minor, currency)?)
                        }
                        _ => None,
                    };
                    stock::move_incoming(&transaction, tenant, *to, -quantity).await?;
                    let base = NewMovement {
                        org: row.to_org_id,
                        variant: line.variant_id,
                        kind: MovementKind::TransferIn,
                        quantity: *quantity,
                        touches_available: true,
                        unit_cost: cost,
                        lot_id: None,
                        reason_code: None,
                        note: row.number.clone(),
                        source_type: "transfer",
                        source_id: row.id,
                        actor: Some(principal.user_id()),
                    };
                    let pieces: Vec<LotPiece> =
                        serde_json::from_str(&line.lots).unwrap_or_default();
                    let serial_list: Vec<String> =
                        serde_json::from_str(&line.serials).unwrap_or_default();
                    let from = usize::try_from(line.received_quantity).unwrap_or(0);
                    let count = usize::try_from(*quantity).unwrap_or(0);
                    if let Some(arrived) = serial_list.get(from..from + count) {
                        crate::lots::serials_in(
                            &transaction,
                            tenant,
                            line.variant_id,
                            row.to_org_id,
                            arrived,
                        )
                        .await?;
                    }
                    for (code, expires_on, part) in
                        slice_pieces(&pieces, line.received_quantity, *quantity)
                    {
                        let mut new = base.clone();
                        new.quantity = part;
                        let lot = code.as_deref().map(|code| (code, expires_on));
                        crate::lots::record_in(&transaction, tenant, &mut touched, new, lot, &[])
                            .await?;
                    }
                    transfer_line::Entity::update_many()
                        .col_expr(
                            transfer_line::Column::ReceivedQuantity,
                            Expr::value(line.received_quantity + quantity),
                        )
                        .filter(transfer_line::Column::Id.eq(line.id))
                        .scoped(tenant)
                        .exec(&transaction)
                        .await?;
                }
                let done = current.iter().all(|line| {
                    line.received_quantity + wanted.get(&line.id).copied().unwrap_or(0)
                        >= line.quantity
                });
                let to = if done { status::RECEIVED } else { status::PARTIALLY_RECEIVED };
                set_status(&transaction, tenant, row.id, to, false).await?;
                let row = transfer::Entity::find_by_id(id)
                    .scoped(tenant)
                    .one(&transaction)
                    .await?
                    .ok_or_else(|| not_found(id))?;
                let transfer = load(&transaction, tenant, row).await?;
                let actor = Actor::from(principal);
                self.outbox
                    .record(
                        &transaction,
                        tenant,
                        actor,
                        subject(id),
                        &events::TransferReceived(Change::after(snapshot(&transfer))),
                    )
                    .await?;
                stock::publish(&transaction, tenant, &self.outbox, &actor, touched).await?;
                transaction.commit().await?;
                Ok(transfer)
            })
            .await
    }

    /// Đóng phiếu đang đi đường / nhận một phần: phần chưa tới bỏ khỏi `incoming` (mất trên đường —
    /// điều chỉnh có lý do nếu cần) — hay huỷ phiếu nháp.
    pub(crate) async fn finish_transfer(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        cancel: bool,
    ) -> Result<Transfer, Error> {
        let database = tenant.database();
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let row = self.lock_transfer(&transaction, tenant, principal, id, !cancel).await?;
                let to = if cancel { status::CANCELLED } else { status::CLOSED };
                let allowed = if cancel {
                    row.status == status::DRAFT
                } else {
                    row.status == status::IN_TRANSIT || row.status == status::PARTIALLY_RECEIVED
                };
                if !allowed {
                    return Err(invalid_transition(&row.status, to));
                }
                if !cancel {
                    let lines = load(&transaction, tenant, row.clone()).await?.lines;
                    let mut keyed = Vec::new();
                    for line in &lines {
                        let left = line.quantity - line.received_quantity;
                        if left > 0 {
                            keyed.push((
                                stock::level_id(
                                    &transaction,
                                    tenant,
                                    row.to_org_id,
                                    line.variant_id,
                                )
                                .await?,
                                left,
                            ));
                        }
                    }
                    keyed.sort();
                    for (level, left) in keyed {
                        stock::move_incoming(&transaction, tenant, level, -left).await?;
                    }
                }
                set_status(&transaction, tenant, row.id, to, false).await?;
                let row = transfer::Entity::find_by_id(id)
                    .scoped(tenant)
                    .one(&transaction)
                    .await?
                    .ok_or_else(|| not_found(id))?;
                let transfer = load(&transaction, tenant, row).await?;
                transaction.commit().await?;
                Ok(transfer)
            })
            .await
    }
}

/// Một phần lô của dòng chuyển (JSON trong `inventory_transfer_lines.lots`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct LotPiece {
    lot_code: Option<String>,
    expires_on: Option<Date>,
    quantity: i64,
}

/// JSON các phần lô đã lấy ở kho gửi (mã và hạn dùng của từng lô).
async fn pieces_json<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    picks: &[(Option<Uuid>, i64)],
) -> Result<String, Error> {
    let mut pieces = Vec::with_capacity(picks.len());
    for (lot, quantity) in picks {
        let found = match lot {
            Some(id) => {
                crate::entity::lot::Entity::find_by_id(*id).scoped(tenant).one(connection).await?
            }
            None => None,
        };
        pieces.push(LotPiece {
            lot_code: found.as_ref().map(|row| row.lot_code.clone()),
            expires_on: found.and_then(|row| row.expires_on),
            quantity: *quantity,
        });
    }
    serde_json::to_string(&pieces).map_err(Error::internal)
}

/// Các phần lô của `take` đơn vị sau khi bỏ `skip` đơn vị đầu (đã nhận ở lần trước), theo thứ tự
/// lúc gửi. Dòng không có phần lô nào (biến thể không theo dõi lô): một phần không lô.
fn slice_pieces(
    pieces: &[LotPiece],
    skip: i64,
    take: i64,
) -> Vec<(Option<String>, Option<Date>, i64)> {
    if pieces.is_empty() {
        return vec![(None, None, take)];
    }
    let (mut skip, mut take) = (skip, take);
    let mut out = Vec::new();
    for piece in pieces {
        let mut available = piece.quantity;
        let skipped = skip.min(available);
        skip -= skipped;
        available -= skipped;
        let part = take.min(available);
        if part > 0 {
            out.push((piece.lot_code.clone(), piece.expires_on, part));
            take -= part;
        }
    }
    if take > 0 {
        out.push((None, None, take));
    }
    out
}

impl InventoryService {
    /// Biến thể (cờ lô / serial) của các dòng phiếu chuyển `id`, hỏi `Catalog` trước transaction.
    async fn transfer_variants(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<std::collections::HashMap<Uuid, autoxide_catalog::api::VariantSnapshot>, Error>
    {
        let ids: Vec<Uuid> = transfer_line::Entity::find()
            .filter(transfer_line::Column::TransferId.eq(id))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?
            .into_iter()
            .map(|line| line.variant_id)
            .collect();
        if ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        self.variants(tenant, &ids).await
    }
}

#[cfg(test)]
mod tests {
    use super::{LotPiece, slice_pieces};
    use autoxide_kernel::time::Date;

    #[test]
    fn chia_lo_khi_nhan_tung_phan() {
        let piece = |code: &str, quantity| LotPiece {
            lot_code: Some(code.to_owned()),
            expires_on: None,
            quantity,
        };
        let pieces = [piece("A", 3), piece("B", 4)];
        let codes = |parts: Vec<(Option<String>, Option<Date>, i64)>| {
            parts.into_iter().map(|(code, _, q)| (code.unwrap_or_default(), q)).collect::<Vec<_>>()
        };
        assert_eq!(codes(slice_pieces(&pieces, 0, 2)), [("A".to_owned(), 2)]);
        assert_eq!(codes(slice_pieces(&pieces, 2, 3)), [("A".to_owned(), 1), ("B".to_owned(), 2)]);
        assert_eq!(codes(slice_pieces(&pieces, 5, 2)), [("B".to_owned(), 2)]);
        assert_eq!(codes(slice_pieces(&[], 0, 2)), [(String::new(), 2)]);
    }
}
