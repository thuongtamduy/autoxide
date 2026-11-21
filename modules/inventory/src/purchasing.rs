//! Nhập hàng (bước 42 mục 6, D-194): nhà cung cấp và đơn đặt hàng nhà cung cấp. Phiếu nhập và trả
//! hàng nhà cung cấp ở `receipts.rs`.
//!
//! Đơn đặt hàng: nháp → `ordered` (cộng `incoming` ở kho nhận) → nhận một phần / đủ qua phiếu nhập →
//! `closed` (phần chưa về bỏ khỏi `incoming`) hay `cancelled` (khi chưa nhận gì).

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::rows;
use autoxide_kernel::search;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, root_of};
use autoxide_kernel::time::Date;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, QueryFilter,
    QueryOrder,
};
use uuid::Uuid;

use crate::entity::{purchase_order, purchase_order_line, supplier};
use crate::permissions::{PURCHASING_MANAGE, PURCHASING_READ};
use crate::service::{
    InventoryService, PURCHASING_FEATURE, check_quantity, fail_if_any, now, readable, writable,
};
use crate::stock;
use crate::transfers::{document_number, invalid_transition};

/// Trạng thái của đơn đặt hàng.
pub(crate) mod status {
    pub(crate) const DRAFT: &str = "draft";
    pub(crate) const ORDERED: &str = "ordered";
    pub(crate) const PARTIALLY_RECEIVED: &str = "partially_received";
    pub(crate) const RECEIVED: &str = "received";
    pub(crate) const CLOSED: &str = "closed";
    pub(crate) const CANCELLED: &str = "cancelled";
}

/// Nhà cung cấp mới hay thay đổi (trường `None` khi sửa = giữ nguyên).
#[derive(Debug, Clone, Default)]
pub(crate) struct SupplierInput {
    pub org_id: Option<Uuid>,
    pub code: Option<String>,
    pub name: Option<String>,
    pub tax_code: Option<Option<String>>,
    pub contact_name: Option<Option<String>>,
    pub phone: Option<Option<String>>,
    pub email: Option<Option<String>>,
    pub address: Option<Option<String>>,
    pub payment_terms_days: Option<i32>,
    pub currency: Option<String>,
    pub active: Option<bool>,
}

/// Nhà cung cấp kèm công nợ phải trả (khi có `ledger`).
#[derive(Debug, Clone)]
pub(crate) struct Supplier {
    pub row: supplier::Model,
    pub payable: Option<Money>,
}

/// Một dòng của đơn đặt hàng mới.
#[derive(Debug, Clone)]
pub(crate) struct PurchaseLineInput {
    pub variant_id: Uuid,
    pub quantity: i64,
    pub unit_cost: Money,
}

/// Đơn đặt hàng mới hay thay đổi (khi còn nháp).
#[derive(Debug, Clone)]
pub(crate) struct PurchaseOrderInput {
    pub org_id: Uuid,
    pub supplier_id: Uuid,
    pub expected_on: Option<Date>,
    pub note: Option<String>,
    pub lines: Vec<PurchaseLineInput>,
}

/// Đơn đặt hàng kèm dòng.
#[derive(Debug, Clone)]
pub(crate) struct PurchaseOrder {
    pub row: purchase_order::Model,
    pub lines: Vec<purchase_order_line::Model>,
}

pub(crate) fn supplier_not_found(id: Uuid) -> Error {
    Error::not_found("inventory.supplier_not_found", "There is no supplier `{id}` in your scope.")
        .with_param("id", id)
}

pub(crate) fn purchase_order_not_found(id: Uuid) -> Error {
    Error::not_found(
        "inventory.purchase_order_not_found",
        "There is no purchase order `{id}` in your scope.",
    )
    .with_param("id", id)
}

fn code_taken(code: &str) -> Error {
    Error::conflict("inventory.supplier_code_taken", "The supplier code `{code}` is already used.")
        .with_param("code", code.to_owned())
}

/// Chữ tuỳ chọn: bỏ khoảng trắng hai đầu, rỗng thành `None`, kiểm độ dài.
fn optional_text(
    problems: &mut Vec<FieldError>,
    field: &str,
    value: Option<&str>,
    max: usize,
) -> Option<String> {
    let value = value.map(str::trim).filter(|value| !value.is_empty())?;
    if value.chars().count() > max {
        problems.push(
            FieldError::new(field, "length", "must be at most {max} characters")
                .with_param("max", max),
        );
    }
    Some(value.to_owned())
}

