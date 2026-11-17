//! Hợp đồng của module `invoices` cho module khác: lập chứng từ (nháp), phát hành (cấp số không nhảy
//! số), đọc, huỷ. Module khác lấy lúc khởi động: `context.require::<dyn Invoices>()` (đăng ký SAU
//! `invoices`).
//!
//! `invoices` tính thuế qua `dyn TaxEngine` và ghi sổ qua `dyn Ledger` khi phát hành chứng từ bán
//! hàng (Nợ `receivable.customer` / Có `revenue.sales`, `tax.vat_output`): module gọi không tự đánh
//! số, không tự tính thuế, không tự ghi sổ cho chứng từ.

use autoxide_kernel::decimal::DecimalString;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Loại chứng từ. Bốn loại đầu là chứng từ bán hàng (tính thuế, ghi sổ khi phát hành).
pub const KINDS: [&str; 9] = [
    "vat_invoice",
    "sales_invoice",
    "cash_register_invoice",
    "retail_receipt",
    "delivery_note",
    "goods_receipt",
    "cash_receipt",
    "cash_payment",
    "withholding_certificate",
];

/// Chứng từ bán hàng: tính thuế qua `TaxEngine`, ghi doanh thu khi phát hành.
pub fn is_sales_kind(kind: &str) -> bool {
    matches!(kind, "vat_invoice" | "sales_invoice" | "cash_register_invoice" | "retail_receipt")
}

/// The buyer printed on a document (a copy taken when the document is written).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = InvoicesBuyer)]
pub struct Buyer {
    /// `consumer` or `business`.
    #[serde(default = "consumer")]
    pub kind: String,
    /// Up to 200 characters.
    pub name: Option<String>,
    /// 10 or 13 digits (`0101234567-001`), or a 12-digit personal id used as tax code.
    pub tax_code: Option<String>,
    /// One line, up to 400 characters.
    pub address: Option<String>,
    pub email: Option<String>,
}

fn consumer() -> String {
    "consumer".to_owned()
}

impl Buyer {
    pub fn consumer(name: Option<String>) -> Self {
        Buyer { kind: consumer(), name, ..Buyer::default() }
    }

    pub fn business(name: impl Into<String>, tax_code: impl Into<String>) -> Self {
        Buyer {
            kind: "business".to_owned(),
            name: Some(name.into()),
            tax_code: Some(tax_code.into()),
            ..Buyer::default()
        }
    }

    #[must_use]
    pub fn address(mut self, address: impl Into<String>) -> Self {
        self.address = Some(address.into());
        self
    }

    #[must_use]
    pub fn email(mut self, email: impl Into<String>) -> Self {
        self.email = Some(email.into());
        self
    }
}

/// One line of a document to write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = InvoicesLineInput)]
#[non_exhaustive]
pub struct NewDocumentLine {
    /// `goods`, `service` or `note` (text only, no amount).
    #[serde(default = "goods")]
    pub kind: String,
    /// Your id of the item (product code…).
    pub item_ref: Option<String>,
    /// 1 to 500 characters.
    pub description: String,
    /// Unit, e.g. `pcs`, `month`.
    pub unit: Option<String>,
    /// Quantity, as a decimal string; default `1`.
    pub quantity: Option<DecimalString>,
    pub unit_price: Option<Money>,
    /// Discount on the line.
    pub discount: Option<Money>,
    /// Tax category (`GET /api/v1/tax/categories`), required on sales documents.
    pub tax_category: Option<String>,
}

fn goods() -> String {
    "goods".to_owned()
}

impl NewDocumentLine {
    pub fn new(
        description: impl Into<String>,
        quantity: rust_decimal::Decimal,
        unit_price: Money,
    ) -> Self {
        NewDocumentLine {
            kind: goods(),
            item_ref: None,
            description: description.into(),
            unit: None,
            quantity: Some(DecimalString(quantity)),
            unit_price: Some(unit_price),
            discount: None,
            tax_category: None,
        }
    }

