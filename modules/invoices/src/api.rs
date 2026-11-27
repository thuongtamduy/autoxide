//! Contract of the `invoices` module for other modules: create documents (draft), issue (gapless numbering),
//! read, cancel. Other modules obtain it at startup: `context.require::<dyn Invoices>()` (registered AFTER
//! `invoices`).
//!
//! `invoices` computes tax through `dyn TaxEngine` and posts to the ledger through `dyn Ledger` when issuing
//! a sales document (Debit `receivable.customer` / Credit `revenue.sales`, `tax.vat_output`): the calling
//! module does not number, compute tax, or post to the ledger for documents itself.

use autoxide_kernel::decimal::DecimalString;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use autoxide_kernel::time::{Date, Timestamp};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Document kind. The first four kinds are sales documents (taxed, posted to the ledger on issue).
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

/// Sales document: tax computed through `TaxEngine`, revenue posted on issue.
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

/// A draft document to create. `reference` is required (D-184): unique per (tenant, `source_module`).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NewDocument {
    /// Branch creating the document.
    pub org: OrgId,
    /// Seller on whose behalf it is created (requires an authorization still in effect).
    pub on_behalf_of: Option<OrgId>,
    pub kind: String,
    /// Series symbol; if empty, uses the only series symbol in use of the branch for this kind and issue year.
    pub series: Option<Uuid>,
    /// Expected document date; if empty, it is the issue date.
    pub issue_date: Option<Date>,
    /// Whether line prices include tax (`true`) or not (`false`).
    pub prices_include_tax: bool,
    pub currency: Currency,
    pub buyer: Buyer,
    pub lines: Vec<NewDocumentLine>,
    pub source_module: String,
    pub source_id: Option<String>,
    pub reference: String,
    /// The creator already booked the revenue in the ledger itself (e.g. `orders` books it when delivery
    /// completes): issuing posts no journal entry, the document is just an invoice. Defaults to `false`.
    pub booked_by_source: bool,
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
            booked_by_source: false,
        }
    }

    /// The creator already booked the revenue (see [`NewDocument::booked_by_source`]).
    #[must_use]
    pub fn booked_by_source(mut self) -> Self {
        self.booked_by_source = true;
        self
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
    pub issue_date: Option<Date>,
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
    pub retention_until: Option<Date>,
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
    pub issued_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Print format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderFormat {
    /// HTML built from the tenant's template (or the default template).
    Html,
    /// Fixed-layout PDF from the same view model.
    Pdf,
}

/// A print: content type, suggested file name, content.
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

/// Creates, issues and reads documents.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait Invoices: Send + Sync {
    /// Creates a draft document (no number yet), tax precomputed. Same `reference` + same content → `Replayed`.
    async fn create_document(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        draft: NewDocument,
    ) -> Result<Submitted<Document>, Error>;

    /// Issues: assigns the next number of the series symbol (gapless), sends the e-invoice if the symbol has
    /// a provider, posts to the ledger. Calling again → the same number.
    async fn issue(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        document: Uuid,
    ) -> Result<Document, Error>;

    /// A document, or `None`.
    async fn document(&self, tenant: &TenantContext, id: Uuid) -> Result<Option<Document>, Error>;

    /// Creates a draft adjustment invoice (`direction`: `increase` / `decrease`) for the issued document
    /// `original`, lines being the difference (e.g. returned goods). Same `reference` + same content →
    /// `Replayed`. Only issuing the draft with [`Invoices::issue`] marks the original document `adjusted`.
    async fn adjust(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        original: Uuid,
        direction: &str,
        lines: Vec<NewDocumentLine>,
        reference: String,
    ) -> Result<Submitted<Document>, Error>;

    /// Cancels an issued document (reports the error to the provider, reverses the journal entry).
    ///
    /// Errors: `invoices.not_cancellable` (a draft, a corrected document, or the status changed meanwhile —
    /// call again); `invoices.einvoice_not_cancellable` (409, param `legal_basis`) when the document was sent,
    /// or is being sent, to the tax authority through an e-invoice provider and the seller's tax jurisdiction
    /// forbids cancelling it (Vietnam since 2025-06-01) — issue an adjustment with direction `decrease`
    /// ([`Invoices::adjust`]) or a replacement instead; `invoices.einvoice_cancel_unsupported` (409, param
    /// `provider`) when the provider cannot cancel a sent invoice — adjust or replace it the same way. A document
    /// the tax authority rejected is cancelled.
    async fn cancel(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        document: Uuid,
        reason: String,
    ) -> Result<Document, Error>;

    /// Print of a document (D-353): other modules (orders, inventory) create delivery notes, goods-receipt
    /// notes, receipts… through [`Invoices::create_document`] and then print through here — one place holding
    /// per-tenant print templates. `locale`: `vi` or `en`. No such document: `invoices.document_not_found`.
    async fn render(
        &self,
        tenant: &TenantContext,
        document: Uuid,
        format: RenderFormat,
        locale: &str,
    ) -> Result<Rendered, Error>;

    /// Packs the tenant's accounting archive (general journal, documents, `manifest.json` with the SHA-256 of
    /// each file) into shared storage, outside the tenant's directory — it remains after the tenant is fully
    /// deleted. Returns the archive ID.
    async fn archive(&self, tenant: &TenantContext, actor: Actor) -> Result<Uuid, Error>;
}
