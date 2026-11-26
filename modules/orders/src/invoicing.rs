//! Hoá đơn điện tử của đơn (đặc tả 45 mục "Hoá đơn và chứng từ của người mua", D-215, D-222):
//!
//! - **Khi nào:** cài đặt `orders.invoice_timing` của nút — `on_confirm`, `on_delivered` (mặc định:
//!   lúc chuyển quyền sở hữu, NĐ 123/2020 sửa bởi NĐ 70/2025 Điều 9 — cần đối chiếu), `on_complete`,
//!   `on_request` (chỉ khi người mua / nhân viên xin). `orders` nghe event của chính mình rồi gọi
//!   `dyn Invoices` của `invoices` — NGOÀI transaction của đơn.
//! - **Lập gì:** một chứng từ bán hàng (`vat_invoice` / `sales_invoice` theo `orders.invoice_kind`;
//!   bán tại quầy: `cash_register_invoice` — hoá đơn khởi tạo từ máy tính tiền), giá gồm thuế như
//!   trên đơn, khoản giảm theo dòng, phí giao là dòng dịch vụ. `invoices` cấp số, tính thuế, gửi
//!   nhà cung cấp hoá đơn điện tử (`EInvoiceProvider`) theo ký hiệu. Chứng từ mang cờ
//!   `booked_by_source`: doanh thu đã do `orders` ghi khi giao (không ghi hai lần).
//! - **Trả hàng:** hoàn tiền của một lần trả (`orders.refund_completed` có `return_id`) → hoá đơn
//!   điều chỉnh giảm của hoá đơn gốc, dòng là phần hàng trả (cần đối chiếu điều về trả hàng của NĐ
//!   70/2025 trước khi chạy thật).
//! - Mã tham chiếu `orders:<id>:invoice` / `orders:<id>:return:<return_id>:invoice`: gọi lại không
//!   lập hai chứng từ.

use std::sync::Arc;

use autoxide_invoices::api::{Buyer, Document, NewDocument, NewDocumentLine, RenderFormat};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Delivered, EventHandler, Subscription};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{OrderStatus, events as names};
use crate::checkout::Contact;
use crate::entity::{invoice_link, line, order, return_request};
use crate::errors;
use crate::events::{Change, InvoiceLinked};
use crate::policy::{INVOICE_KIND, INVOICE_TIMING};
use crate::service::{OrdersService, now};

/// Thông tin xuất hoá đơn cho doanh nghiệp người mua đưa lúc đặt hay xin sau (`invoice_request`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, serde::Serialize)]
pub(crate) struct InvoiceRequest {
    pub tax_code: Option<String>,
    pub name: Option<String>,
    pub address: Option<String>,
    pub email: Option<String>,
}

/// Mốc của đơn có thể lập hoá đơn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Confirmed,
    Delivered,
    Completed,
}

impl Stage {
    fn timing(self) -> &'static str {
        match self {
            Stage::Confirmed => "on_confirm",
            Stage::Delivered => "on_delivered",
            Stage::Completed => "on_complete",
        }
    }
}

pub(crate) fn subscriptions(service: &Arc<OrdersService>) -> Vec<Subscription> {
    let on = |stage| Arc::new(OnStage { service: service.clone(), stage });
    vec![
        Subscription::new(names::ORDER_CONFIRMED, on(Stage::Confirmed)),
        Subscription::new(names::ORDER_DELIVERED, on(Stage::Delivered)),
        Subscription::new(names::ORDER_COMPLETED, on(Stage::Completed)),
        Subscription::new(names::REFUND_COMPLETED, Arc::new(OnRefund(service.clone()))),
    ]
}

fn order_of(event: &Delivered) -> Option<Uuid> {
    event.payload["after"]["order_id"].as_str().and_then(|id| id.parse().ok())
}

struct OnStage {
    service: Arc<OrdersService>,
    stage: Stage,
}