    #[must_use]
    pub fn service(mut self) -> Self {
        self.kind = "service".to_owned();
        self
    }

    #[must_use]
    pub fn tax_category(mut self, category: impl Into<String>) -> Self {
        self.tax_category = Some(category.into());
        self
    }

    #[must_use]
    pub fn unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    #[must_use]
    pub fn item_ref(mut self, item: impl Into<String>) -> Self {
        self.item_ref = Some(item.into());
        self
    }
}

/// Một chứng từ nháp cần lập. `reference` bắt buộc (D-184): duy nhất theo (khách, `source_module`).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NewDocument {
    /// Nút lập chứng từ.
    pub org: OrgId,
    /// Người bán được lập thay (cần uỷ nhiệm còn hiệu lực).
    pub on_behalf_of: Option<OrgId>,
    pub kind: String,
    /// Ký hiệu; bỏ trống thì dùng ký hiệu đang dùng duy nhất của nút cho loại này và năm phát hành.
    pub series: Option<Uuid>,
    /// Ngày lập dự kiến; bỏ trống thì là ngày phát hành.
    pub issue_date: Option<NaiveDate>,
    /// Giá trên dòng đã gồm thuế (`true`) hay chưa (`false`).
    pub prices_include_tax: bool,
    pub currency: Currency,
    pub buyer: Buyer,
    pub lines: Vec<NewDocumentLine>,
    pub source_module: String,
    pub source_id: Option<String>,
    pub reference: String,
}

impl NewDocument {
    pub fn new(
        org: OrgId,
        kind: impl Into<String>,
        currency: Currency,
        buyer: Buyer,
        lines: Vec<NewDocumentLine>,
        source_module: impl Into<String>,
        reference: impl Into<String>,
    ) -> Self {
        NewDocument {
            org,
            on_behalf_of: None,
            kind: kind.into(),
            series: None,
            issue_date: None,
            prices_include_tax: false,
            currency,
            buyer,
            lines,
            source_module: source_module.into(),
            source_id: None,
            reference: reference.into(),
        }
    }

    #[must_use]
    pub fn source_id(mut self, id: impl Into<String>) -> Self {
        self.source_id = Some(id.into());
        self
    }

    #[must_use]
    pub fn series(mut self, series: Uuid) -> Self {
        self.series = Some(series);
        self
    }

    #[must_use]
    pub fn on_behalf_of(mut self, seller: OrgId) -> Self {
        self.on_behalf_of = Some(seller);
        self
    }

    #[must_use]
    pub fn prices_include_tax(mut self) -> Self {
        self.prices_include_tax = true;
        self
    }
}

/// One line of a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = InvoicesLine)]
pub struct DocumentLine {
    pub line_no: i32,
    pub kind: String,
    pub item_ref: Option<String>,
    pub description: String,
    pub unit: Option<String>,
    pub quantity: String,
    pub unit_price: Money,
    pub discount: Money,
    /// Amount before taxes.
    pub net: Money,
    pub tax_category: Option<String>,
    /// VAT rate applied, e.g. `"8"`; `null` when not taxed.
    pub rate_percent: Option<String>,
    pub tax: Money,
    pub total: Money,
}

