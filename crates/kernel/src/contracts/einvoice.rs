//! Hoá đơn điện tử qua nhà cung cấp (bước 36): trait chung để module `invoices` gửi hoá đơn tới nhà
//! cung cấp dịch vụ hoá đơn điện tử (chuyển cơ quan thuế) mà không phụ thuộc nhà cung cấp nào.
//!
//! Bộ nối của một nhà cung cấp là một module riêng, chỉ phụ thuộc kernel, đăng ký TRƯỚC `invoices`,
//! góp bằng `context.contribute::<dyn EInvoiceProvider>(…)`. Mọi hàm nhận khách (`TenantId`): bộ nối
//! tự đọc thông tin kết nối của khách đó. Test dùng `FakeEInvoiceProvider` của testkit.

use chrono::NaiveDate;
use uuid::Uuid;

use crate::error::{Error, ErrorKind};
use crate::money::{Currency, Money};
use crate::tenancy::TenantId;

/// Loại hoá đơn điện tử.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EInvoiceKind {
    /// Hoá đơn giá trị gia tăng.
    Vat,
    /// Hoá đơn bán hàng.
    Sales,
    /// Hoá đơn khởi tạo từ máy tính tiền.
    CashRegister,
    /// Chứng từ khấu trừ thuế thu nhập cá nhân.
    WithholdingCertificate,
}

impl EInvoiceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EInvoiceKind::Vat => "vat",
            EInvoiceKind::Sales => "sales",
            EInvoiceKind::CashRegister => "cash_register",
            EInvoiceKind::WithholdingCertificate => "withholding_certificate",
        }
    }
}

/// Những gì một nhà cung cấp làm được.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct EInvoiceCapabilities {
    pub kinds: Vec<EInvoiceKind>,
    /// Nhà cung cấp tự cấp số hoá đơn (số nội bộ vẫn cấp; số của nhà cung cấp lưu khi chấp nhận).
    pub assigns_numbers: bool,
    pub adjust: bool,
    pub replace: bool,
    pub cancel: bool,
}

impl EInvoiceCapabilities {
    pub fn new(kinds: &[EInvoiceKind]) -> Self {
        EInvoiceCapabilities { kinds: kinds.to_vec(), ..EInvoiceCapabilities::default() }
    }

    /// Điều chỉnh, thay thế, huỷ đều được.
    #[must_use]
    pub fn with_corrections(mut self) -> Self {
        self.adjust = true;
        self.replace = true;
        self.cancel = true;
        self
    }

    #[must_use]
    pub fn assigning_numbers(mut self) -> Self {
        self.assigns_numbers = true;
        self
    }
}

/// Một bên trên hoá đơn (ảnh chụp lúc lập).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Party {
    pub name: String,
    pub tax_code: Option<String>,
    /// Địa chỉ dạng một dòng chữ.
    pub address: Option<String>,
    pub email: Option<String>,
}

impl Party {
    pub fn new(name: impl Into<String>) -> Self {
        Party { name: name.into(), ..Party::default() }
    }

    #[must_use]
    pub fn tax_code(mut self, code: Option<String>) -> Self {
        self.tax_code = code;
        self
    }

    #[must_use]
    pub fn address(mut self, address: Option<String>) -> Self {
        self.address = address;
        self
    }

    #[must_use]
    pub fn email(mut self, email: Option<String>) -> Self {
        self.email = email;
        self
    }
}

/// Một dòng hoá đơn.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct EInvoiceLine {
    pub line_no: i32,
    pub description: String,
    pub unit: Option<String>,
    /// Số lượng, chữ thập phân chuẩn hoá (vd `"2"`, `"1.5"`).
    pub quantity: String,
    pub unit_price: Money,
    /// Tiền trước thuế (sau chiết khấu).
    pub net: Money,
    /// Thuế suất (vd `"8"`), `None` khi không chịu thuế.
    pub rate_percent: Option<String>,
    pub tax: Money,
    pub total: Money,
}

