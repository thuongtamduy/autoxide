//! Contract of the `tax` module for other modules: tax calculation for a document, withholding tax, tax
//! profiles (inherited along the organization tree) and invoice requirements. Other modules obtain it at
//! startup: `context.require::<dyn TaxEngine>()` (registered AFTER `tax`).
//!
//! Every function is read-only; do NOT call in a write transaction (it reads the control database — SQLite blocks other writes).

use autoxide_kernel::decimal::{Decimal, DecimalString};
use autoxide_kernel::error::Error;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use autoxide_kernel::time::{Date, Timestamp};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Who buys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = TaxBuyerKind)]
pub enum BuyerKind {
    Consumer,
    Business,
}

/// Whether the line amounts include the taxes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = TaxPriceMode)]
pub enum PriceMode {
    /// Amounts include the taxes; they are split out.
    Inclusive,
    /// Amounts are before taxes; taxes are added.
    Exclusive,
}

/// Kind of invoice the seller issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = TaxInvoiceType)]
pub enum InvoiceType {
    /// VAT invoice: VAT shown per rate (seller declares VAT by credit).
    VatInvoice,
    /// Sales invoice: no VAT shown per line (household business, direct method, exempt seller).
    SalesInvoice,
}

/// One line to tax.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct TaxableLine {
    /// The caller's id of the line, given back in the answer.
    pub line_id: String,
    /// Tax category code, e.g. `vat.standard` (`GET /api/v1/tax/categories`).
    pub category: String,
    /// Amount of the line (after discounts), in the document currency; not negative.
    pub amount: Money,
    /// Quantity, for taxes charged per unit.
    pub quantity: Option<DecimalString>,
}

impl TaxableLine {
    pub fn new(line_id: impl Into<String>, category: impl Into<String>, amount: Money) -> Self {
        TaxableLine { line_id: line_id.into(), category: category.into(), amount, quantity: None }
    }

    pub fn with_quantity(mut self, quantity: Decimal) -> Self {
        self.quantity = Some(DecimalString(quantity));
        self
    }
}

/// What to tax.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TaxQuoteRequest {
    /// Tax jurisdiction, e.g. `VN`.
    pub jurisdiction: String,
    /// Day the tax applies (in the seller's time zone).
    pub tax_point: Date,
    /// The selling unit: its tax profile (or the nearest one above) decides the regime.
    pub seller: OrgId,
    pub buyer_kind: BuyerKind,
    pub price_mode: PriceMode,
    pub currency: Currency,
    pub lines: Vec<TaxableLine>,
}

impl TaxQuoteRequest {
    pub fn new(
        jurisdiction: impl Into<String>,
        tax_point: Date,
        seller: OrgId,
        price_mode: PriceMode,
        currency: Currency,
        lines: Vec<TaxableLine>,
    ) -> Self {
        TaxQuoteRequest {
            jurisdiction: jurisdiction.into(),
            tax_point,
            seller,
            buyer_kind: BuyerKind::Consumer,
            price_mode,
            currency,
            lines,
        }
    }

    pub fn for_business(mut self) -> Self {
        self.buyer_kind = BuyerKind::Business;
        self
    }
}

/// One tax on a line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TaxComponent {
    /// `vat`, `excise`, `pit`…
    #[schema(example = "vat")]
    pub tax_kind: String,
    /// Rate in percent as a decimal string, e.g. `"8"`; `"0"` for a per-unit or exempt tax.
    #[schema(example = "8")]
    pub rate_percent: String,
    /// `true`: not subject to this tax (shown as such on the invoice, not as 0%).
    pub exempt: bool,
    pub amount: Money,
}

/// A line with its taxes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TaxedLine {
    pub line_id: String,
    pub category: String,
    /// Amount before taxes.
    pub net: Money,
    pub taxes: Vec<TaxComponent>,
    /// Amount with taxes: `net` + every tax.
    pub gross: Money,
}

/// Total of one tax at one rate on the whole document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TaxTotal {
    pub tax_kind: String,
    pub rate_percent: String,
    pub exempt: bool,
    /// Amount the tax is computed on.
    pub base: Money,
    pub amount: Money,
}

/// Taxes of a document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct TaxQuote {
    pub lines: Vec<TaxedLine>,
    pub totals: Vec<TaxTotal>,
    pub net: Money,
    pub tax: Money,
    pub gross: Money,
    pub invoice_type: InvoiceType,
    /// The seller's regime used: `vat_credit`, `vat_direct`, `household` or `exempt`.
    #[schema(example = "vat_credit")]
    pub regime: String,
    /// What was used (rates and their ids, pack version, rounding): keep it with the document.
    #[schema(value_type = Object)]
    pub snapshot: serde_json::Value,
}

