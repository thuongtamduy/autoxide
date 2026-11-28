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
use rust_decimal::Decimal;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set};
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

/// Số đã chốt của một dòng đơn (hay phí giao) để so với hoá đơn (LG-10, D-20261126-invoice-tax-difference):
/// `item_ref` là SKU (`None`: phí giao); `amounts` là (thuế, tổng) đơn đã tính cho phần còn trên hoá đơn — số chính
/// xác, chưa làm tròn; `None` khi không tính được (tràn số): khi đó luôn báo khác để người vận hành xem.
#[derive(Debug, Clone)]
struct SettledLine {
    item_ref: Option<String>,
    amounts: Option<(Decimal, Decimal)>,
    /// Lệch làm tròn bỏ qua cho dòng này: [`TOLERANCE`], gấp đôi với dòng huỷ một phần (thuế của đơn đã làm tròn cho
    /// cả dòng, hoá đơn làm tròn lại cho phần còn lại — hai lần làm tròn chồng nhau, và thuế nhiều tầng làm tròn từng
    /// tầng).
    tolerance: i64,
}

/// Số đã chốt của đơn để so với hoá đơn bán của nó.
#[derive(Debug, Clone, Default)]
pub(crate) struct Settled {
    lines: Vec<SettledLine>,
    /// Đơn trộn dòng giá gồm thuế và chưa thuế: hoá đơn chỉ có một chế độ giá nên không lập đúng được — luôn báo
    /// khác (kiểm độc lập #575, V5).
    mixed_modes: bool,
}

/// Khoản giảm của `quantity` đơn vị dòng `row` trên hoá đơn: chia theo số lượng, làm tròn xuống (không bao giờ giảm
/// quá phần của số lượng này). Nhân trong `i128`: không tràn.
fn discount_part(row: &line::Model, quantity: i64) -> i64 {
    if quantity == row.quantity || row.quantity == 0 {
        return row.discount_minor;
    }
    let part = i128::from(row.discount_minor) * i128::from(quantity) / i128::from(row.quantity);
    // Phần nhỏ hơn khoản giảm cả dòng nên luôn vừa `i64`.
    i64::try_from(part).unwrap_or(row.discount_minor)
}

/// (thuế, tổng) đơn đã chốt cho phần còn trên hoá đơn của dòng `row`, dựng như dòng hoá đơn (kiểm độc lập #575,
/// M1): tiền trước giảm của số lượng còn lại trừ khoản giảm [`discount_part`] — với giá gồm thuế đó là tổng, với giá
/// chưa thuế là phần trước thuế; thuế chia theo tỉ lệ phần đó trên phần tương ứng của cả dòng. Số chính xác, không
/// làm tròn trước khi so. Dòng không huỷ thì đúng bằng số của đơn.
fn remaining_amounts(row: &line::Model) -> Option<(Decimal, Decimal)> {
    let left = row.quantity - row.cancelled_quantity;
    let (tax, total) = (Decimal::from(row.tax_minor), Decimal::from(row.total_minor));
    if left == row.quantity {
        return Some((tax, total));
    }
    let part = Decimal::from(row.unit_price_minor)
        .checked_mul(Decimal::from(left))?
        .checked_sub(Decimal::from(discount_part(row, left)))?;
    let whole = if row.tax_inclusive { total } else { total.checked_sub(tax)? };
    let part_tax =
        if whole.is_zero() { Decimal::ZERO } else { tax.checked_mul(part)?.checked_div(whole)? };
    let part_total = if row.tax_inclusive { part } else { part.checked_add(part_tax)? };
    Some((part_tax, part_total))
}

/// Dòng đơn còn trên hoá đơn (chưa huỷ hết), như lúc lập hoá đơn.
fn invoiced(line: &line::Model) -> bool {
    line.quantity > line.cancelled_quantity
}

/// Chế độ giá của hoá đơn: theo dòng đầu còn trên hoá đơn (dòng huỷ hết không tính).
fn prices_include_tax(lines: &[line::Model]) -> bool {
    lines.iter().find(|line| invoiced(line)).is_none_or(|line| line.tax_inclusive)
}