pub(crate) async fn load_order<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    row: purchase_order::Model,
) -> Result<PurchaseOrder, Error> {
    let lines = purchase_order_line::Entity::find()
        .filter(purchase_order_line::Column::PurchaseOrderId.eq(row.id))
        .scoped(tenant)
        .order_by_asc(purchase_order_line::Column::Id)
        .all(connection)
        .await?;
    Ok(PurchaseOrder { row, lines })
}

impl InventoryService {
    /// Công nợ phải trả của các nhà cung cấp (một lần gọi `Ledger::balances`); `None` khi chưa cài
    /// `ledger`.
    async fn payables(
        &self,
        tenant: &TenantContext,
        rows: &[supplier::Model],
    ) -> Result<Vec<Option<Money>>, Error> {
        let Some(ledger) = &self.ledger else { return Ok(vec![None; rows.len()]) };
        let mut keys = Vec::with_capacity(rows.len());
        for row in rows {
            keys.push(crate::receipts::payable_key(row)?);
        }
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        Ok(ledger.balances(tenant, &keys, None).await?.into_iter().map(Some).collect())
    }

    pub(crate) async fn list_suppliers(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        q: Option<&str>,
        active: Option<bool>,
        page: &PageRequest,
    ) -> Result<Page<Supplier>, Error> {
        let scope = principal.scope_condition(&PURCHASING_READ, supplier::Column::OrgId, None)?;
        let mut select = supplier::Entity::find().scoped(tenant).filter(scope);
        if let Some(q) = q.map(str::trim).filter(|q| !q.is_empty()) {
            select = select.filter(supplier::Column::NameKey.contains(search::fold(q)));
        }
        if let Some(active) = active {
            select = select.filter(supplier::Column::Active.eq(active));
        }
        let found =
            fetch_page(select, supplier::Column::Id, |row| row.id, page, tenant.database().read())
                .await?;
        let payables = self.payables(tenant, &found.data).await?;
        let data = found
            .data
            .into_iter()
            .zip(payables)
            .map(|(row, payable)| Supplier { row, payable })
            .collect();
        Ok(Page { data, pagination: found.pagination })
    }

    pub(crate) async fn get_supplier(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Supplier, Error> {
        let read = tenant.database().read();
        let row = supplier::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| supplier_not_found(id))?;
        readable(principal, read, &PURCHASING_READ, row.org_id, || supplier_not_found(id)).await?;
        let payable = self.payables(tenant, std::slice::from_ref(&row)).await?.pop().flatten();
        Ok(Supplier { row, payable })
    }

