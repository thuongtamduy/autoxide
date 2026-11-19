//! Phiếu nhập và trả hàng nhà cung cấp (bước 42 mục 6, D-194, D-223…D-227).
//!
//! - Phiếu nhập (`PN-…`): theo đơn đặt hàng (nhận một phần được, không quá số còn lại) hay nhập
//!   thẳng; `reference` chống trùng vĩnh viễn (D-184). Mỗi dòng ghi sổ `receipt` có giá vốn (tính
//!   lại giá vốn bình quân), kèm lô / serial khi biến thể theo dõi.
//! - Trả hàng nhà cung cấp (`TH-…`): ghi sổ `supplier_return`, không lấy hàng đã giữ cho đơn.
//! - Ghi sổ cái qua `dyn Ledger` (module `ledger`, không bắt buộc) bằng job xếp trong cùng
//!   transaction với phiếu: Nợ `inventory.stock` / Có `payable.supplier` (đối tượng nhà cung cấp);
//!   trả hàng đảo lại. Mã tham chiếu `inventory.receipt:<id>` / `inventory.supplier_return:<id>`:
//!   ledger chống trùng theo mã nên job chạy lại an toàn. Phiếu mang `ledger_status`
//!   (`pending` / `posted` / `failed` / `not_applicable`); phiếu `failed` ghi lại được.

use std::collections::HashMap;

use autoxide_catalog::api::VariantSnapshot;
use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{EnqueueOptions, Job, JobQueue};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::reference::{self, Reference, Submitted};
use autoxide_kernel::rows::Bounds;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_ledger::api::{
    AccountKey, Counterparty, LineAccount, NewEntry, NewLine, OpenAccount, Side,
};
use chrono::NaiveDate;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::entity::{
    level, purchase_order, purchase_order_line, receipt, receipt_line, supplier, supplier_return,
    supplier_return_line,
};
use crate::events::{self, Change};
use crate::permissions::{PURCHASING_MANAGE, PURCHASING_READ, READ, RECEIVE};
use crate::purchasing::{purchase_order_not_found, status as po_status, supplier_not_found};
use crate::service::{InventoryService, check_quantity, fail_if_any, now, readable, writable};
use crate::stock::{self, MovementKind, NewMovement, Touched};
use crate::transfers::document_number;

/// Job ghi phiếu nhập / trả hàng vào sổ cái.
pub(crate) const POST_RECEIPT: &str = "inventory.post_receipt_to_ledger";
pub(crate) const POST_SUPPLIER_RETURN: &str = "inventory.post_supplier_return_to_ledger";

/// Mục đích tài khoản hàng tồn kho trong `ledger` (riêng của module, bên thường Nợ).
pub(crate) const STOCK_ACCOUNT: &str = "inventory.stock";

/// Trạng thái ghi sổ của phiếu.
pub(crate) mod ledger_status {
    pub(crate) const PENDING: &str = "pending";
    pub(crate) const POSTED: &str = "posted";
    pub(crate) const FAILED: &str = "failed";
    pub(crate) const NOT_APPLICABLE: &str = "not_applicable";
}

/// Payload của job ghi sổ.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct PostReceipt {
    pub id: Uuid,
}

impl Job for PostReceipt {
    const KIND: &'static str = POST_RECEIPT;
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct PostSupplierReturn {
    pub id: Uuid,
}

impl Job for PostSupplierReturn {
    const KIND: &'static str = POST_SUPPLIER_RETURN;
}

/// Một dòng nhập.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ReceiptLineInput {
    pub variant_id: Uuid,
    pub quantity: i64,
    pub unit_cost: Money,
    pub lot_code: Option<String>,
    pub expires_on: Option<NaiveDate>,
    pub serials: Vec<String>,
}

/// Phiếu nhập mới.
#[derive(Debug, Clone)]
pub(crate) struct ReceiptInput {
    pub org_id: Uuid,
    pub supplier_id: Option<Uuid>,
    pub purchase_order_id: Option<Uuid>,
    pub reference: String,
    pub lines: Vec<ReceiptLineInput>,
}

/// Phiếu nhập (hay trả hàng) kèm dòng.
#[derive(Debug, Clone)]
pub(crate) struct Receipt {
    pub row: receipt::Model,
    pub lines: Vec<receipt_line::Model>,
}

#[derive(Debug, Clone)]
pub(crate) struct SupplierReturn {
    pub row: supplier_return::Model,
    pub lines: Vec<supplier_return_line::Model>,
}