/// Thuế phí giao đã chốt: thuế của đơn trừ thuế các dòng (đơn tính thuế phí giao riêng, `pricing::apply_tax`).
fn shipping_tax(row: &order::Model, lines: &[line::Model]) -> i64 {
    (row.tax_minor - lines.iter().map(|line| line.tax_minor).sum::<i64>()).max(0)
}

/// Số đã chốt của đơn `row` (dòng cấp trên `lines`).
pub(crate) fn settled_lines(row: &order::Model, lines: &[line::Model]) -> Settled {
    let mut settled: Vec<SettledLine> = lines
        .iter()
        .filter(|line| invoiced(line))
        .map(|line| SettledLine {
            item_ref: Some(line.sku.clone()),
            amounts: remaining_amounts(line),
            tolerance: if line.cancelled_quantity > 0 { 2 * TOLERANCE } else { TOLERANCE },
        })
        .collect();
    let shipping_gross = row.shipping_fee_minor - row.shipping_discount_minor;
    if shipping_gross > 0 {
        let tax = Decimal::from(shipping_tax(row, lines));
        settled.push(SettledLine {
            item_ref: None,
            amounts: Some((tax, Decimal::from(shipping_gross))),
            tolerance: TOLERANCE,
        });
    }
    let mut modes = lines.iter().filter(|line| invoiced(line)).map(|line| line.tax_inclusive);
    let mixed_modes = modes.next().is_some_and(|first| modes.any(|mode| mode != first));
    Settled { lines: settled, mixed_modes }
}

