//! E-invoicing through a provider (step 36): shared traits that let the `invoices` module send invoices to an
//! e-invoice service provider (which forwards them to the tax authority) without depending on any specific
//! provider.
//!
//! A provider's adapter is its own module that depends only on the kernel and is registered BEFORE
//! `invoices`, contributing via `context.contribute::<dyn EInvoiceProvider>(...)`. Every function takes the
//! tenant (`TenantId`): the adapter reads that tenant's connection settings itself. Tests use
//! `FakeEInvoiceProvider` from the testkit.
use crate::time::Date;
use uuid::Uuid;

use crate::error::{Error, ErrorKind};
use crate::money::{Currency, Money};
use crate::tenancy::TenantId;

/// Kind of e-invoice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EInvoiceKind {
    /// Value-added tax invoice.
    Vat,
    /// Sales invoice.
    Sales,
    /// Invoice generated from a cash register.
    CashRegister,
    /// Personal income tax withholding certificate.
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

/// What a provider can do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct EInvoiceCapabilities {
    pub kinds: Vec<EInvoiceKind>,
    /// The provider assigns invoice numbers itself (an internal number is still assigned; the provider's number is stored on acceptance).
    pub assigns_numbers: bool,
    pub adjust: bool,
    pub replace: bool,
    pub cancel: bool,
}

impl EInvoiceCapabilities {
    pub fn new(kinds: &[EInvoiceKind]) -> Self {
        EInvoiceCapabilities { kinds: kinds.to_vec(), ..EInvoiceCapabilities::default() }
    }

    /// Adjusting, replacing and cancelling are all supported.
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

/// A party on the invoice (snapshot taken when issued).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Party {
    pub name: String,
    pub tax_code: Option<String>,
    /// Address as a single line of text.
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

/// One invoice line.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct EInvoiceLine {
    pub line_no: i32,
    pub description: String,
    pub unit: Option<String>,
    /// Quantity, normalized decimal string (e.g. `"2"`, `"1.5"`).
    pub quantity: String,
    pub unit_price: Money,
    /// Amount before tax (after discount).
    pub net: Money,
    /// Tax rate (e.g. `"8"`), `None` when not taxable.
    pub rate_percent: Option<String>,
    pub tax: Money,
    pub total: Money,
}

impl EInvoiceLine {
    /// Line `line_no`: quantity (decimal string), unit price, amount before tax, tax.
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

/// Totals per tax rate.
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

/// Relationship to an already issued invoice (Decree 123/2020 Article 19, amended by Decree 70/2025; needs verification).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    Adjustment,
    Replacement,
}

/// Related invoice (the one being adjusted / replaced).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RelatedInvoice {
    pub relation: Relation,
    pub provider_ref: Option<String>,
    pub series: String,
    pub number: i64,
    pub issue_date: Date,
}

impl RelatedInvoice {
    pub fn new(
        relation: Relation,
        series: impl Into<String>,
        number: i64,
        issue_date: Date,
    ) -> Self {
        RelatedInvoice { relation, provider_ref: None, series: series.into(), number, issue_date }
    }

    #[must_use]
    pub fn provider_ref(mut self, reference: Option<String>) -> Self {
        self.provider_ref = reference;
        self
    }
}

/// An invoice sent to the provider.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct EInvoice {
    /// ID of the document in this system: the provider de-duplicates by this value (resending the same
    /// invoice).
    pub document_id: Uuid,
    pub kind: EInvoiceKind,
    pub seller: Party,
    /// Seller issued on behalf of when there is an authorization (the issuing party is `seller`).
    pub on_behalf_of: Option<Party>,
    pub buyer: Party,
    /// Series symbol, e.g. `1C26TAA`.
    pub series: String,
    /// Internal number; may be omitted when the provider assigns numbers.
    pub number: Option<i64>,
    pub issue_date: Date,
    pub currency: Currency,
    /// Exchange rate to Vietnamese dong (decimal string), when the currency is not VND.
    pub exchange_rate: Option<String>,
    pub lines: Vec<EInvoiceLine>,
    pub totals: Vec<EInvoiceTaxTotal>,
    pub related: Option<RelatedInvoice>,
    pub payment_method: String,
}

impl EInvoice {
    /// An invoice with no series and no lines yet: add them with [`series`](Self::series), [`lines`](Self::lines).
    pub fn new(
        document_id: Uuid,
        kind: EInvoiceKind,
        seller: Party,
        buyer: Party,
        issue_date: Date,
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

/// An invoice the provider has accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct IssuedEInvoice {
    pub provider_ref: String,
    /// Number assigned by the provider (when `assigns_numbers`).
    pub number: Option<i64>,
    /// Tax authority code, if available immediately.
    pub authority_code: Option<String>,
    /// Lookup code for the buyer.
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

/// Status at the tax authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EInvoiceStatus {
    Pending,
    Accepted {
        authority_code: Option<String>,
        /// Invoice number assigned by the provider, when the provider assigns numbers itself and only learns
        /// the number once accepted (`None`: none, or already returned in [`IssuedEInvoice::number`]). This
        /// is the legally valid number for tax purposes; `invoices` stores it and returns it together with
        /// the tax authority code (D-20261122-einvoice-provider-number).
        number: Option<i64>,
    },
    Rejected {
        reason: String,
    },
}

/// Cancels / reports an error on an invoice.
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

/// Download format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EInvoiceFormat {
    Xml,
    Pdf,
}

/// An e-invoice provider.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait EInvoiceProvider: Send + Sync {
    /// Stable name (`vnpt`, `fake`, ...): written on the document series.
    fn provider(&self) -> &'static str;

    fn capabilities(&self) -> EInvoiceCapabilities;

    /// Sends an invoice. Resending the same `document_id` (after a rejection and a fix) reuses the same number.
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

/// `einvoice.rejected` (422): the provider / tax authority rejected it.
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