/// Một dòng trả hàng nhà cung cấp; giá trống = giá vốn bình quân hiện tại.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ReturnLineInput {
    pub variant_id: Uuid,
    pub quantity: i64,
    pub unit_cost: Option<Money>,
    pub lot_code: Option<String>,
    pub serials: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ReturnInput {
    pub org_id: Uuid,
    pub supplier_id: Uuid,
    pub receipt_id: Option<Uuid>,
    pub reference: String,
    pub lines: Vec<ReturnLineInput>,
}

#[derive(Serialize)]
struct ReceiptFingerprint<'a> {
    org_id: Uuid,
    supplier_id: Option<Uuid>,
    purchase_order_id: Option<Uuid>,
    lines: &'a [ReceiptLineInput],
}

#[derive(Serialize)]
struct ReturnFingerprint<'a> {
    org_id: Uuid,
    supplier_id: Uuid,
    receipt_id: Option<Uuid>,
    lines: &'a [ReturnLineInput],
}

pub(crate) fn receipt_not_found(id: Uuid) -> Error {
    Error::not_found(
        "inventory.receipt_not_found",
        "There is no goods receipt `{id}` in your scope.",
    )
    .with_param("id", id)
}

fn over_receipt() -> Error {
    Error::conflict(
        "inventory.over_receipt",
        "More units would be received than were ordered or are in transit.",
    )
}

/// Khoá tài khoản công nợ phải trả của nhà cung cấp `row` (theo tiền của nhà cung cấp).
pub(crate) fn payable_key(row: &supplier::Model) -> Result<AccountKey, Error> {
    Ok(AccountKey::new(
        crate::purchasing::book_of(row),
        "payable.supplier",
        Currency::parse(&row.currency)?,
    )
    .with_counterparty(Counterparty::new("supplier", row.id.to_string())))
}

/// Tổng tiền các dòng (cùng một tiền tệ).
fn total(lines: impl Iterator<Item = (i64, Money)>, currency: Currency) -> Result<Money, Error> {
    let mut sum = Money::zero(currency);
    for (quantity, cost) in lines {
        sum = sum.checked_add(cost.checked_mul(quantity)?)?;
    }
    Ok(sum)
}

fn check_tracking(
    problems: &mut Vec<FieldError>,
    index: usize,
    variant: Option<&VariantSnapshot>,
    lot_code: Option<&str>,
    serials: &[String],
    quantity: i64,
    adding: bool,
) {
    let Some(variant) = variant else { return };
    if !variant.track_inventory {
        problems.push(FieldError::new(
            format!("lines[{index}].variant_id"),
            "not_tracked",
            "this variant does not track stock",
        ));
    }
    if adding && variant.lot_tracked && lot_code.is_none() {
        problems.push(FieldError::new(
            format!("lines[{index}].lot_code"),
            "required",
            "this variant tracks lots: give the lot of the units added",
        ));
    }
    if let Some(code) = lot_code {
        crate::lots::check_lot_code(problems, &format!("lines[{index}].lot_code"), code);
    }
    if variant.serial_tracked {
        crate::lots::check_serials(problems, &format!("lines[{index}].serials"), serials, quantity);
    }
}

async fn load_receipt<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    row: receipt::Model,
) -> Result<Receipt, Error> {
    let lines = receipt_line::Entity::find()
        .filter(receipt_line::Column::ReceiptId.eq(row.id))
        .scoped(tenant)
        .order_by_asc(receipt_line::Column::Id)
        .all(connection)
        .await?;
    Ok(Receipt { row, lines })
}

async fn load_return<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    row: supplier_return::Model,
) -> Result<SupplierReturn, Error> {
    let lines = supplier_return_line::Entity::find()
        .filter(supplier_return_line::Column::SupplierReturnId.eq(row.id))
        .scoped(tenant)
        .order_by_asc(supplier_return_line::Column::Id)
        .all(connection)
        .await?;
    Ok(SupplierReturn { row, lines })
}