/// [`settled_lines`] của đơn `id`, đọc từ database.
async fn settled_amounts(
    read: &impl ConnectionTrait,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<Settled, Error> {
    let Some(row) = order::Entity::find_by_id(id).scoped(tenant).one(read).await? else {
        return Ok(Settled::default());
    };
    // Cùng phép lọc với lúc lập hoá đơn: chỉ dòng cấp trên.
    let lines = line::Entity::find()
        .filter(line::Column::OrderId.eq(id))
        .filter(line::Column::ParentLineId.is_null())
        .scoped(tenant)
        .all(read)
        .await?;
    Ok(settled_lines(&row, &lines))
}

/// Loại dòng liên kết của một hoá đơn bán theo loại chứng từ.
fn link_kind_of(kind: &str) -> String {
    if kind == "cash_register_invoice" { "cash_register" } else { "sales" }.to_owned()
}

/// Hoá đơn bán (không phải điều chỉnh) khác số đơn đã chốt (LG-10): đơn trộn hai chế độ giá; thuế của một dòng
/// hoá đơn lệch quá [`TOLERANCE`] so với thuế đơn đã tính cho dòng đó (khớp theo SKU, phí giao: dòng không có mã —
/// so tiền thuế, không so thuế suất: hàng chịu nhiều tầng thuế như TTĐB rồi GTGT có thuế dòng khác `trước thuế ×
/// suất GTGT`, kiểm độc lập #575, V4); hay tổng hoá đơn lệch tổng đơn quá [`TOLERANCE`] mỗi dòng. Ngưỡng cho phép
/// hoá đơn làm tròn thuế theo cách của nó (vd phí giao gồm thuế ghi trên hoá đơn giá chưa thuế lệch tối đa 1 đồng).
pub(crate) fn differs_from_order(
    settled: &Settled,
    link_kind: &str,
    document: Option<&Document>,
) -> bool {
    let Some(document) = document.filter(|_| link_kind != "adjustment") else { return false };
    if settled.mixed_modes {
        return true;
    }
    // Một dòng không tính được (tràn số): không so được — báo để người vận hành xem.
    let Some(amounts) =
        settled.lines.iter().map(|line| line.amounts).collect::<Option<Vec<(Decimal, Decimal)>>>()
    else {
        return true;
    };
    let line_differs = document.lines.iter().any(|invoice_line| {
        let mut candidates = settled
            .lines
            .iter()
            .zip(&amounts)
            .filter(|(line, _)| line.item_ref == invoice_line.item_ref)
            .peekable();
        candidates.peek().is_some()
            && candidates.all(|(line, (tax, _))| {
                (Decimal::from(invoice_line.tax.minor()) - *tax).abs()
                    > Decimal::from(line.tolerance)
            })
    });
    let expected =
        amounts.iter().try_fold(Decimal::ZERO, |sum, (_, total)| sum.checked_add(*total));
    let Some(expected) = expected else { return true };
    let allowed = Decimal::from(settled.lines.iter().map(|line| line.tolerance).sum::<i64>());
    line_differs
        || (!amounts.is_empty()
            && (Decimal::from(document.total.minor()) - expected).abs() > allowed)
}

/// Lệch do làm tròn được bỏ qua khi so hoá đơn với đơn: 1 đơn vị tiền nhỏ nhất mỗi dòng.
const TOLERANCE: i64 = 1;

/// Dòng hoá đơn cho `quantity` đơn vị của dòng đơn `row` (khoản giảm chia theo số lượng).
fn document_line(row: &line::Model, quantity: i64, currency: Currency) -> NewDocumentLine {
    let mut document_line = NewDocumentLine::new(
        description(row),
        rust_decimal::Decimal::from(quantity),
        Money::new(row.unit_price_minor, currency),
    )
    .tax_category(tax_category(row))
    .item_ref(row.sku.clone());
    let discount = discount_part(row, quantity);
    if discount > 0 {
        document_line.discount = Some(Money::new(discount, currency));
    }
    document_line
}

/// Hoá đơn của một đơn theo lời khuyên của engine thuế (LG-7).
#[derive(Debug, PartialEq, Eq)]
enum InvoiceChoice {
    /// Luật không buộc hoá đơn điện tử: không tự lập.
    NotRequired,
    /// Loại luật đặt, hay mặc định khi bán tại quầy.
    Kind(String),
    /// Người bán chọn: cài đặt `orders.invoice_kind`.
    Configured,
}

/// `advice`: lời khuyên của engine thuế (`None`: chưa cài `tax` hay người bán chưa có hồ sơ thuế — như trước:
/// bán tại quầy → hoá đơn máy tính tiền, còn lại theo cài đặt).
///
/// `requested`: người mua đã xin hoá đơn (`invoice_request` của đơn) — luật không buộc thì vẫn lập theo yêu cầu,
/// chỉ thôi TỰ lập. `configured`: người bán tự đặt `orders.invoice_kind` (không tính giá trị mặc định) — lựa chọn
/// đó thắng loại engine gợi ý cho người bán không theo phương pháp khấu trừ (ghi cảnh báo), không âm thầm đổi.
fn invoice_choice(
    advice: Option<autoxide_tax::api::InvoiceAdvice>,
    at_counter: bool,
    requested: bool,
    configured: Option<&str>,
) -> InvoiceChoice {
    if !requested && advice.as_ref().is_some_and(|advice| !advice.einvoice_required) {
        return InvoiceChoice::NotRequired;
    }
    match advice.and_then(|advice| advice.kind) {
        Some(kind) if kind == "cash_register_invoice" => InvoiceChoice::Kind(kind),
        Some(kind) => match configured {
            Some(chosen) if chosen != kind => {
                tracing::warn!(
                    suggested = %kind,
                    chosen,
                    "orders.invoice_kind differs from the invoice the tax data suggests; the setting wins"
                );
                InvoiceChoice::Configured
            }
            _ => InvoiceChoice::Kind(kind),
        },
        None if at_counter => InvoiceChoice::Kind("cash_register_invoice".to_owned()),
        None => InvoiceChoice::Configured,
    }
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
        // Hoá đơn có một chế độ giá cho mọi dòng (theo dòng đầu); đơn luôn coi phí giao là giá đã gồm thuế.
        let inclusive = prices_include_tax(&lines);
        if row.shipping_fee_minor > 0 {
            let option: Value = row
                .shipping_option
                .as_deref()
                .and_then(|raw| serde_json::from_str(raw).ok())
                .unwrap_or(Value::Null);
            let category = option["tax_category"].as_str().unwrap_or("vat.standard").to_owned();
            // Hoá đơn giá chưa thuế: ghi phí giao bằng phần trước thuế mà đơn đã tách (đã trừ giảm phí giao),
            // để người mua không bị cộng thuế lần hai lên khoản đã gồm thuế (kiểm độc lập #575, V2).
            let (price, discount) = if inclusive {
                (row.shipping_fee_minor, row.shipping_discount_minor)
            } else {
                let gross = row.shipping_fee_minor - row.shipping_discount_minor;
                (gross - shipping_tax(&row, &lines), 0)
            };
            let mut shipping = NewDocumentLine::new(
                "Shipping fee",
                rust_decimal::Decimal::ONE,
                Money::new(price, currency),
            )
            .service()
            .tax_category(category);
            if discount > 0 {
                shipping.discount = Some(Money::new(discount, currency));
            }
            if price > 0 {
                document_lines.push(shipping);
            }
        }
        if document_lines.is_empty() {
            return Ok(None);
        }
        // Loại hoá đơn theo dữ liệu thuế của người bán (LG-7, D-20261128-tax-pack-vn-decree-68): engine thuế nói
        // có bắt buộc hoá đơn điện tử không và loại nào luật đặt; không có hồ sơ thuế thì như trước (bán tại quầy →
        // hoá đơn máy tính tiền, còn lại theo `orders.invoice_kind`).
        let org = OrgId::from_uuid(row.org_id);
        let at_counter = row.channel == "pos";
        let advice = match &self.sources.tax {
            Some(engine) => {
                let on =
                    engine.tax_date(tenant, org, autoxide_kernel::time::Timestamp::now()).await?;
                engine.invoice_advice(tenant, org, on, at_counter).await?
            }
            None => None,
        };
        // Giá trị người bán tự đặt (không tính mặc định của cài đặt).
        let configured = match &self.orgs {
            Some(orgs) => orgs
                .setting(tenant, org, INVOICE_KIND.key())
                .await?
                .and_then(|found| found.value)
                .and_then(|value| value.as_str().map(str::to_owned)),
            None => None,
        };
        let requested = row.invoice_request.is_some();
        let kind = match invoice_choice(advice, at_counter, requested, configured.as_deref()) {
            InvoiceChoice::NotRequired => {
                // Hộ kinh doanh dưới ngưỡng, người mua chưa xin: luật không buộc hoá đơn điện tử, không tự lập.
                // Người mua xin (`POST …/invoice`) thì `invoice_request` có giá trị và hoá đơn vẫn được lập.
                tracing::info!(order_id = %id, "no e-invoice required for this seller; none issued automatically");
                return Ok(None);
            }
            InvoiceChoice::Kind(kind) => kind,
            InvoiceChoice::Configured => self
                .setting(tenant, org, &INVOICE_KIND)
                .await?
                .as_ref()
                .and_then(Value::as_str)
                .unwrap_or("vat_invoice")
                .to_owned(),
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
        if inclusive {
            draft = draft.prices_include_tax();
        }
        let document = match invoices.create_document(tenant, Actor::system(), draft).await? {
            Submitted::Created(document) | Submitted::Replayed(document) => document,
        };
        let link_kind = link_kind_of(&kind);
        self.save_link(tenant, id, None, &link_kind, &document, &reference).await?;
        let issued = invoices.issue(tenant, Actor::system(), document.id).await?;
        // Hoá đơn tính thuế theo ngày lập (thời điểm xác định thuế); đơn đã chốt thuế theo ngày đặt. Qua mốc
        // đổi thuế suất thì hai số khác nhau: không sửa lặng lẽ, báo cho người dùng (LG-10).
        if differs_from_order(&settled_lines(&row, &lines), &link_kind, Some(&issued)) {
            tracing::warn!(
                order_id = %id,
                document_id = %issued.id,
                order_tax = row.tax_minor,
                invoice_tax = issued.tax.minor(),
                "the invoice differs from the tax or total settled on the order"
            );
        }
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
    ) -> Result<(Page<(invoice_link::Model, Option<Document>)>, Settled), Error> {
        let read = tenant.database().read();
        // Theo thứ tự tạo (id UUIDv7 sinh cùng lúc `created_at`), keyset trong database (index
        // `orders_invoice_links_order_page_idx`).
        let select = invoice_link::Entity::find()
            .filter(invoice_link::Column::OrderId.eq(id))
            .filter(invoice_link::Column::Status.ne("draft"))
            .scoped(tenant);
        let links = fetch_page(select, invoice_link::Column::Id, |row| row.id, page, read).await?;
        let mut found = Vec::with_capacity(links.data.len());
        for link in links.data {
            found.push(self.with_document(tenant, link).await?);
        }
        let settled = settled_amounts(read, tenant, id).await?;
        Ok((Page { data: found, pagination: links.pagination }, settled))
    }

    /// Một chứng từ (đã phát hành, không còn nháp) của đơn `id`, đọc lại từ database.
    pub(crate) async fn order_document(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        link_id: Uuid,
    ) -> Result<Option<(invoice_link::Model, Option<Document>, Settled)>, Error> {
        let read = tenant.database().read();
        let found = invoice_link::Entity::find_by_id(link_id)
            .filter(invoice_link::Column::OrderId.eq(id))
            .filter(invoice_link::Column::Status.ne("draft"))
            .scoped(tenant)
            .one(read)
            .await?;
        match found {
            Some(link) => {
                let (link, document) = self.with_document(tenant, link).await?;
                Ok(Some((link, document, settled_amounts(read, tenant, id).await?)))
            }
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

#[cfg(test)]
mod choice_tests {
    use super::{InvoiceChoice, invoice_choice};
    use autoxide_tax::api::InvoiceAdvice;

    /// Engine thuế quyết khi người bán có hồ sơ thuế (LG-7); không thì như trước theo kênh.
    #[test]
    fn loai_hoa_don_theo_loi_khuyen_cua_engine_thue() {
        let advice = |required, kind: Option<&str>| {
            Some(InvoiceAdvice::new(required, kind.map(str::to_owned), Vec::new()))
        };
        let kind = |name: &str| InvoiceChoice::Kind(name.to_owned());
        // Hộ dưới ngưỡng NĐ 68/2026: không tự lập, kể cả tại quầy…
        assert_eq!(
            invoice_choice(advice(false, None), true, false, None),
            InvoiceChoice::NotRequired
        );
        // …nhưng người mua đã xin hoá đơn thì vẫn lập (kiểm độc lập #620).
        assert_eq!(
            invoice_choice(advice(false, Some("sales_invoice")), false, true, None),
            kind("sales_invoice")
        );
        assert_eq!(
            invoice_choice(advice(false, None), true, true, None),
            kind("cash_register_invoice")
        );
        // Hộ kinh doanh bán online: hoá đơn bán hàng, không phải hoá đơn GTGT.
        assert_eq!(
            invoice_choice(advice(true, Some("sales_invoice")), false, false, None),
            kind("sales_invoice")
        );
        // Người bán đã tự chọn loại khác: lựa chọn đó thắng, không âm thầm đổi.
        assert_eq!(
            invoice_choice(advice(true, Some("sales_invoice")), false, false, Some("vat_invoice")),
            InvoiceChoice::Configured
        );
        assert_eq!(
            invoice_choice(
                advice(true, Some("cash_register_invoice")),
                true,
                false,
                Some("vat_invoice")
            ),
            kind("cash_register_invoice")
        );
        // Người bán khấu trừ: tự chọn; tại quầy giữ mặc định máy tính tiền.
        assert_eq!(
            invoice_choice(advice(true, None), false, false, None),
            InvoiceChoice::Configured
        );
        assert_eq!(
            invoice_choice(advice(true, None), true, false, None),
            kind("cash_register_invoice")
        );
        assert_eq!(invoice_choice(None, true, false, None), kind("cash_register_invoice"));
        assert_eq!(invoice_choice(None, false, false, None), InvoiceChoice::Configured);
    }
}