/// An accounting document (invoice, receipt, note…).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = InvoicesDocument)]
pub struct Document {
    pub id: Uuid,
    /// The unit that writes the document.
    pub org_id: Uuid,
    /// The seller the document is written for, under a mandate.
    pub on_behalf_of_org_id: Option<Uuid>,
    pub kind: String,
    /// `adjustment` or `replacement` of `related_document_id`.
    pub relation: Option<String>,
    pub related_document_id: Option<Uuid>,
    /// For an adjustment: `increase` or `decrease`.
    pub direction: String,
    pub series_id: Option<Uuid>,
    /// Series symbol, e.g. `1C26TAA`.
    pub symbol: Option<String>,
    /// Number in the series; `null` while a draft.
    pub number: Option<i64>,
    /// `draft`, `issued`, `submitting`, `accepted`, `rejected`, `cancelled`, `replaced` or `adjusted`.
    pub status: String,
    pub issue_date: Option<NaiveDate>,
    pub prices_include_tax: bool,
    pub currency: String,
    pub subtotal: Money,
    pub tax: Money,
    pub total: Money,
    pub buyer: Buyer,
    /// Copy of the seller taken at issue (legal name, tax code, address).
    #[schema(value_type = Object)]
    pub seller: serde_json::Value,
    pub lines: Vec<DocumentLine>,
    /// E-invoice: provider, its reference and number, tax authority code, lookup code.
    pub einvoice_provider: Option<String>,
    pub provider_ref: Option<String>,
    pub provider_number: Option<i64>,
    pub authority_code: Option<String>,
    pub lookup_code: Option<String>,
    pub rejection_reason: Option<String>,
    /// Kept at least until this day (Accounting Law 88/2015, Article 41).
    pub retention_until: Option<NaiveDate>,
    /// For a document in a foreign currency: the rate used at issue (1 `currency` = `exchange_rate`
    /// of the book currency), kept as issued; `null` in the book currency or while a draft.
    pub exchange_rate: Option<String>,
    /// Id of the ledger exchange rate used.
    pub exchange_rate_id: Option<Uuid>,
    /// The total in the book currency, converted with `exchange_rate`.
    pub base_total: Option<Money>,
    pub source_module: String,
    pub source_id: Option<String>,
    pub reference: String,
    pub issued_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Định dạng bản in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderFormat {
    /// HTML dựng từ mẫu của khách (hay mẫu mặc định).
    Html,
    /// PDF bố cục cố định từ cùng view model.
    Pdf,
}

/// Một bản in: kiểu nội dung, tên file gợi ý, nội dung.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Rendered {
    pub content_type: &'static str,
    pub file_name: String,
    pub body: Vec<u8>,
}

impl Rendered {
    pub fn new(content_type: &'static str, file_name: String, body: Vec<u8>) -> Self {
        Rendered { content_type, file_name, body }
    }
}

/// Lập, phát hành và đọc chứng từ.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait Invoices: Send + Sync {
    /// Lập chứng từ nháp (chưa có số), thuế tính sẵn. Cùng `reference` + cùng nội dung → `Replayed`.
    async fn create_document(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        draft: NewDocument,
    ) -> Result<Submitted<Document>, Error>;

    /// Phát hành: cấp số tiếp theo của ký hiệu (không nhảy số), gửi hoá đơn điện tử nếu ký hiệu có
    /// nhà cung cấp, ghi sổ. Gọi lại → cùng số.
    async fn issue(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        document: Uuid,
    ) -> Result<Document, Error>;

    /// Một chứng từ, hoặc `None`.
    async fn document(&self, tenant: &TenantContext, id: Uuid) -> Result<Option<Document>, Error>;

    /// Huỷ chứng từ đã phát hành (thông báo sai sót với nhà cung cấp, đảo bút toán).
    async fn cancel(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        document: Uuid,
        reason: String,
    ) -> Result<Document, Error>;

    /// Bản in của chứng từ (D-353): module khác (đơn hàng, kho) lập phiếu giao hàng, phiếu nhập,
    /// phiếu thu… qua [`Invoices::create_document`] rồi in qua đây — một chỗ giữ mẫu in theo khách.
    /// `locale`: `vi` hay `en`. Không có chứng từ: `invoices.document_not_found`.
    async fn render(
        &self,
        tenant: &TenantContext,
        document: Uuid,
        format: RenderFormat,
        locale: &str,
    ) -> Result<Rendered, Error>;

    /// Gói lưu trữ kế toán của khách (sổ nhật ký chung, chứng từ, `manifest.json` có SHA-256 từng
    /// file) vào nơi lưu chung, ngoài thư mục của khách — còn lại sau khi xoá hẳn khách. Trả ID bản lưu.
    async fn archive(&self, tenant: &TenantContext, actor: Actor) -> Result<Uuid, Error>;
}