/// Tax a platform withholds from a payment to a seller (Vietnam: Decree 117/2025/ND-CP).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WithholdingRequest {
    pub jurisdiction: String,
    pub on: Date,
    /// `marketplace` or `affiliate`.
    pub payer_kind: String,
    /// The payee's regime (`household`…); businesses declare themselves (nothing withheld).
    pub payee_regime: String,
    /// `goods`, `services`, `production_transport` or `other`.
    pub business_line: String,
    pub amount: Money,
}

impl WithholdingRequest {
    pub fn new(
        jurisdiction: impl Into<String>,
        on: Date,
        payer_kind: impl Into<String>,
        payee_regime: impl Into<String>,
        business_line: impl Into<String>,
        amount: Money,
    ) -> Self {
        WithholdingRequest {
            jurisdiction: jurisdiction.into(),
            on,
            payer_kind: payer_kind.into(),
            payee_regime: payee_regime.into(),
            business_line: business_line.into(),
            amount,
        }
    }
}

/// Taxes to withhold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct WithholdingQuote {
    pub components: Vec<TaxComponent>,
    pub total: Money,
    #[schema(value_type = Object)]
    pub snapshot: serde_json::Value,
}

/// Tax profile of an organization unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TaxProfile {
    /// The unit the profile is set on (the unit asked about, or the nearest one above it).
    pub org_id: Uuid,
    #[schema(example = "VN")]
    pub jurisdiction: String,
    /// Tax code: 10 or 13 digits (`0101234567-001` for a branch), or a 12-digit personal id used
    /// as tax code.
    pub tax_code: Option<String>,
    /// `vat_credit` (VAT by credit), `vat_direct` (VAT on revenue), `household` (household
    /// business) or `exempt`.
    #[schema(example = "vat_credit")]
    pub regime: String,
    /// `goods`, `services`, `production_transport`, `other`.
    pub business_lines: Vec<String>,
    /// Estimated revenue a year, for thresholds (cash register invoices).
    pub annual_revenue: Option<Money>,
    pub legal_name: Option<String>,
    #[schema(value_type = Option<Object>)]
    pub address: Option<serde_json::Value>,
}

/// What the seller must do about invoices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct InvoiceRequirements {
    /// Invoices must come from a cash register connected to the tax authority.
    pub cash_register_required: bool,
    /// Legal basis of the rule that applied.
    pub legal_basis: Option<String>,
}

/// Tax engine (published by the `tax` module).
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait TaxEngine: Send + Sync {
    /// Tax of a document. Errors: `tax.jurisdiction_unknown`, `tax.category_unknown`,
    /// `tax.rate_missing` (a tax rate is never guessed), `validation_failed`.
    async fn quote(
        &self,
        tenant: &TenantContext,
        request: TaxQuoteRequest,
    ) -> Result<TaxQuote, Error>;
    /// Withholding tax. A business recipient (who self-declares) is not withheld.
    async fn withholding(
        &self,
        tenant: &TenantContext,
        request: WithholdingRequest,
    ) -> Result<WithholdingQuote, Error>;
    /// Tax profile of branch `org` or of the nearest ancestor branch that has one.
    async fn profile(
        &self,
        tenant: &TenantContext,
        org: OrgId,
    ) -> Result<Option<TaxProfile>, Error>;
    /// Invoice requirement of seller `org` on date `on`.
    async fn invoice_requirements(
        &self,
        tenant: &TenantContext,
        org: OrgId,
        on: Date,
    ) -> Result<InvoiceRequirements, Error>;
    /// Whether the law of seller `org`'s tax jurisdiction forbids cancelling an e-invoice already sent to the
    /// tax authority, for an invoice dated `on` (Vietnam: since 2025-06-01 an erroneous e-invoice is fixed by an
    /// adjustment or a replacement, not cancelled). `Some(legal basis)` when it does — then cancel through an
    /// adjustment (decrease) or a replacement instead.
    ///
    /// The default implementation forbids nothing; an engine with jurisdiction rules overrides it.
    async fn einvoice_cancel_forbidden(
        &self,
        _tenant: &TenantContext,
        _org: OrgId,
        _on: Date,
    ) -> Result<Option<String>, Error> {
        Ok(None)
    }
    /// The tax date of instant `at` for seller `org`: the calendar day in the time zone of the seller's tax
    /// jurisdiction (pack rule `tax_date.time_zone`; Vietnam: `Asia/Ho_Chi_Minh`), else in the unit's time
    /// zone (`organizations.timezone`), else in UTC. Use it for the tax point of an order and the issue
    /// date of an invoice — never the UTC day of the instant.
    ///
    /// The default implementation returns the UTC day; an engine that knows time zones overrides it.
    async fn tax_date(
        &self,
        _tenant: &TenantContext,
        _org: OrgId,
        at: Timestamp,
    ) -> Result<Date, Error> {
        Ok(at.date_utc())
    }
}