#[async_trait::async_trait]
impl EventHandler for OnStage {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(id) = order_of(event) else { return Ok(()) };
        if self.service.invoices.is_none() {
            return Ok(());
        }
        let row = self.service.order(&event.tenant, id).await?;
        let timing = self.service.invoice_timing(&event.tenant, &row).await?;
        if timing == self.stage.timing() {
            let issued = self.service.issue_invoice(&event.tenant, id).await;
            return skip_unfixable(issued.map(|_| ()), id);
        }
        Ok(())
    }
}

/// Lỗi khi tự xuất hoá đơn mà thử lại không hết (chưa có dãy số, thiếu thông tin người mua…): ghi log
/// và coi như đã xử lý event. Không thì event của đơn kẹt, mọi bên nghe sau (bảo hành, điểm của khách,
/// nhắc đánh giá…) chờ tới khi hết lượt thử. Chứng từ nháp (nếu đã lập) nằm ở `invoices`; người bán sửa
/// cấu hình rồi xin lại bằng `POST /api/v1/orders/{id}/invoice`. Lỗi thoáng qua (database, nhà cung
/// cấp không trả lời) vẫn trả lỗi để thử lại.
fn skip_unfixable(outcome: Result<(), Error>, order_id: Uuid) -> Result<(), Error> {
    use autoxide_kernel::error::ErrorKind;
    match outcome {
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::NotFound | ErrorKind::Validation | ErrorKind::Conflict
            ) =>
        {
            tracing::warn!(%order_id, code = error.code(), "order invoice not issued automatically");
            Ok(())
        }
        other => other,
    }
}

struct OnRefund(Arc<OrdersService>);

#[async_trait::async_trait]
impl EventHandler for OnRefund {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let (Some(id), Some(return_id)) = (
            order_of(event),
            event.payload["after"]["return_id"].as_str().and_then(|id| id.parse::<Uuid>().ok()),
        ) else {
            return Ok(());
        };
        skip_unfixable(self.0.adjust_invoice(&event.tenant, id, return_id).await, id)
    }
}

/// Tên dòng trên hoá đơn: tên sản phẩm kèm nhãn biến thể.
fn description(row: &line::Model) -> String {
    let label = serde_json::from_str::<Value>(&row.snapshot)
        .ok()
        .and_then(|snapshot| snapshot["variant_label"].as_str().map(str::to_owned))
        .filter(|label| !label.trim().is_empty());
    let text = match label {
        Some(label) => format!("{} - {label}", row.name),
        None => row.name.clone(),
    };
    text.chars().take(500).collect()
}