impl InventoryService {
    /// Nhập hàng vào kho (xem đầu file). Chạy đúng một lần theo `reference`.
    pub(crate) async fn create_receipt(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: ReceiptInput,
    ) -> Result<Submitted<Receipt>, Error> {
        let mut problems = Vec::new();
        let reference = Reference::checked(&input.reference, "reference", &mut problems);
        if input.lines.is_empty() || input.lines.len() > crate::transfers::MAX_LINES {
            problems.push(
                FieldError::new("lines", "length", "must hold 1 to {max} lines")
                    .with_param("max", crate::transfers::MAX_LINES),
            );
        }
        let currency = input.lines.first().map_or(Currency::VND, |line| line.unit_cost.currency());
        let ids: Vec<Uuid> = input.lines.iter().map(|line| line.variant_id).collect();
        let variants = self.variants(tenant, &ids).await?;
        for (index, line) in input.lines.iter().enumerate() {
            check_quantity(
                &mut problems,
                &format!("lines[{index}].quantity"),
                line.quantity,
                false,
            );
            if line.unit_cost.is_negative() || line.unit_cost.currency() != currency {
                problems.push(FieldError::new(
                    format!("lines[{index}].unit_cost"),
                    "invalid",
                    "must not be negative and every line must use the same currency",
                ));
            }
            check_tracking(
                &mut problems,
                index,
                variants.get(&line.variant_id),
                line.lot_code.as_deref(),
                &line.serials,
                line.quantity,
                true,
            );
        }
        if input.purchase_order_id.is_some() && input.supplier_id.is_some() {
            problems.push(FieldError::new(
                "supplier_id",
                "not_allowed",
                "comes from the purchase order: leave it out",
            ));
        }
        fail_if_any(problems)?;
        let Some(reference) = reference else {
            return Err(Error::internal("reference checked without a problem"));
        };
        if input.supplier_id.is_some() || input.purchase_order_id.is_some() {
            self.require_feature(tenant, &crate::service::PURCHASING_FEATURE).await?;
        }
        self.ensure_warehouse(tenant, input.org_id).await?;
        let amount =
            total(input.lines.iter().map(|line| (line.quantity, line.unit_cost)), currency)?;
        let fingerprint = reference::fingerprint(&ReceiptFingerprint {
            org_id: input.org_id,
            supplier_id: input.supplier_id,
            purchase_order_id: input.purchase_order_id,
            lines: &input.lines,
        })?;
        let database = tenant.database();
        let (input, reference, fingerprint, variants) =
            (&input, &reference, &fingerprint, &variants);
        reference::submit_once(database, move || async move {
            let transaction = database.begin_write().await?;
            writable(principal, &transaction, &RECEIVE, input.org_id).await?;
            let stored = receipt::Entity::find()
                .filter(receipt::Column::Reference.eq(reference.as_str()))
                .scoped(tenant)
                .one(&transaction)
                .await?
                .map(|row| {
                    let print = row.reference_fingerprint.clone();
                    (row, print)
                });
            if let Some(row) = reference::replayed(stored, reference, fingerprint)? {
                return Ok(Submitted::Replayed(load_receipt(&transaction, tenant, row).await?));
            }
            // Nhà cung cấp: của đơn đặt hàng, hay nêu thẳng.
            let mut supplier_id = input.supplier_id;
            if let Some(order) = input.purchase_order_id {
                let po = autoxide_kernel::rows::lock_one(&transaction, purchase_order::Entity::find_by_id(order).scoped(tenant))
                    .await?
                    .ok_or_else(|| purchase_order_not_found(order))?;
                readable(principal, &transaction, &PURCHASING_READ, po.org_id, || purchase_order_not_found(order))
                    .await
                    .or(Ok::<(), Error>(()))?;
                if po.org_id != input.org_id
                    || !(po.status == po_status::ORDERED || po.status == po_status::PARTIALLY_RECEIVED)
                {
                    return Err(crate::transfers::invalid_transition(&po.status, po_status::RECEIVED));
                }
                supplier_id = Some(po.supplier_id);
                let lines = crate::purchasing::load_order(&transaction, tenant, po.clone()).await?.lines;
                let mut received: HashMap<Uuid, i64> = HashMap::new();
                for line in &input.lines {
                    let po_line = lines.iter().find(|po_line| po_line.variant_id == line.variant_id).ok_or_else(over_receipt)?;
                    let sum = received.entry(po_line.id).or_default();
                    *sum += line.quantity;
                    if *sum > po_line.quantity - po_line.received_quantity {
                        return Err(over_receipt());
                    }
                }
                let mut keyed = Vec::new();
                for po_line in &lines {
                    if let Some(quantity) = received.get(&po_line.id) {
                        keyed.push((stock::level_id(&transaction, tenant, po.org_id, po_line.variant_id).await?, po_line, *quantity));
                    }
                }
                keyed.sort_by_key(|(level, _, _)| *level);
                for (level, po_line, quantity) in &keyed {
                    stock::move_incoming(&transaction, tenant, *level, -quantity).await?;
                    purchase_order_line::Entity::update_many()
                        .col_expr(purchase_order_line::Column::ReceivedQuantity, Expr::value(po_line.received_quantity + quantity))
                        .filter(purchase_order_line::Column::Id.eq(po_line.id))
                        .scoped(tenant)
                        .exec(&transaction)
                        .await?;
                }
                let done = lines.iter().all(|po_line| {
                    po_line.received_quantity + received.get(&po_line.id).copied().unwrap_or(0) >= po_line.quantity
                });
                let next = if done { po_status::RECEIVED } else { po_status::PARTIALLY_RECEIVED };
                purchase_order::Entity::update_many()
                    .col_expr(purchase_order::Column::Status, Expr::value(next))
                    .col_expr(purchase_order::Column::UpdatedAt, Expr::value(now()))
                    .filter(purchase_order::Column::Id.eq(order))
                    .scoped(tenant)
                    .exec(&transaction)
                    .await?;
            }
            if let Some(id) = supplier_id {
                supplier::Entity::find_by_id(id).scoped(tenant).one(&transaction).await?.ok_or_else(|| supplier_not_found(id))?;
            }
            let posts = self.ledger.is_some() && supplier_id.is_some() && amount.is_positive();
            let stamp = now();
            let row = receipt::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                number: Set(document_number(&transaction, tenant, "PN").await?),
                org_id: Set(input.org_id),
                supplier_id: Set(supplier_id),
                purchase_order_id: Set(input.purchase_order_id),
                reference: Set(reference.as_str().to_owned()),
                reference_fingerprint: Set(fingerprint.clone()),
                posted_at: Set(stamp),
                total_minor: Set(amount.minor()),
                currency: Set(currency.code().to_owned()),
                ledger_status: Set(if posts { ledger_status::PENDING } else { ledger_status::NOT_APPLICABLE }.to_owned()),
                ledger_entry_id: Set(None),
                ledger_posted_at: Set(None),
                ledger_error_code: Set(None),
                created_by: Set(Some(principal.user_id())),
            };
            let row = receipt::Entity::insert(row).exec_with_returning(&transaction).await?;
            let mut ordered = Vec::with_capacity(input.lines.len());
            for line in &input.lines {
                ordered.push((stock::level_id(&transaction, tenant, input.org_id, line.variant_id).await?, line));
            }
            ordered.sort_by_key(|(level, _)| *level);
            let mut touched = Touched::default();
            for (_, line) in &ordered {
                let variant = variants.get(&line.variant_id);
                receipt_line::Entity::insert(receipt_line::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    receipt_id: Set(row.id),
                    variant_id: Set(line.variant_id),
                    quantity: Set(line.quantity),
                    unit_cost_minor: Set(line.unit_cost.minor()),
                    unit_cost_currency: Set(line.unit_cost.currency().code().to_owned()),
                    lot_code: Set(line.lot_code.clone()),
                    expires_on: Set(line.expires_on),
                    serials: Set(serde_json::to_string(&line.serials).map_err(Error::internal)?),
                })
                .exec_without_returning(&transaction)
                .await?;
                let new = NewMovement {
                    org: input.org_id,
                    variant: line.variant_id,
                    kind: MovementKind::Receipt,
                    quantity: line.quantity,
                    touches_available: true,
                    unit_cost: Some(line.unit_cost),
                    lot_id: None,
                    reason_code: None,
                    note: row.number.clone(),
                    source_type: "receipt",
                    source_id: row.id,
                    actor: Some(principal.user_id()),
                };
                let lot = line.lot_code.as_deref().filter(|_| variant.is_some_and(|v| v.lot_tracked)).map(|code| (code, line.expires_on));
                let serials: &[String] = if variant.is_some_and(|v| v.serial_tracked) { &line.serials } else { &[] };
                crate::lots::record_in(&transaction, tenant, &mut touched, new, lot, serials).await?;
            }
            if posts {
                JobQueue::new("inventory")
                    .enqueue(&transaction, tenant, &PostReceipt { id: row.id }, EnqueueOptions::default().unique_key(format!("receipt:{}", row.id)))
                    .await?;
            }
            let receipt = load_receipt(&transaction, tenant, row).await?;
            let actor = Actor::from(principal);
            let after = json!({
                "id": receipt.row.id,
                "number": receipt.row.number,
                "org_id": receipt.row.org_id,
                "supplier_id": receipt.row.supplier_id,
                "purchase_order_id": receipt.row.purchase_order_id,
                "total": amount,
                "lines": receipt.lines.iter().map(|line| json!({"variant_id": line.variant_id, "quantity": line.quantity})).collect::<Vec<_>>(),
            });
            self.outbox
                .record(&transaction, tenant, actor, Some(format!("inventory.receipt:{}", receipt.row.id)), &events::GoodsReceived(Change::after(after)))
                .await?;
            stock::publish(&transaction, tenant, &self.outbox, &actor, touched).await?;
            transaction.commit().await?;
            Ok(Submitted::Created(receipt))
        })
        .await
    }

    pub(crate) async fn get_receipt(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Receipt, Error> {
        let read = tenant.database().read();
        let row = receipt::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| receipt_not_found(id))?;
        if readable(principal, read, &RECEIVE, row.org_id, || receipt_not_found(id)).await.is_err()
        {
            readable(principal, read, &READ, row.org_id, || receipt_not_found(id)).await?;
        }
        load_receipt(read, tenant, row).await
    }

    pub(crate) async fn list_receipts(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        supplier: Option<Uuid>,
        page: &PageRequest,
    ) -> Result<Page<Receipt>, Error> {
        let scope = principal.scope_condition(&RECEIVE, receipt::Column::OrgId, None)?;
        let mut select = receipt::Entity::find().scoped(tenant).filter(scope);
        if let Some(supplier) = supplier {
            select = select.filter(receipt::Column::SupplierId.eq(supplier));
        }
        let read = tenant.database().read();
        let found = fetch_page(select, receipt::Column::Id, |row| row.id, page, read).await?;
        let mut data = Vec::with_capacity(found.data.len());
        for row in found.data {
            data.push(load_receipt(read, tenant, row).await?);
        }
        Ok(Page { data, pagination: found.pagination })
    }

    /// Ghi sổ lại phiếu nhập `ledger_status = failed`: về `pending` và xếp job lại.
    pub(crate) async fn retry_receipt_ledger(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Receipt, Error> {
        let transaction = tenant.database().begin_write().await?;
        let row = receipt::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| receipt_not_found(id))?;
        readable(principal, &transaction, &PURCHASING_MANAGE, row.org_id, || receipt_not_found(id))
            .await?;
        writable(principal, &transaction, &PURCHASING_MANAGE, row.org_id).await?;
        if row.ledger_status != ledger_status::FAILED {
            return Err(crate::transfers::invalid_transition(
                &row.ledger_status,
                ledger_status::PENDING,
            ));
        }
        receipt::Entity::update_many()
            .col_expr(receipt::Column::LedgerStatus, Expr::value(ledger_status::PENDING))
            .col_expr(receipt::Column::LedgerErrorCode, Expr::value(None::<String>))
            .filter(receipt::Column::Id.eq(id))
            .filter(receipt::Column::LedgerStatus.eq(ledger_status::FAILED))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        JobQueue::new("inventory")
            .enqueue(
                &transaction,
                tenant,
                &PostReceipt { id },
                EnqueueOptions::default().unique_key(format!("receipt:{id}:{}", now().timestamp())),
            )
            .await?;
        let row = receipt::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| receipt_not_found(id))?;
        let receipt = load_receipt(&transaction, tenant, row).await?;
        transaction.commit().await?;
        Ok(receipt)
    }

    /// Trả hàng cho nhà cung cấp (xem đầu file). Chạy đúng một lần theo `reference`.
    pub(crate) async fn create_supplier_return(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: ReturnInput,
    ) -> Result<Submitted<SupplierReturn>, Error> {
        let mut problems = Vec::new();
        let reference = Reference::checked(&input.reference, "reference", &mut problems);
        if input.lines.is_empty() || input.lines.len() > crate::transfers::MAX_LINES {
            problems.push(
                FieldError::new("lines", "length", "must hold 1 to {max} lines")
                    .with_param("max", crate::transfers::MAX_LINES),
            );
        }
        let ids: Vec<Uuid> = input.lines.iter().map(|line| line.variant_id).collect();
        let variants = self.variants(tenant, &ids).await?;
        for (index, line) in input.lines.iter().enumerate() {
            check_quantity(
                &mut problems,
                &format!("lines[{index}].quantity"),
                line.quantity,
                false,
            );
            check_tracking(
                &mut problems,
                index,
                variants.get(&line.variant_id),
                line.lot_code.as_deref(),
                &line.serials,
                line.quantity,
                false,
            );
        }
        fail_if_any(problems)?;
        let Some(reference) = reference else {
            return Err(Error::internal("reference checked without a problem"));
        };
        self.require_feature(tenant, &crate::service::PURCHASING_FEATURE).await?;
        self.check_stock_unit(tenant, input.org_id).await?;
        let fingerprint = reference::fingerprint(&ReturnFingerprint {
            org_id: input.org_id,
            supplier_id: input.supplier_id,
            receipt_id: input.receipt_id,
            lines: &input.lines,
        })?;
        let database = tenant.database();
        let (input, reference, fingerprint, variants) =
            (&input, &reference, &fingerprint, &variants);
        reference::submit_once(database, move || async move {
            let transaction = database.begin_write().await?;
            writable(principal, &transaction, &PURCHASING_MANAGE, input.org_id).await?;
            let stored = supplier_return::Entity::find()
                .filter(supplier_return::Column::Reference.eq(reference.as_str()))
                .scoped(tenant)
                .one(&transaction)
                .await?
                .map(|row| {
                    let print = row.reference_fingerprint.clone();
                    (row, print)
                });
            if let Some(row) = reference::replayed(stored, reference, fingerprint)? {
                return Ok(Submitted::Replayed(load_return(&transaction, tenant, row).await?));
            }
            let supplier = supplier::Entity::find_by_id(input.supplier_id)
                .scoped(tenant)
                .one(&transaction)
                .await?
                .ok_or_else(|| supplier_not_found(input.supplier_id))?;
            let mut ordered = Vec::with_capacity(input.lines.len());
            for line in &input.lines {
                ordered.push((
                    stock::level_id(&transaction, tenant, input.org_id, line.variant_id).await?,
                    line,
                ));
            }
            ordered.sort_by_key(|(level, _)| *level);
            // Giá trả: giá nêu, hay giá vốn bình quân hiện tại của dòng tồn.
            let mut costs = HashMap::new();
            for (level_id, line) in &ordered {
                let cost = match line.unit_cost {
                    Some(cost) => cost,
                    None => {
                        let row = level::Entity::find_by_id(*level_id)
                            .scoped(tenant)
                            .one(&transaction)
                            .await?;
                        let currency = row
                            .as_ref()
                            .and_then(|row| row.average_cost_currency.clone())
                            .unwrap_or_else(|| supplier.currency.clone());
                        Money::from_columns(row.map_or(0, |row| row.average_cost_minor), &currency)?
                    }
                };
                costs.insert(line.variant_id, cost);
            }
            let currency = costs.values().next().map_or(Currency::VND, |cost| cost.currency());
            if costs.values().any(|cost| cost.currency() != currency) {
                return Err(Error::validation(vec![FieldError::new(
                    "lines",
                    "currency",
                    "every line must use the same currency",
                )]));
            }
            let amount = total(
                input.lines.iter().map(|line| (line.quantity, costs[&line.variant_id])),
                currency,
            )?;
            let posts = self.ledger.is_some() && amount.is_positive();
            let stamp = now();
            let row = supplier_return::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                number: Set(document_number(&transaction, tenant, "TH").await?),
                org_id: Set(input.org_id),
                supplier_id: Set(supplier.id),
                receipt_id: Set(input.receipt_id),
                reference: Set(reference.as_str().to_owned()),
                reference_fingerprint: Set(fingerprint.clone()),
                posted_at: Set(stamp),
                total_minor: Set(amount.minor()),
                currency: Set(currency.code().to_owned()),
                ledger_status: Set(if posts {
                    ledger_status::PENDING
                } else {
                    ledger_status::NOT_APPLICABLE
                }
                .to_owned()),
                ledger_entry_id: Set(None),
                ledger_posted_at: Set(None),
                ledger_error_code: Set(None),
                created_by: Set(Some(principal.user_id())),
            };
            let row =
                supplier_return::Entity::insert(row).exec_with_returning(&transaction).await?;
            let mut touched = Touched::default();
            let mut short = Vec::new();
            for (level_id, line) in &ordered {
                let cost = costs[&line.variant_id];
                supplier_return_line::Entity::insert(supplier_return_line::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    supplier_return_id: Set(row.id),
                    variant_id: Set(line.variant_id),
                    quantity: Set(line.quantity),
                    unit_cost_minor: Set(cost.minor()),
                    unit_cost_currency: Set(cost.currency().code().to_owned()),
                    lot_code: Set(line.lot_code.clone()),
                    serials: Set(serde_json::to_string(&line.serials).map_err(Error::internal)?),
                })
                .exec_without_returning(&transaction)
                .await?;
                // Không lấy hàng đã giữ cho đơn.
                if !stock::move_available(
                    &transaction,
                    tenant,
                    &mut touched,
                    *level_id,
                    -line.quantity,
                    Bounds::NON_NEGATIVE,
                )
                .await?
                {
                    short.push(format!("variant `{}`", line.variant_id));
                    continue;
                }
                let variant = variants.get(&line.variant_id);
                let tracking = crate::lots::Tracking {
                    lot_tracked: variant.is_some_and(|v| v.lot_tracked),
                    lot_code: line.lot_code.as_deref(),
                    serials: variant
                        .is_some_and(|v| v.serial_tracked)
                        .then_some(line.serials.as_slice()),
                    serial_status: "returned",
                    order: None,
                };
                let new = NewMovement {
                    org: input.org_id,
                    variant: line.variant_id,
                    kind: MovementKind::SupplierReturn,
                    quantity: -line.quantity,
                    touches_available: false,
                    unit_cost: Some(cost),
                    lot_id: None,
                    reason_code: None,
                    note: row.number.clone(),
                    source_type: "supplier_return",
                    source_id: row.id,
                    actor: Some(principal.user_id()),
                };
                crate::lots::record_out(&transaction, tenant, &mut touched, new, &tracking).await?;
            }
            if !short.is_empty() {
                return Err(crate::errors::insufficient_stock(&short.join(", ")));
            }
            if posts {
                JobQueue::new("inventory")
                    .enqueue(
                        &transaction,
                        tenant,
                        &PostSupplierReturn { id: row.id },
                        EnqueueOptions::default().unique_key(format!("supplier_return:{}", row.id)),
                    )
                    .await?;
            }
            let found = load_return(&transaction, tenant, row).await?;
            let actor = Actor::from(principal);
            let after = json!({
                "id": found.row.id,
                "number": found.row.number,
                "org_id": found.row.org_id,
                "supplier_id": found.row.supplier_id,
                "total": amount,
            });
            self.outbox
                .record(
                    &transaction,
                    tenant,
                    actor,
                    Some(format!("inventory.supplier_return:{}", found.row.id)),
                    &events::SupplierReturned(Change::after(after)),
                )
                .await?;
            stock::publish(&transaction, tenant, &self.outbox, &actor, touched).await?;
            transaction.commit().await?;
            Ok(Submitted::Created(found))
        })
        .await
    }

    pub(crate) async fn get_supplier_return(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<SupplierReturn, Error> {
        let read = tenant.database().read();
        let not_found = || {
            Error::not_found(
                "inventory.supplier_return_not_found",
                "There is no supplier return `{id}` in your scope.",
            )
            .with_param("id", id)
        };
        let row = supplier_return::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(not_found)?;
        readable(principal, read, &PURCHASING_READ, row.org_id, not_found).await?;
        load_return(read, tenant, row).await
    }

    pub(crate) async fn list_supplier_returns(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<SupplierReturn>, Error> {
        let scope =
            principal.scope_condition(&PURCHASING_READ, supplier_return::Column::OrgId, None)?;
        let select = supplier_return::Entity::find().scoped(tenant).filter(scope);
        let read = tenant.database().read();
        let found =
            fetch_page(select, supplier_return::Column::Id, |row| row.id, page, read).await?;
        let mut data = Vec::with_capacity(found.data.len());
        for row in found.data {
            data.push(load_return(read, tenant, row).await?);
        }
        Ok(Page { data, pagination: found.pagination })
    }

    /// Tài khoản hàng tồn kho của sổ `book` (mục đích riêng `inventory.stock`, bên thường Nợ).
    async fn stock_account(
        &self,
        tenant: &TenantContext,
        book: autoxide_kernel::tenancy::OrgId,
        currency: Currency,
    ) -> Result<Uuid, Error> {
        let Some(ledger) = &self.ledger else {
            return Err(Error::internal("ledger is not installed"));
        };
        let key = AccountKey::new(book, STOCK_ACCOUNT, currency);
        ledger
            .open_account(tenant, OpenAccount::new(key).normal_side(Side::Debit).name("Inventory"))
            .await
    }

    /// Job: ghi phiếu nhập `id` vào sổ cái (Nợ hàng tồn kho / Có phải trả nhà cung cấp).
    pub(crate) async fn post_receipt(&self, tenant: &TenantContext, id: Uuid) -> Result<(), Error> {
        let Some(row) =
            receipt::Entity::find_by_id(id).scoped(tenant).one(tenant.database().read()).await?
        else {
            return Ok(());
        };
        if row.ledger_status != ledger_status::PENDING {
            return Ok(());
        }
        let Some(supplier_id) = row.supplier_id else { return Ok(()) };
        let reference = format!("inventory.receipt:{id}");
        let outcome = self
            .post_document(
                tenant,
                supplier_id,
                "receipt",
                id,
                &reference,
                row.total_minor,
                &row.currency,
                false,
            )
            .await;
        self.settle_posting(tenant, "receipt", id, outcome).await
    }

    /// Job: ghi trả hàng nhà cung cấp `id` vào sổ cái (đảo bút toán nhập).
    pub(crate) async fn post_supplier_return(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<(), Error> {
        let Some(row) = supplier_return::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
        else {
            return Ok(());
        };
        if row.ledger_status != ledger_status::PENDING {
            return Ok(());
        }
        let reference = format!("inventory.supplier_return:{id}");
        let outcome = self
            .post_document(
                tenant,
                row.supplier_id,
                "supplier_return",
                id,
                &reference,
                row.total_minor,
                &row.currency,
                true,
            )
            .await;
        self.settle_posting(tenant, "supplier_return", id, outcome).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn post_document(
        &self,
        tenant: &TenantContext,
        supplier_id: Uuid,
        kind: &str,
        id: Uuid,
        reference: &str,
        total_minor: i64,
        currency: &str,
        reverse: bool,
    ) -> Result<Uuid, Error> {
        let Some(ledger) = &self.ledger else {
            return Err(Error::internal("ledger is not installed"));
        };
        let supplier = supplier::Entity::find_by_id(supplier_id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| supplier_not_found(supplier_id))?;
        let amount = Money::from_columns(total_minor, currency)?;
        let book = crate::purchasing::book_of(&supplier);
        let stock_account =
            LineAccount::Id(self.stock_account(tenant, book, amount.currency()).await?);
        let payable = LineAccount::Key(
            AccountKey::new(book, "payable.supplier", amount.currency())
                .with_counterparty(Counterparty::new("supplier", supplier.id.to_string())),
        );
        let lines = if reverse {
            vec![NewLine::debit(payable, amount), NewLine::credit(stock_account, amount)]
        } else {
            vec![NewLine::debit(stock_account, amount), NewLine::credit(payable, amount)]
        };
        let description = if reverse {
            "Return of goods to a supplier"
        } else {
            "Goods received from a supplier"
        };
        let entry = NewEntry::new(book, "inventory", kind, reference, description, lines)
            .source_id(id.to_string());
        Ok(ledger.post(tenant, Actor::system(), entry).await?.into_inner().id)
    }

    /// Ghi kết quả ghi sổ vào phiếu: thành công → `posted`; lỗi nghiệp vụ → `failed` + mã (ghi lại
    /// bằng tay); lỗi hệ thống → trả lỗi để job chạy lại.
    async fn settle_posting(
        &self,
        tenant: &TenantContext,
        kind: &str,
        id: Uuid,
        outcome: Result<Uuid, Error>,
    ) -> Result<(), Error> {
        let write = tenant.database().write();
        let (status, entry, code) = match outcome {
            Ok(entry) => (ledger_status::POSTED, Some(entry), None),
            Err(error) if error.kind().status() < 500 => {
                (ledger_status::FAILED, None, Some(error.code().to_owned()))
            }
            Err(error) => return Err(error),
        };
        let posted_at = (status == ledger_status::POSTED).then(now);
        if kind == "receipt" {
            receipt::Entity::update_many()
                .col_expr(receipt::Column::LedgerStatus, Expr::value(status))
                .col_expr(receipt::Column::LedgerEntryId, Expr::value(entry))
                .col_expr(receipt::Column::LedgerPostedAt, Expr::value(posted_at))
                .col_expr(receipt::Column::LedgerErrorCode, Expr::value(code))
                .filter(receipt::Column::Id.eq(id))
                .filter(receipt::Column::LedgerStatus.eq(ledger_status::PENDING))
                .scoped(tenant)
                .exec(write)
                .await?;
        } else {
            supplier_return::Entity::update_many()
                .col_expr(supplier_return::Column::LedgerStatus, Expr::value(status))
                .col_expr(supplier_return::Column::LedgerEntryId, Expr::value(entry))
                .col_expr(supplier_return::Column::LedgerPostedAt, Expr::value(posted_at))
                .col_expr(supplier_return::Column::LedgerErrorCode, Expr::value(code))
                .filter(supplier_return::Column::Id.eq(id))
                .filter(supplier_return::Column::LedgerStatus.eq(ledger_status::PENDING))
                .scoped(tenant)
                .exec(write)
                .await?;
        }
        Ok(())
    }
}