impl EInvoiceLine {
    /// Dòng `line_no`: số lượng (chữ thập phân), đơn giá, tiền trước thuế, thuế.
    pub fn new(
        line_no: i32,
        description: impl Into<String>,
        quantity: impl Into<String>,
        unit_price: Money,
        net: Money,
        tax: Money,
    ) -> Self {
        let total = net.checked_add(tax).unwrap_or(net);
        EInvoiceLine {
            line_no,
            description: description.into(),
            unit: None,
            quantity: quantity.into(),
            unit_price,
            net,
            rate_percent: None,
            tax,
            total,
        }
    }

    #[must_use]
    pub fn unit(mut self, unit: Option<String>) -> Self {
        self.unit = unit;
        self
    }

    #[must_use]
    pub fn rate_percent(mut self, rate: Option<String>) -> Self {
        self.rate_percent = rate;
        self
    }
}

/// Tổng theo thuế suất.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct EInvoiceTaxTotal {
    pub rate_percent: Option<String>,
    pub net: Money,
    pub tax: Money,
}

impl EInvoiceTaxTotal {
    pub fn new(rate_percent: Option<String>, net: Money, tax: Money) -> Self {
        EInvoiceTaxTotal { rate_percent, net, tax }
    }
}

/// Quan hệ với hoá đơn đã phát hành (NĐ 123/2020 Điều 19, sửa bởi NĐ 70/2025 — cần đối chiếu).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    Adjustment,
    Replacement,
}

/// Hoá đơn liên quan (bị điều chỉnh / bị thay thế).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RelatedInvoice {
    pub relation: Relation,
    pub provider_ref: Option<String>,
    pub series: String,
    pub number: i64,
    pub issue_date: NaiveDate,
}

impl RelatedInvoice {
    pub fn new(
        relation: Relation,
        series: impl Into<String>,
        number: i64,
        issue_date: NaiveDate,
    ) -> Self {
        RelatedInvoice { relation, provider_ref: None, series: series.into(), number, issue_date }
    }

    #[must_use]
    pub fn provider_ref(mut self, reference: Option<String>) -> Self {
        self.provider_ref = reference;
        self
    }
}

/// Một hoá đơn gửi nhà cung cấp.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct EInvoice {
    /// ID chứng từ trong hệ thống: nhà cung cấp chống trùng theo mã này (gửi lại cùng hoá đơn).
    pub document_id: Uuid,
    pub kind: EInvoiceKind,
    pub seller: Party,
    /// Người bán được lập thay khi có uỷ nhiệm (bên lập là `seller`).
    pub on_behalf_of: Option<Party>,
    pub buyer: Party,
    /// Ký hiệu, vd `1C26TAA`.
    pub series: String,
    /// Số nội bộ; nhà cung cấp tự cấp số thì có thể bỏ qua.
    pub number: Option<i64>,
    pub issue_date: NaiveDate,
    pub currency: Currency,
    /// Tỉ giá sang đồng Việt Nam (chữ thập phân), khi khác VND.
    pub exchange_rate: Option<String>,
    pub lines: Vec<EInvoiceLine>,
    pub totals: Vec<EInvoiceTaxTotal>,
    pub related: Option<RelatedInvoice>,
    pub payment_method: String,
}

impl EInvoice {
    /// Hoá đơn chưa có ký hiệu, dòng: thêm bằng [`series`](Self::series), [`lines`](Self::lines).
    pub fn new(
        document_id: Uuid,
        kind: EInvoiceKind,
        seller: Party,
        buyer: Party,
        issue_date: NaiveDate,
        currency: Currency,
    ) -> Self {
        EInvoice {
            document_id,
            kind,
            seller,
            on_behalf_of: None,
            buyer,
            series: String::new(),
            number: None,
            issue_date,
            currency,
            exchange_rate: None,
            lines: Vec::new(),
            totals: Vec::new(),
            related: None,
            payment_method: "TM/CK".to_owned(),
        }
    }

    #[must_use]
    pub fn series(mut self, series: impl Into<String>, number: Option<i64>) -> Self {
        self.series = series.into();
        self.number = number;
        self
    }