fn tax_category(row: &line::Model) -> String {
    serde_json::from_str::<Value>(&row.snapshot)
        .ok()
        .and_then(|snapshot| snapshot["tax_category"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "vat.standard".to_owned())
}

/// Dòng hoá đơn cho `quantity` đơn vị của dòng đơn `row` (khoản giảm chia theo số lượng).
fn document_line(row: &line::Model, quantity: i64, currency: Currency) -> NewDocumentLine {
    let mut document_line = NewDocumentLine::new(
        description(row),
        rust_decimal::Decimal::from(quantity),
        Money::new(row.unit_price_minor, currency),
    )
    .tax_category(tax_category(row))
    .item_ref(row.sku.clone());
    let discount = if quantity == row.quantity || row.quantity == 0 {
        row.discount_minor
    } else {
        // Làm tròn xuống: không bao giờ giảm quá phần của số lượng này.
        row.discount_minor * quantity / row.quantity
    };
    if discount > 0 {
        document_line.discount = Some(Money::new(discount, currency));
    }
    document_line
}

impl OrdersService {
    /// `orders.invoice_timing` hiệu lực của đơn.
    pub(crate) async fn invoice_timing(
        &self,
        tenant: &TenantContext,
        row: &order::Model,
    ) -> Result<String, Error> {
        let value = self.setting(tenant, OrgId::from_uuid(row.org_id), &INVOICE_TIMING).await?;
        Ok(value.as_ref().and_then(Value::as_str).unwrap_or("on_delivered").to_owned())
    }

    async fn invoice_link(
        &self,
        tenant: &TenantContext,
        reference: &str,
    ) -> Result<Option<invoice_link::Model>, Error> {
        Ok(invoice_link::Entity::find()
            .filter(invoice_link::Column::Reference.eq(reference))
            .scoped(tenant)
            .one(tenant.database().write())
            .await?)
    }

    /// Lập và phát hành hoá đơn của đơn `id` (gọi lại: trả hoá đơn đã có). `invoices` chưa cài →
    /// `None`.
    pub(crate) async fn issue_invoice(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Option<invoice_link::Model>, Error> {
        let Some(invoices) = self.invoices.clone() else { return Ok(None) };
        let reference = format!("orders:{id}:invoice");
        if let Some(link) = self.invoice_link(tenant, &reference).await?
            && link.status != "draft"
        {
            return Ok(Some(link));
        }
        let row = self.order(tenant, id).await?;
        let currency = Currency::parse(&row.currency)?;
        let lines = line::Entity::find()
            .filter(line::Column::OrderId.eq(id))
            .filter(line::Column::ParentLineId.is_null())
            .scoped(tenant)
            .order_by_asc(line::Column::Id)
            .all(tenant.database().read())
            .await?;
        let mut document_lines: Vec<NewDocumentLine> = lines
            .iter()
            .filter(|row| row.quantity > row.cancelled_quantity)
            .map(|row| document_line(row, row.quantity - row.cancelled_quantity, currency))
            .collect();
        if row.shipping_fee_minor > 0 {
            let option: Value = row
                .shipping_option
                .as_deref()
                .and_then(|raw| serde_json::from_str(raw).ok())
                .unwrap_or(Value::Null);
            let category = option["tax_category"].as_str().unwrap_or("vat.standard").to_owned();
            let mut shipping = NewDocumentLine::new(
                "Shipping fee",
                rust_decimal::Decimal::ONE,
                Money::new(row.shipping_fee_minor, currency),
            )
            .service()
            .tax_category(category);
            if row.shipping_discount_minor > 0 {
                shipping.discount = Some(Money::new(row.shipping_discount_minor, currency));
            }
            document_lines.push(shipping);
        }
        if document_lines.is_empty() {
            return Ok(None);
        }
        let kind = if row.channel == "pos" {
            "cash_register_invoice".to_owned()
        } else {
            self.setting(tenant, OrgId::from_uuid(row.org_id), &INVOICE_KIND)
                .await?
                .as_ref()
                .and_then(Value::as_str)
                .unwrap_or("vat_invoice")
                .to_owned()
        };
        let mut draft = NewDocument::new(
            OrgId::from_uuid(row.org_id),
            kind.clone(),
            currency,
            buyer_of(&row),
            document_lines,
            "orders",
            reference.clone(),
        )
        .source_id(id.to_string())
        .booked_by_source();
        if lines.first().is_none_or(|line| line.tax_inclusive) {
            draft = draft.prices_include_tax();
        }
        let document = match invoices.create_document(tenant, Actor::system(), draft).await? {
            Submitted::Created(document) | Submitted::Replayed(document) => document,
        };
        let link_kind = if kind == "cash_register_invoice" { "cash_register" } else { "sales" };
        self.save_link(tenant, id, None, link_kind, &document, &reference).await?;
        let issued = invoices.issue(tenant, Actor::system(), document.id).await?;
        self.link_issued(tenant, &row, &reference, &issued).await
    }

    /// Ghi (một lần) dòng liên kết ở trạng thái nháp.
    async fn save_link(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
        return_id: Option<Uuid>,
        kind: &str,
        document: &Document,
        reference: &str,
    ) -> Result<(), Error> {
        let at = now();
        let row = invoice_link::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            order_id: Set(order_id),
            return_id: Set(return_id),
            kind: Set(kind.to_owned()),
            document_id: Set(document.id),
            status: Set("draft".to_owned()),
            reference: Set(reference.to_owned()),
            created_at: Set(at),
            updated_at: Set(at),
        };
        // Lập lại (xin lại sau khi phát hành chưa được): dòng của tham chiếu đã có thì thôi. Nêu cột của
        // khoá unique — `on_conflict_do_nothing()` chỉ bỏ qua trùng khoá chính (ID mới, không bao giờ trùng).
        invoice_link::Entity::insert(row)
            .on_conflict_do_nothing_on([
                invoice_link::Column::TenantId,
                invoice_link::Column::Reference,
            ])
            .exec_without_returning(tenant.database().write())
            .await?;
        Ok(())
    }

    /// Chứng từ đã phát hành: đổi trạng thái liên kết và ghi `orders.invoice_linked` (một lần —
    /// chỉ bên đổi được dòng từ `draft` mới ghi event).
    async fn link_issued(
        &self,
        tenant: &TenantContext,
        row: &order::Model,
        reference: &str,
        issued: &Document,
    ) -> Result<Option<invoice_link::Model>, Error> {
        let (database, outbox) = (tenant.database(), &self.outbox);
        let status = issued.status.clone();
        let reference = reference.to_owned();
        let (order_id, org_id, channel) = (row.id, row.org_id, row.channel.clone());
        let number = issued.number;
        let document_id = issued.id;
        database
            .write_with_retry(move || {
                let (status, reference, channel) =
                    (status.clone(), reference.clone(), channel.clone());
                async move {
                    let transaction = database.begin_write().await?;
                    let changed = invoice_link::Entity::update_many()
                        .col_expr(invoice_link::Column::Status, Expr::value(status.clone()))
                        .col_expr(invoice_link::Column::UpdatedAt, Expr::value(now()))
                        .filter(invoice_link::Column::Reference.eq(reference.as_str()))
                        .filter(invoice_link::Column::Status.eq("draft"))
                        .scoped(tenant)
                        .exec(&transaction)
                        .await?;
                    if changed.rows_affected > 0 {
                        let event = InvoiceLinked(Change::after(json!({
                            "order_id": order_id, "tenant_id": tenant.id(), "org_id": org_id,
                            "channel": channel, "document_id": document_id,
                            "invoice_number": number, "status": status, "reference": reference,
                        })));
                        outbox
                            .record(
                                &transaction,
                                tenant,
                                Actor::system(),
                                Some(format!("orders.order:{order_id}")),
                                &event,
                            )
                            .await?;
                    }
                    let link = invoice_link::Entity::find()
                        .filter(invoice_link::Column::Reference.eq(reference.as_str()))
                        .scoped(tenant)
                        .one(&transaction)
                        .await?;
                    transaction.commit().await?;
                    Ok(link)
                }
            })
            .await
    }

    /// Hoá đơn điều chỉnh giảm cho lần trả `return_id` của đơn `id`, khi đơn đã có hoá đơn.
    pub(crate) async fn adjust_invoice(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        return_id: Uuid,
    ) -> Result<(), Error> {
        let Some(invoices) = self.invoices.clone() else { return Ok(()) };
        let original = invoice_link::Entity::find()
            .filter(invoice_link::Column::OrderId.eq(id))
            .filter(invoice_link::Column::ReturnId.is_null())
            .filter(invoice_link::Column::Status.ne("draft"))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        let Some(original) = original else { return Ok(()) };
        let reference = format!("orders:{id}:return:{return_id}:invoice");
        if let Some(link) = self.invoice_link(tenant, &reference).await?
            && link.status != "draft"
        {
            return Ok(());
        }
        let row = self.order(tenant, id).await?;
        let currency = Currency::parse(&row.currency)?;
        let request = return_request::Entity::find_by_id(return_id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(errors::return_not_found)?;
        let returned: Vec<ReturnedLine> =
            serde_json::from_str(&request.lines).map_err(Error::internal)?;
        let lines = line::Entity::find()
            .filter(line::Column::OrderId.eq(id))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        let document_lines: Vec<NewDocumentLine> = returned
            .iter()
            .filter(|item| item.quantity > 0)
            .filter_map(|item| {
                let found = lines.iter().find(|line| line.id == item.line_id)?;
                Some(document_line(found, item.quantity, currency))
            })
            .collect();
        if document_lines.is_empty() {
            return Ok(());
        }
        let document = match invoices
            .adjust(
                tenant,
                Actor::system(),
                original.document_id,
                "decrease",
                document_lines,
                reference.clone(),
            )
            .await?
        {
            Submitted::Created(document) | Submitted::Replayed(document) => document,
        };
        self.save_link(tenant, id, Some(return_id), "adjustment", &document, &reference).await?;
        let issued = invoices.issue(tenant, Actor::system(), document.id).await?;
        self.link_issued(tenant, &row, &reference, &issued).await?;
        Ok(())
    }

    /// Đơn `id` mà người gọi có `orders.manage` ở nút bán của nó.
    pub(crate) async fn managed_order(
        &self,
        tenant: &TenantContext,
        principal: &autoxide_kernel::access::Principal,
        id: Uuid,
    ) -> Result<order::Model, Error> {
        principal.require(&crate::permissions::MANAGE)?;
        let row = self.order(tenant, id).await?;
        let target = autoxide_kernel::access::OrgTarget::new(OrgId::from_uuid(row.org_id), None);
        principal.require_at(tenant.database().read(), &crate::permissions::MANAGE, target).await?;
        Ok(row)
    }

    /// `POST /{id}/invoice` (nhân viên) và `POST /me/orders/{id}/invoice` (người mua): ghi thông tin
    /// xuất hoá đơn và lập ngay khi đơn đã tới mốc (hay chính sách là `on_request`). Đơn đã có hoá
    /// đơn → 409.
    pub(crate) async fn request_invoice(
        &self,
        tenant: &TenantContext,
        row: order::Model,
        request: InvoiceRequest,
    ) -> Result<Option<invoice_link::Model>, Error> {
        if self.invoices.is_none() {
            return Err(errors::invoices_unavailable());
        }
        let mut problems = Vec::new();
        if let Some(code) = request.tax_code.as_deref() {
            let digits: String = code.chars().filter(|c| *c != '-').collect();
            if !(digits.len() == 10 || digits.len() == 12 || digits.len() == 13)
                || !digits.bytes().all(|b| b.is_ascii_digit())
            {
                problems.push(FieldError::new("tax_code", "invalid", "is not a tax code"));
            }
            if request.name.as_deref().is_none_or(|name| name.trim().is_empty()) {
                problems.push(FieldError::new("name", "required", "is required with a tax code"));
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let reference = format!("orders:{}:invoice", row.id);
        // Chứng từ còn nháp (tự xuất chưa được, vd lúc đó chưa có dãy số): xin lại thì phát hành tiếp.
        if self.invoice_link(tenant, &reference).await?.is_some_and(|link| link.status != "draft") {
            return Err(errors::invoice_already_issued());
        }
        let raw = serde_json::to_string(&request).map_err(Error::internal)?;
        order::Entity::update_many()
            .col_expr(order::Column::InvoiceRequest, Expr::value(raw))
            .filter(order::Column::Id.eq(row.id))
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        let status = OrderStatus::parse(&row.status).unwrap_or(OrderStatus::Cancelled);
        let timing = self.invoice_timing(tenant, &row).await?;
        let reached = match timing.as_str() {
            "on_confirm" => !matches!(status, OrderStatus::PendingPayment | OrderStatus::Cancelled),
            "on_complete" => status == OrderStatus::Completed,
            "on_request" => status != OrderStatus::Cancelled,
            _ => matches!(status, OrderStatus::Delivered | OrderStatus::Completed),
        };
        if reached { self.issue_invoice(tenant, row.id).await } else { Ok(None) }
    }

    /// Chứng từ của đơn (hoá đơn, hoá đơn điều chỉnh) cho người mua.
    pub(crate) async fn order_documents(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<(invoice_link::Model, Option<Document>)>, Error> {
        // Theo thứ tự tạo (id UUIDv7 sinh cùng lúc `created_at`), keyset trong database (index
        // `orders_invoice_links_order_page_idx`).
        let select = invoice_link::Entity::find()
            .filter(invoice_link::Column::OrderId.eq(id))
            .filter(invoice_link::Column::Status.ne("draft"))
            .scoped(tenant);
        let links = fetch_page(
            select,
            invoice_link::Column::Id,
            |row| row.id,
            page,
            tenant.database().read(),
        )
        .await?;
        let mut found = Vec::with_capacity(links.data.len());
        for link in links.data {
            found.push(self.with_document(tenant, link).await?);
        }
        Ok(Page { data: found, pagination: links.pagination })
    }

    /// Một chứng từ (đã phát hành, không còn nháp) của đơn `id`, đọc lại từ database.
    pub(crate) async fn order_document(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        link_id: Uuid,
    ) -> Result<Option<(invoice_link::Model, Option<Document>)>, Error> {
        let found = invoice_link::Entity::find_by_id(link_id)
            .filter(invoice_link::Column::OrderId.eq(id))
            .filter(invoice_link::Column::Status.ne("draft"))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        match found {
            Some(link) => Ok(Some(self.with_document(tenant, link).await?)),
            None => Ok(None),
        }
    }

    /// Chứng từ của `invoices` gắn với một dòng liên kết (không có khi chưa bật hoá đơn điện tử).
    async fn with_document(
        &self,
        tenant: &TenantContext,
        link: invoice_link::Model,
    ) -> Result<(invoice_link::Model, Option<Document>), Error> {
        let document = match &self.invoices {
            Some(invoices) => invoices.document(tenant, link.document_id).await?,
            None => None,
        };
        Ok((link, document))
    }

    /// Bản in (PDF) của một chứng từ của đơn `id`.
    pub(crate) async fn order_document_pdf(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        document_id: Uuid,
        locale: &str,
    ) -> Result<autoxide_invoices::api::Rendered, Error> {
        let Some(invoices) = &self.invoices else { return Err(errors::invoices_unavailable()) };
        let linked = invoice_link::Entity::find()
            .filter(invoice_link::Column::OrderId.eq(id))
            .filter(invoice_link::Column::DocumentId.eq(document_id))
            .filter(invoice_link::Column::Status.ne("draft"))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        if linked.is_none() {
            return Err(errors::document_not_found());
        }
        invoices.render(tenant, document_id, RenderFormat::Pdf, locale).await
    }
}

/// Một dòng của yêu cầu trả hàng (cột `lines` của `orders_returns`).
#[derive(Debug, Clone, Deserialize)]
struct ReturnedLine {
    line_id: Uuid,
    quantity: i64,
}

/// Người mua trên hoá đơn: doanh nghiệp khi có mã số thuế, không thì người tiêu dùng.
fn buyer_of(row: &order::Model) -> Buyer {
    let request: Option<InvoiceRequest> =
        row.invoice_request.as_deref().and_then(|raw| serde_json::from_str(raw).ok());
    let contact: Option<Contact> = serde_json::from_str(&row.contact).ok();
    match request {
        Some(InvoiceRequest { tax_code: Some(code), name, address, email }) => {
            let mut buyer = Buyer::business(name.unwrap_or_default(), code);
            if let Some(address) = address {
                buyer = buyer.address(address);
            }
            if let Some(email) = email.or_else(|| contact.and_then(|contact| contact.email)) {
                buyer = buyer.email(email);
            }
            buyer
        }
        _ => {
            let mut buyer = Buyer::consumer(contact.as_ref().map(|contact| contact.name.clone()));
            if let Some(email) = contact.and_then(|contact| contact.email) {
                buyer = buyer.email(email);
            }
            buyer
        }
    }
}