    /// Tạo (`id = None`) hay sửa nhà cung cấp.
    pub(crate) async fn save_supplier(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Option<Uuid>,
        input: SupplierInput,
    ) -> Result<Supplier, Error> {
        self.require_feature(tenant, &PURCHASING_FEATURE).await?;
        let mut problems = Vec::new();
        let code = optional_text(&mut problems, "code", input.code.as_deref(), 64);
        let name = optional_text(&mut problems, "name", input.name.as_deref(), 200);
        if id.is_none() && code.is_none() {
            problems.push(FieldError::new("code", "required", "is required"));
        }
        if id.is_none() && name.is_none() {
            problems.push(FieldError::new("name", "required", "is required"));
        }
        let text =
            |problems: &mut Vec<FieldError>, field: &str, value: &Option<Option<String>>, max| {
                value.as_ref().map(|value| optional_text(problems, field, value.as_deref(), max))
            };
        let tax_code = text(&mut problems, "tax_code", &input.tax_code, 20);
        let contact_name = text(&mut problems, "contact_name", &input.contact_name, 200);
        let phone = text(&mut problems, "phone", &input.phone, 32);
        let email = text(&mut problems, "email", &input.email, 254);
        let address = text(&mut problems, "address", &input.address, 500);
        if input.payment_terms_days.is_some_and(|days| !(0..=365).contains(&days)) {
            problems.push(FieldError::new(
                "payment_terms_days",
                "range",
                "must be a whole number of days from 0 to 365",
            ));
        }
        let currency = match &input.currency {
            Some(code) => match Currency::parse(code) {
                Ok(currency) => Some(currency),
                Err(_) => {
                    problems.push(FieldError::new(
                        "currency",
                        "invalid",
                        "must be an ISO 4217 code like `VND`",
                    ));
                    None
                }
            },
            None => None,
        };
        fail_if_any(problems)?;
        let transaction = tenant.database().begin_write().await?;
        let stamp = now();
        let saved = match id {
            None => {
                let org = input.org_id.unwrap_or_else(|| root_of(tenant.id()).as_uuid());
                writable(principal, &transaction, &PURCHASING_MANAGE, org).await?;
                let (code, name) = (code.unwrap_or_default(), name.unwrap_or_default());
                let row = supplier::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    org_id: Set(org),
                    code: Set(code.clone()),
                    name_key: Set(search::fold(&name)),
                    name: Set(name),
                    tax_code: Set(tax_code.flatten()),
                    contact_name: Set(contact_name.flatten()),
                    phone: Set(phone.flatten()),
                    email: Set(email.flatten()),
                    address: Set(address.flatten()),
                    payment_terms_days: Set(input.payment_terms_days.unwrap_or(0)),
                    currency: Set(currency.map_or("VND", Currency::code).to_owned()),
                    active: Set(input.active.unwrap_or(true)),
                    created_at: Set(stamp),
                    updated_at: Set(stamp),
                };
                supplier::Entity::insert(row).exec_with_returning(&transaction).await.map_err(
                    |error| {
                        if crate::service::unique_violation(&error) {
                            code_taken(&code)
                        } else {
                            error.into()
                        }
                    },
                )?
            }
            Some(id) => {
                let current =
                    rows::lock_one(&transaction, supplier::Entity::find_by_id(id).scoped(tenant))
                        .await?
                        .ok_or_else(|| supplier_not_found(id))?;
                readable(principal, &transaction, &PURCHASING_READ, current.org_id, || {
                    supplier_not_found(id)
                })
                .await?;
                writable(principal, &transaction, &PURCHASING_MANAGE, current.org_id).await?;
                let mut row: supplier::ActiveModel = current.clone().into();
                if let Some(code) = &code {
                    row.code = Set(code.clone());
                }
                if let Some(name) = name {
                    row.name_key = Set(search::fold(&name));
                    row.name = Set(name);
                }
                if let Some(value) = tax_code {
                    row.tax_code = Set(value);
                }
                if let Some(value) = contact_name {
                    row.contact_name = Set(value);
                }
                if let Some(value) = phone {
                    row.phone = Set(value);
                }
                if let Some(value) = email {
                    row.email = Set(value);
                }
                if let Some(value) = address {
                    row.address = Set(value);
                }
                if let Some(days) = input.payment_terms_days {
                    row.payment_terms_days = Set(days);
                }
                if let Some(currency) = currency {
                    row.currency = Set(currency.code().to_owned());
                }
                if let Some(active) = input.active {
                    row.active = Set(active);
                }
                row.updated_at = Set(stamp);
                supplier::Entity::update(row).exec(&transaction).await.map_err(|error| {
                    if crate::service::unique_violation(&error) {
                        code_taken(code.as_deref().unwrap_or_default())
                    } else {
                        error.into()
                    }
                })?
            }
        };
        transaction.commit().await?;
        let payable = self.payables(tenant, std::slice::from_ref(&saved)).await?.pop().flatten();
        Ok(Supplier { row: saved, payable })
    }

    fn check_order(&self, input: &PurchaseOrderInput) -> Result<(), Error> {
        let mut problems = Vec::new();
        if input.lines.is_empty() || input.lines.len() > crate::transfers::MAX_LINES {
            problems.push(
                FieldError::new("lines", "length", "must hold 1 to {max} lines")
                    .with_param("max", crate::transfers::MAX_LINES),
            );
        }
        let currency = input.lines.first().map(|line| line.unit_cost.currency());
        let mut seen = std::collections::BTreeSet::new();
        for (index, line) in input.lines.iter().enumerate() {
            check_quantity(
                &mut problems,
                &format!("lines[{index}].quantity"),
                line.quantity,
                false,
            );
            if line.unit_cost.is_negative() || Some(line.unit_cost.currency()) != currency {
                problems.push(FieldError::new(
                    format!("lines[{index}].unit_cost"),
                    "invalid",
                    "must not be negative and every line must use the same currency",
                ));
            }
            if !seen.insert(line.variant_id) {
                problems.push(FieldError::new(
                    format!("lines[{index}].variant_id"),
                    "duplicate",
                    "lists the same variant twice",
                ));
            }
        }
        if input.note.as_deref().is_some_and(|note| note.chars().count() > 500) {
            problems.push(FieldError::new("note", "length", "must be at most 500 characters"));
        }
        fail_if_any(problems)
    }

    /// Tổng tiền của các dòng (tràn số → lỗi).
    fn order_total(lines: &[PurchaseLineInput]) -> Result<Money, Error> {
        let currency = lines.first().map_or(Currency::VND, |line| line.unit_cost.currency());
        let mut total = Money::zero(currency);
        for line in lines {
            total = total.checked_add(line.unit_cost.checked_mul(line.quantity)?)?;
        }
        Ok(total)
    }

    async fn write_lines(
        transaction: &DatabaseTransaction,
        tenant: &TenantContext,
        order: Uuid,
        lines: &[PurchaseLineInput],
    ) -> Result<(), Error> {
        purchase_order_line::Entity::delete_many()
            .filter(purchase_order_line::Column::PurchaseOrderId.eq(order))
            .scoped(tenant)
            .exec(transaction)
            .await?;
        for line in lines {
            purchase_order_line::Entity::insert(purchase_order_line::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                purchase_order_id: Set(order),
                variant_id: Set(line.variant_id),
                quantity: Set(line.quantity),
                received_quantity: Set(0),
                unit_cost_minor: Set(line.unit_cost.minor()),
                unit_cost_currency: Set(line.unit_cost.currency().code().to_owned()),
            })
            .exec_without_returning(transaction)
            .await?;
        }
        Ok(())
    }

    /// Tạo đơn đặt hàng nháp.
    pub(crate) async fn create_purchase_order(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: PurchaseOrderInput,
    ) -> Result<PurchaseOrder, Error> {
        self.check_order(&input)?;
        self.require_feature(tenant, &PURCHASING_FEATURE).await?;
        let ids: Vec<Uuid> = input.lines.iter().map(|line| line.variant_id).collect();
        let variants = self.variants(tenant, &ids).await?;
        if let Some(line) = input
            .lines
            .iter()
            .find(|line| variants.get(&line.variant_id).is_some_and(|v| !v.track_inventory))
        {
            return Err(crate::errors::not_tracked(line.variant_id));
        }
        self.ensure_warehouse(tenant, input.org_id).await?;
        let total = Self::order_total(&input.lines)?;
        let transaction = tenant.database().begin_write().await?;
        writable(principal, &transaction, &PURCHASING_MANAGE, input.org_id).await?;
        let supplier = supplier::Entity::find_by_id(input.supplier_id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| supplier_not_found(input.supplier_id))?;
        readable(principal, &transaction, &PURCHASING_READ, supplier.org_id, || {
            supplier_not_found(supplier.id)
        })
        .await?;
        let stamp = now();
        let row = purchase_order::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            number: Set(document_number(&transaction, tenant, "PO").await?),
            org_id: Set(input.org_id),
            supplier_id: Set(supplier.id),
            status: Set(status::DRAFT.to_owned()),
            expected_on: Set(input.expected_on),
            total_minor: Set(total.minor()),
            currency: Set(total.currency().code().to_owned()),
            note: Set(input.note.unwrap_or_default()),
            created_by: Set(Some(principal.user_id())),
            created_at: Set(stamp),
            updated_at: Set(stamp),
        };
        let row = purchase_order::Entity::insert(row).exec_with_returning(&transaction).await?;
        Self::write_lines(&transaction, tenant, row.id, &input.lines).await?;
        let order = load_order(&transaction, tenant, row).await?;
        transaction.commit().await?;
        Ok(order)
    }

    pub(crate) async fn get_purchase_order(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<PurchaseOrder, Error> {
        let read = tenant.database().read();
        let row = purchase_order::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| purchase_order_not_found(id))?;
        readable(principal, read, &PURCHASING_READ, row.org_id, || purchase_order_not_found(id))
            .await?;
        load_order(read, tenant, row).await
    }

    pub(crate) async fn list_purchase_orders(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        supplier: Option<Uuid>,
        status: Option<&str>,
        page: &PageRequest,
    ) -> Result<Page<PurchaseOrder>, Error> {
        let scope =
            principal.scope_condition(&PURCHASING_READ, purchase_order::Column::OrgId, None)?;
        let mut select = purchase_order::Entity::find().scoped(tenant).filter(scope);
        if let Some(supplier) = supplier {
            select = select.filter(purchase_order::Column::SupplierId.eq(supplier));
        }
        if let Some(status) = status {
            select = select.filter(purchase_order::Column::Status.eq(status));
        }
        let read = tenant.database().read();
        let found =
            fetch_page(select, purchase_order::Column::Id, |row| row.id, page, read).await?;
        let mut data = Vec::with_capacity(found.data.len());
        for row in found.data {
            data.push(load_order(read, tenant, row).await?);
        }
        Ok(Page { data, pagination: found.pagination })
    }

    /// Sửa đơn nháp: hạn về, ghi chú, dòng (thay toàn bộ).
    pub(crate) async fn update_purchase_order(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        input: PurchaseOrderInput,
    ) -> Result<PurchaseOrder, Error> {
        self.check_order(&input)?;
        let ids: Vec<Uuid> = input.lines.iter().map(|line| line.variant_id).collect();
        self.variants(tenant, &ids).await?;
        let total = Self::order_total(&input.lines)?;
        let transaction = tenant.database().begin_write().await?;
        let row = self.lock_order(&transaction, tenant, principal, id).await?;
        if row.status != status::DRAFT {
            return Err(invalid_transition(&row.status, status::DRAFT));
        }
        purchase_order::Entity::update_many()
            .col_expr(purchase_order::Column::ExpectedOn, Expr::value(input.expected_on))
            .col_expr(purchase_order::Column::Note, Expr::value(input.note.unwrap_or_default()))
            .col_expr(purchase_order::Column::TotalMinor, Expr::value(total.minor()))
            .col_expr(purchase_order::Column::Currency, Expr::value(total.currency().code()))
            .col_expr(purchase_order::Column::UpdatedAt, Expr::value(now()))
            .filter(purchase_order::Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        Self::write_lines(&transaction, tenant, id, &input.lines).await?;
        let row = purchase_order::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| purchase_order_not_found(id))?;
        let order = load_order(&transaction, tenant, row).await?;
        transaction.commit().await?;
        Ok(order)
    }

    pub(crate) async fn lock_order(
        &self,
        transaction: &DatabaseTransaction,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<purchase_order::Model, Error> {
        let row =
            rows::lock_one(transaction, purchase_order::Entity::find_by_id(id).scoped(tenant))
                .await?
                .ok_or_else(|| purchase_order_not_found(id))?;
        readable(principal, transaction, &PURCHASING_READ, row.org_id, || {
            purchase_order_not_found(id)
        })
        .await?;
        writable(principal, transaction, &PURCHASING_MANAGE, row.org_id).await?;
        Ok(row)
    }

    /// Chuyển trạng thái đơn: `order` (nháp → đã đặt, cộng `incoming`), `close` (bỏ phần chưa về
    /// khỏi `incoming`), `cancel` (nháp, hay đã đặt mà chưa nhận gì).
    pub(crate) async fn move_purchase_order(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        to: &'static str,
    ) -> Result<PurchaseOrder, Error> {
        let database = tenant.database();
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let row = self.lock_order(&transaction, tenant, principal, id).await?;
                let lines = load_order(&transaction, tenant, row.clone()).await?.lines;
                let received = lines.iter().any(|line| line.received_quantity > 0);
                let allowed = match to {
                    status::ORDERED => row.status == status::DRAFT,
                    status::CLOSED => {
                        row.status == status::ORDERED || row.status == status::PARTIALLY_RECEIVED
                    }
                    status::CANCELLED => {
                        row.status == status::DRAFT || (row.status == status::ORDERED && !received)
                    }
                    _ => false,
                };
                if !allowed {
                    return Err(invalid_transition(&row.status, to));
                }
                let was_ordered = row.status != status::DRAFT;
                // Đặt: cộng phần chưa về vào `incoming`; đóng / huỷ đơn đã đặt: trừ lại.
                let sign = if to == status::ORDERED {
                    1
                } else if was_ordered {
                    -1
                } else {
                    0
                };
                if sign != 0 {
                    let mut keyed = Vec::new();
                    for line in &lines {
                        let left = line.quantity - line.received_quantity;
                        if left > 0 {
                            keyed.push((
                                stock::level_id(&transaction, tenant, row.org_id, line.variant_id)
                                    .await?,
                                left,
                            ));
                        }
                    }
                    keyed.sort();
                    for (level, left) in keyed {
                        stock::move_incoming(&transaction, tenant, level, sign * left).await?;
                    }
                }
                purchase_order::Entity::update_many()
                    .col_expr(purchase_order::Column::Status, Expr::value(to))
                    .col_expr(purchase_order::Column::UpdatedAt, Expr::value(now()))
                    .filter(purchase_order::Column::Id.eq(id))
                    .scoped(tenant)
                    .exec(&transaction)
                    .await?;
                let row = purchase_order::Entity::find_by_id(id)
                    .scoped(tenant)
                    .one(&transaction)
                    .await?
                    .ok_or_else(|| purchase_order_not_found(id))?;
                let order = load_order(&transaction, tenant, row).await?;
                transaction.commit().await?;
                Ok(order)
            })
            .await
    }
}

/// Nút của nhà cung cấp làm sổ cái cho bút toán công nợ (đơn vị mua hàng).
pub(crate) fn book_of(row: &supplier::Model) -> OrgId {
    OrgId::from_uuid(row.org_id)
}