    #[must_use]
    pub fn lines(mut self, lines: Vec<EInvoiceLine>, totals: Vec<EInvoiceTaxTotal>) -> Self {
        self.lines = lines;
        self.totals = totals;
        self
    }

    #[must_use]
    pub fn on_behalf_of(mut self, party: Option<Party>) -> Self {
        self.on_behalf_of = party;
        self
    }

    #[must_use]
    pub fn related(mut self, related: Option<RelatedInvoice>) -> Self {
        self.related = related;
        self
    }
}

/// Hoá đơn nhà cung cấp đã nhận.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct IssuedEInvoice {
    pub provider_ref: String,
    /// Số do nhà cung cấp cấp (khi `assigns_numbers`).
    pub number: Option<i64>,
    /// Mã của cơ quan thuế, nếu có ngay.
    pub authority_code: Option<String>,
    /// Mã tra cứu cho người mua.
    pub lookup_code: Option<String>,
}

impl IssuedEInvoice {
    pub fn new(provider_ref: impl Into<String>) -> Self {
        IssuedEInvoice {
            provider_ref: provider_ref.into(),
            number: None,
            authority_code: None,
            lookup_code: None,
        }
    }

    #[must_use]
    pub fn lookup_code(mut self, code: impl Into<String>) -> Self {
        self.lookup_code = Some(code.into());
        self
    }

    #[must_use]
    pub fn number(mut self, number: Option<i64>) -> Self {
        self.number = number;
        self
    }
}

/// Trạng thái ở cơ quan thuế.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EInvoiceStatus {
    Pending,
    Accepted { authority_code: Option<String> },
    Rejected { reason: String },
}

/// Huỷ / thông báo sai sót một hoá đơn.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct EInvoiceCancel {
    pub provider_ref: String,
    pub reason: String,
}

impl EInvoiceCancel {
    pub fn new(provider_ref: impl Into<String>, reason: impl Into<String>) -> Self {
        EInvoiceCancel { provider_ref: provider_ref.into(), reason: reason.into() }
    }
}

/// Định dạng tải về.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EInvoiceFormat {
    Xml,
    Pdf,
}

/// Một nhà cung cấp hoá đơn điện tử.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait EInvoiceProvider: Send + Sync {
    /// Tên ổn định (`vnpt`, `fake`…): ghi trên ký hiệu chứng từ.
    fn provider(&self) -> &'static str;

    fn capabilities(&self) -> EInvoiceCapabilities;

    /// Gửi hoá đơn. Gửi lại cùng `document_id` (sau khi bị từ chối và sửa) dùng lại cùng số.
    async fn issue(&self, tenant: TenantId, invoice: &EInvoice) -> Result<IssuedEInvoice, Error>;

    async fn status(&self, tenant: TenantId, provider_ref: &str) -> Result<EInvoiceStatus, Error>;

    async fn cancel(&self, tenant: TenantId, request: &EInvoiceCancel) -> Result<(), Error>;

    async fn download(
        &self,
        tenant: TenantId,
        provider_ref: &str,
        format: EInvoiceFormat,
    ) -> Result<bytes::Bytes, Error>;
}

/// `einvoice.provider_unavailable` (503).
pub fn provider_unavailable() -> Error {
    Error::unavailable(
        "einvoice.provider_unavailable",
        "The e-invoice provider is not answering. Try again later.",
    )
}

/// `einvoice.rejected` (422): nhà cung cấp / cơ quan thuế từ chối.
pub fn rejected(reason: impl ToString) -> Error {
    Error::new(
        ErrorKind::Validation,
        "einvoice.rejected",
        "The invoice was refused: {reason}. Fix it and send it again with the same number.",
    )
    .with_param("reason", reason)
}

/// `einvoice.not_supported` (422).
pub fn not_supported(provider: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "einvoice.not_supported",
        "The provider `{provider}` does not support this kind of invoice.",
    )
    .with_param("provider", provider)
}
