//! Contract of the `ledger` module for other modules: record double-entry journal entries, reverse entries,
//! open accounts by purpose, read balances. Other modules obtain it at startup:
//! `context.require::<dyn Ledger>()` (registered AFTER `ledger`).
//!
//! The ledger records only **obligations** (D-187): there are no wallet accounts and it never holds buyers'
//! money. Balances are derived from entries; entries are append-only, mistakes are corrected with reversing
//! entries.

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

/// Fake ledger for other modules' tests (feature `testing`).
#[cfg(feature = "testing")]
pub mod testing;

/// Debit or credit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = LedgerSide)]
pub enum Side {
    Debit,
    Credit,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Debit => "debit",
            Side::Credit => "credit",
        }
    }

    pub fn parse(raw: &str) -> Option<Side> {
        match raw {
            "debit" => Some(Side::Debit),
            "credit" => Some(Side::Credit),
            _ => None,
        }
    }

    pub fn opposite(self) -> Side {
        match self {
            Side::Debit => Side::Credit,
            Side::Credit => Side::Debit,
        }
    }
}

/// Base purpose of an account (a stable string — a contract with other modules) and its normal side. Other
/// modules add their own purposes with the prefix `<module>.` (e.g. `customers.points_liability`) and open
/// accounts with [`Ledger::open_account`] choosing the normal side themselves.
pub const PURPOSES: [(&str, Side); 22] = [
    ("cash.bank", Side::Debit),
    ("cash.on_hand", Side::Debit),
    ("clearing.gateway", Side::Debit),
    ("clearing.carrier_cod", Side::Debit),
    ("receivable.customer", Side::Debit),
    // Người bán nợ sàn (phí, khấu trừ thay khi tiền COD về thẳng người bán — D-216).
    ("receivable.seller", Side::Debit),
    ("payable.seller_held", Side::Credit),
    ("payable.seller_available", Side::Credit),
    ("payable.supplier", Side::Credit),
    ("revenue.sales", Side::Credit),
    ("revenue.platform_fee", Side::Credit),
    ("revenue.shipping", Side::Credit),
    ("expense.gateway_fee", Side::Debit),
    ("tax.vat_output", Side::Credit),
    ("tax.vat_input", Side::Debit),
    ("tax.excise_payable", Side::Credit),
    ("tax.withheld_vat", Side::Credit),
    ("tax.withheld_pit", Side::Credit),
    ("liability.customer_credit", Side::Credit),
    ("equity.opening", Side::Credit),
    ("fx.gain_loss", Side::Credit),
    ("rounding", Side::Credit),
];

/// Normal side of a base purpose; `None` for a module's own purpose.
pub fn normal_side_of(purpose: &str) -> Option<Side> {
    PURPOSES.iter().find(|(name, _)| *name == purpose).map(|(_, side)| *side)
}

/// The other party an account is kept for (a seller, a customer, a carrier…).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerCounterparty)]
pub struct Counterparty {
    /// `org`, `customer`, `supplier`, `carrier`, `partner`, `bank_account`, or `<module>.<kind>`.
    pub kind: String,
    /// Id of the party in its own module.
    pub id: String,
}

impl Counterparty {
    pub fn new(kind: impl Into<String>, id: impl Into<String>) -> Self {
        Counterparty { kind: kind.into(), id: id.into() }
    }
}

/// Natural key of an account: ledger (branch), purpose, subject, currency.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccountKey {
    pub book: OrgId,
    pub purpose: String,
    pub counterparty: Option<Counterparty>,
    pub currency: Currency,
}

impl AccountKey {
    pub fn new(book: OrgId, purpose: impl Into<String>, currency: Currency) -> Self {
        AccountKey { book, purpose: purpose.into(), counterparty: None, currency }
    }

    #[must_use]
    pub fn with_counterparty(mut self, counterparty: Counterparty) -> Self {
        self.counterparty = Some(counterparty);
        self
    }
}

/// Opens an account with a chosen normal side, floor and name (a module's own purpose, or an account with a floor).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct OpenAccount {
    pub key: AccountKey,
    /// Empty: follow [`PURPOSES`] (a module's own purpose requires it).
    pub normal_side: Option<Side>,
    /// The balance (on the normal side) must not fall below this level; `None`: no limit.
    pub floor_minor: Option<i64>,
    pub name: Option<String>,
}

impl OpenAccount {
    pub fn new(key: AccountKey) -> Self {
        OpenAccount { key, normal_side: None, floor_minor: None, name: None }
    }

    #[must_use]
    pub fn normal_side(mut self, side: Side) -> Self {
        self.normal_side = Some(side);
        self
    }

    #[must_use]
    pub fn floor(mut self, minor: i64) -> Self {
        self.floor_minor = Some(minor);
        self
    }

    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
}

/// Account of an entry line: by ID, or by key (opened if missing — base purposes only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineAccount {
    Id(Uuid),
    Key(AccountKey),
}

/// A line of a new entry.
#[derive(Debug, Clone)]
pub struct NewLine {
    pub account: LineAccount,
    pub side: Side,
    /// Greater than 0, in the same currency as every other line and as the account.
    pub amount: Money,
}

impl NewLine {
    pub fn debit(account: LineAccount, amount: Money) -> Self {
        NewLine { account, side: Side::Debit, amount }
    }

    pub fn credit(account: LineAccount, amount: Money) -> Self {
        NewLine { account, side: Side::Credit, amount }
    }
}

/// A new entry. `reference` is required (D-184): unique per (tenant, `source_module`).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NewEntry {
    /// Branch that owns the ledger (accounting unit).
    pub book: OrgId,
    /// Posting date; empty: today in the branch's time zone.
    pub effective_date: Option<Date>,
    /// Fixed text of the module (goes into the hash chain, cannot be deleted): must **not** contain names,
    /// email addresses, phone numbers — people and tenants are referred to only by ID in `source_id`.
    pub description: String,
    /// Module ghi (vd `orders`).
    pub source_module: String,
    /// Kind of the source document (e.g. `order`).
    pub source_kind: String,
    /// ID of the source document.
    pub source_id: Option<String>,
    pub reference: String,
    pub lines: Vec<NewLine>,
    /// Exchange rate into the ledger's accounting currency (1 unit of the entry's currency = `exchange_rate`
    /// units of the ledger's currency), when the entry's currency differs from the ledger's. Empty: take the
    /// rate in effect on the posting date ([`Ledger::exchange_rate`]). A foreign-currency document passes
    /// exactly the rate captured on the document.
    pub exchange_rate: Option<rust_decimal::Decimal>,
}

impl NewEntry {
    pub fn new(
        book: OrgId,
        source_module: impl Into<String>,
        source_kind: impl Into<String>,
        reference: impl Into<String>,
        description: impl Into<String>,
        lines: Vec<NewLine>,
    ) -> Self {
        NewEntry {
            book,
            effective_date: None,
            description: description.into(),
            source_module: source_module.into(),
            source_kind: source_kind.into(),
            source_id: None,
            reference: reference.into(),
            lines,
            exchange_rate: None,
        }
    }

    #[must_use]
    pub fn on(mut self, day: Date) -> Self {
        self.effective_date = Some(day);
        self
    }

    #[must_use]
    pub fn source_id(mut self, id: impl Into<String>) -> Self {
        self.source_id = Some(id.into());
        self
    }

    /// Converts into the ledger's currency using exactly the rate `rate` (the snapshot on the document).
    #[must_use]
    pub fn exchange_rate(mut self, rate: rust_decimal::Decimal) -> Self {
        self.exchange_rate = Some(rate);
        self
    }
}

/// Reverses an entry: the new entry swaps debit / credit, in the currently open period.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NewReversal {
    pub source_module: String,
    pub reference: String,
    /// Empty: today in the branch's time zone.
    pub effective_date: Option<Date>,
}

impl NewReversal {
    pub fn new(source_module: impl Into<String>, reference: impl Into<String>) -> Self {
        NewReversal {
            source_module: source_module.into(),
            reference: reference.into(),
            effective_date: None,
        }
    }

    #[must_use]
    pub fn on(mut self, day: Date) -> Self {
        self.effective_date = Some(day);
        self
    }
}

/// One line of an entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerEntryLine)]
pub struct EntryLine {
    pub line_no: i32,
    pub account_id: Uuid,
    pub side: Side,
    pub amount: Money,
    /// The amount in the book currency.
    pub base_amount: Money,
    /// Rate used to convert to the book currency; `null` when the same currency.
    pub exchange_rate: Option<String>,
}

/// A ledger entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerEntry)]
pub struct Entry {
    pub id: Uuid,
    pub book_id: Uuid,
    /// The unit the book belongs to.
    pub org_id: Uuid,
    pub period_id: Uuid,
    /// Accounting day, `YYYY-MM-DD`.
    pub effective_date: Date,
    pub posted_at: Timestamp,
    pub description: String,
    pub source_module: String,
    pub source_kind: String,
    pub source_id: Option<String>,
    pub reference: String,
    /// The entry this one reverses.
    pub reverses_entry_id: Option<Uuid>,
    /// The entry that reverses this one, if any.
    pub reversed_by_entry_id: Option<Uuid>,
    pub actor_user_id: Option<Uuid>,
    pub lines: Vec<EntryLine>,
    /// Position in the book's hash chain; `null` until the entry is sealed (within seconds).
    pub sequence: Option<i64>,
    /// When the entry was sealed into the hash chain; `null` while not yet sealed.
    pub sealed_at: Option<Timestamp>,
}

/// A payment collected through a licensed partner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerPayment)]
pub struct Payment {
    pub id: Uuid,
    /// The unit that receives the money (its book gets the entries).
    pub org_id: Uuid,
    pub book_id: Uuid,
    /// Partner, e.g. `vnpay`.
    pub provider: String,
    /// Id of the transaction at the partner, once created there.
    pub provider_ref: Option<String>,
    /// `card`, `wallet`, `vietqr`…
    pub method: String,
    pub description: String,
    pub amount: Money,
    /// Refunded or being refunded.
    pub refunded: Money,
    /// `pending`, `succeeded`, `failed`, `expired` or `needs_review` (the partner reported another
    /// amount; see the exceptions).
    pub status: String,
    /// Partner page where the buyer pays.
    pub checkout_url: Option<String>,
    /// QR payload, when the partner gives one, or the VietQR code of a bank transfer payment.
    pub qr_payload: Option<String>,
    /// Bank transfer payments: the code the buyer must keep in the transfer description.
    pub transfer_code: Option<String>,
    /// Bank transfer payments: the receiving bank account (of the unit, never of the platform).
    pub bank_account_id: Option<Uuid>,
    pub expires_at: Timestamp,
    pub captured_at: Option<Timestamp>,
    /// Module that asked for the payment (`plans`, `orders`…) and its document.
    pub source_module: String,
    pub source_id: Option<String>,
    pub reference: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// A refund of (part of) a payment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerRefund)]
pub struct Refund {
    pub id: Uuid,
    pub payment_id: Uuid,
    pub amount: Money,
    /// `pending`, `succeeded` or `failed`.
    pub status: String,
    pub provider_ref: Option<String>,
    pub reason: String,
    pub reference: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// A new collection. `reference` is required (D-184): unique per (tenant, `source_module`); sending again
/// after a network error calls the partner again with the same payment.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NewPayment {
    /// Branch that receives the money.
    pub org: OrgId,
    /// Partner; empty when the server has only one gateway.
    pub provider: Option<String>,
    pub method: String,
    pub amount: Money,
    /// Short text for the buyer (at most 140 characters), no personal data.
    pub description: String,
    pub return_url: Option<String>,
    pub source_module: String,
    pub source_id: Option<String>,
    pub reference: String,
    /// Latest moment the payment may be paid: the payment (and the link at the partner) expires at this
    /// moment or after the server's usual time, whichever comes first. Must be in the future.
    pub expires_by: Option<Timestamp>,
}

impl NewPayment {
    pub fn new(
        org: OrgId,
        method: impl Into<String>,
        amount: Money,
        description: impl Into<String>,
        source_module: impl Into<String>,
        reference: impl Into<String>,
    ) -> Self {
        NewPayment {
            org,
            provider: None,
            method: method.into(),
            amount,
            description: description.into(),
            return_url: None,
            source_module: source_module.into(),
            source_id: None,
            reference: reference.into(),
            expires_by: None,
        }
    }

    /// See [`NewPayment::expires_by`].
    #[must_use]
    pub fn expires_by(mut self, at: Timestamp) -> Self {
        self.expires_by = Some(at);
        self
    }

    #[must_use]
    pub fn provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    #[must_use]
    pub fn source_id(mut self, id: impl Into<String>) -> Self {
        self.source_id = Some(id.into());
        self
    }

    #[must_use]
    pub fn return_url(mut self, url: impl Into<String>) -> Self {
        self.return_url = Some(url.into());
        self
    }
}

/// Whether `error`, returned by [`Ledger::refund`] (or by a partner's refund call), is a final refusal:
/// sending the same refund again will never work (not supported, more than what is left, not configured,
/// not found…). A server error (5xx), a conflict with work still in progress (409) and a rate limit (429)
/// are not final: send the same refund again later, with the same `reference` (it is never refunded twice).
/// When no partner can make a refund (no partner configured for the payment, the partner does not refund
/// it, or the partner refused it), the ledger opens a `refund_failed` item in its reconciliation queue (once
/// per `reference`), unless nothing is left to refund or a `late_payment` item of the payment is still open.
/// A mistake in the request itself (more than what is left, another currency) only returns the error.
pub fn refund_refused(error: &Error) -> bool {
    use autoxide_kernel::error::ErrorKind;
    let kind = error.kind();
    kind.status() < 500 && !matches!(kind, ErrorKind::Conflict | ErrorKind::TooManyRequests)
}

/// A refund. `reference` is unique per payment.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NewRefund {
    pub payment: Uuid,
    pub amount: Money,
    /// Short text (at most 200 characters), no personal data.
    pub reason: String,
    pub reference: String,
}

impl NewRefund {
    pub fn new(
        payment: Uuid,
        amount: Money,
        reason: impl Into<String>,
        reference: impl Into<String>,
    ) -> Self {
        NewRefund { payment, amount, reason: reason.into(), reference: reference.into() }
    }
}

// ---- Dòng tiền (bước 36 mục 4, D-187, D-216, D-218) ------------------------------------------

/// How money flows for a unit (D-187: the platform never holds buyers' money).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = LedgerFlowMode)]
pub enum FlowMode {
    /// The seller collects into its own partner or bank account; entries go to the seller's book.
    SelfCollect,
    /// A licensed partner collects on behalf of the marketplace and pays the sellers out; entries go
    /// to the marketplace book, the seller's share is held until the return window ends.
    PartnerCollect,
    /// The payment partner splits the money at payment time; the marketplace book records only its
    /// share (fee and withholding), nothing is held.
    SplitAtGateway,
}

impl FlowMode {
    pub fn as_str(self) -> &'static str {
        match self {
            FlowMode::SelfCollect => "self_collect",
            FlowMode::PartnerCollect => "partner_collect",
            FlowMode::SplitAtGateway => "split_at_gateway",
        }
    }

    pub fn parse(raw: &str) -> Option<FlowMode> {
        match raw {
            "self_collect" => Some(FlowMode::SelfCollect),
            "partner_collect" => Some(FlowMode::PartnerCollect),
            "split_at_gateway" => Some(FlowMode::SplitAtGateway),
            _ => None,
        }
    }
}

/// Where cash on delivery goes (D-216): never to the marketplace operator's own account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = LedgerCodRemittance)]
pub enum CodRemittance {
    /// The carrier pays the cash into the licensed partner's collection account.
    PartnerCollection,
    /// The carrier pays the seller directly; fees and withholding become a debt of the seller
    /// (`receivable.seller`).
    SellerDirect,
}

impl CodRemittance {
    pub fn as_str(self) -> &'static str {
        match self {
            CodRemittance::PartnerCollection => "partner_collection",
            CodRemittance::SellerDirect => "seller_direct",
        }
    }

    pub fn parse(raw: &str) -> Option<CodRemittance> {
        match raw {
            "partner_collection" => Some(CodRemittance::PartnerCollection),
            "seller_direct" => Some(CodRemittance::SellerDirect),
            _ => None,
        }
    }
}

/// The money-flow settings in force for a unit (set on it or inherited from the nearest unit above).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerFlowSettings)]
pub struct FlowSettings {
    /// The unit asked about.
    pub org_id: Uuid,
    /// The unit the settings are set on (its book gets the entries in the collect and split
    /// modes); `null` when nothing is set and the defaults apply.
    pub source_org_id: Option<Uuid>,
    pub mode: FlowMode,
    /// Payment partner used in the collect and split modes.
    pub payment_provider: Option<String>,
    /// Partner that pays the sellers out (collect mode).
    pub payout_provider: Option<String>,
    /// Days the seller's share is held after the money comes in (return window), 0 to 365.
    pub hold_days: i32,
    pub cod_remittance: CodRemittance,
    /// The settings come from a unit above.
    pub inherited: bool,
    pub updated_at: Option<Timestamp>,
}

/// Money route of a collection / refund.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Collection {
    /// Through payment gateway `provider` (the partner's `clearing.gateway` account).
    Gateway { provider: String },
    /// Cash collected on delivery (COD) through shipping carrier `carrier`.
    Cod { carrier: String },
    /// Bank transfer into the seller's / tenant's bank account (self-collect mode).
    BankTransfer { bank_account: Uuid },
}

/// Amounts of a sale: `gross` = `net` + `vat` (tax computed by `TaxEngine`, outside the ledger).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaleAmounts {
    pub gross: Money,
    pub net: Money,
    pub vat: Money,
}

impl SaleAmounts {
    pub fn new(gross: Money, net: Money, vat: Money) -> Self {
        SaleAmounts { gross, net, vat }
    }
}

/// A cash-flow task; each task produces (at most) one entry according to the seller's mode.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FlowEvent {
    /// Money of a sale has arrived. Split-at-gateway mode: `operator_share` is the portion the gateway splits
    /// to the marketplace (required).
    Collected { amounts: SaleAmounts, via: Collection, operator_share: Option<Money> },
    /// Marketplace fee `fee` (including the fee's VAT `vat` — kept separate in the ledger and the fee invoice).
    FeeCharged { fee: Money, vat: Money },
    /// Withholding and paying tax on the seller's behalf on revenue `revenue` (Decree 117/2025/ND-CP — verify
    /// against the original text): the ledger asks `TaxEngine`; only household / individual businesses are
    /// withheld, businesses self-declare (no entry produced). `business_line`: `goods`, `services`,
    /// `production_transport`, `other`.
    TaxWithheld { revenue: Money, business_line: String },
    /// Refund to the buyer. Before the return window ends: reduces the held amount. Split-at-gateway mode:
    /// `amounts.gross` is the portion returned out of the marketplace's share.
    Refunded { amounts: SaleAmounts, via: Collection },
    /// Payment partner fee.
    GatewayFeeCharged { provider: String, fee: Money },
}

/// A cash-flow task sent to [`Ledger::settle`]. `reference` is unique per (tenant,
/// `source_module`) — it shares one namespace with [`Ledger::post`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Settlement {
    /// Seller (branch); the mode is read from this branch's settings or the nearest ancestor branch's.
    pub seller: OrgId,
    pub event: FlowEvent,
    pub source_module: String,
    /// Source document (order…): the held amount is attached to it.
    pub source_id: String,
    pub reference: String,
    /// Empty: today in the ledger's time zone.
    pub effective_date: Option<Date>,
}

impl Settlement {
    pub fn new(
        seller: OrgId,
        event: FlowEvent,
        source_module: impl Into<String>,
        source_id: impl Into<String>,
        reference: impl Into<String>,
    ) -> Self {
        Settlement {
            seller,
            event,
            source_module: source_module.into(),
            source_id: source_id.into(),
            reference: reference.into(),
            effective_date: None,
        }
    }

    #[must_use]
    pub fn on(mut self, day: Date) -> Self {
        self.effective_date = Some(day);
        self
    }
}

/// Result of [`Ledger::settle`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settled {
    pub mode: FlowMode,
    /// Branch of the ledger that receives the entry (the seller in self-collect mode, the marketplace in the other two).
    pub book_org: OrgId,
    /// `None`: this task produces no entry (e.g. withholding on behalf of a business seller).
    pub entry: Option<Entry>,
    /// The held amount of the sale, after this task.
    pub hold: Option<Hold>,
}

/// The seller's share of a sale held until the return window ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerHold)]
pub struct Hold {
    pub id: Uuid,
    pub book_id: Uuid,
    /// The unit of the book (the marketplace).
    pub org_id: Uuid,
    pub seller_org_id: Uuid,
    /// Still held: the sale minus fees, withholding and refunds so far.
    pub amount: Money,
    /// Held when the money came in.
    pub original: Money,
    /// When the job moves the money to the seller's available balance.
    pub release_at: Timestamp,
    /// `held` or `released`.
    pub status: String,
    pub source_module: String,
    pub source_id: String,
    /// The entry that moved the money to the available balance.
    pub release_entry_id: Option<Uuid>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// A payout of a seller's available balance through the payout partner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerPayout)]
pub struct Payout {
    pub id: Uuid,
    pub book_id: Uuid,
    /// The unit of the book (the marketplace).
    pub org_id: Uuid,
    /// The seller paid.
    pub payee_org_id: Uuid,
    pub amount: Money,
    pub provider: String,
    pub provider_ref: Option<String>,
    /// `pending` (the partner has not answered yet; send the same request again), `sent` (the
    /// partner is working on it), `succeeded` or `failed` (the money is back on the seller's
    /// available balance).
    pub status: String,
    pub failure_reason: Option<String>,
    pub description: String,
    pub reference: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// A payout of a seller's available balance. `reference` is unique per tenant.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NewPayout {
    pub payee: OrgId,
    pub amount: Money,
    /// Short text (at most 140 characters), no personal data.
    pub description: String,
    pub reference: String,
}

impl NewPayout {
    pub fn new(
        payee: OrgId,
        amount: Money,
        description: impl Into<String>,
        reference: impl Into<String>,
    ) -> Self {
        NewPayout { payee, amount, description: description.into(), reference: reference.into() }
    }
}

/// A collection by VietQR bank transfer into the receiving branch's bank account (self-collect mode).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NewTransferPayment {
    pub org: OrgId,
    /// Receiving account; empty when the branch has only one account in use.
    pub bank_account: Option<Uuid>,
    /// VND only.
    pub amount: Money,
    pub description: String,
    pub source_module: String,
    pub source_id: Option<String>,
    pub reference: String,
    /// Latest moment the transfer may arrive: see [`NewPayment::expires_by`].
    pub expires_by: Option<Timestamp>,
}

impl NewTransferPayment {
    pub fn new(
        org: OrgId,
        amount: Money,
        description: impl Into<String>,
        source_module: impl Into<String>,
        reference: impl Into<String>,
    ) -> Self {
        NewTransferPayment {
            org,
            bank_account: None,
            amount,
            description: description.into(),
            source_module: source_module.into(),
            source_id: None,
            reference: reference.into(),
            expires_by: None,
        }
    }

    /// See [`NewTransferPayment::expires_by`].
    #[must_use]
    pub fn expires_by(mut self, at: Timestamp) -> Self {
        self.expires_by = Some(at);
        self
    }

    #[must_use]
    pub fn bank_account(mut self, id: Uuid) -> Self {
        self.bank_account = Some(id);
        self
    }

    #[must_use]
    pub fn source_id(mut self, id: impl Into<String>) -> Self {
        self.source_id = Some(id.into());
        self
    }
}

/// An exchange rate in force from a day: 1 `base` = `rate` `quote`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerExchangeRate)]
pub struct ExchangeRate {
    pub id: Uuid,
    /// ISO 4217 code, e.g. `USD`.
    pub base: String,
    /// ISO 4217 code, e.g. `VND`.
    pub quote: String,
    /// Units of `quote` for one unit of `base`, as a decimal string, e.g. `"25345.5"`.
    pub rate: DecimalString,
    /// First day the rate is used; it stays in force until a later rate of the same pair.
    pub effective_date: Date,
    /// `manual` (entered by a person) or `provider:<name>` (fetched from a rate source).
    pub source: String,
    pub created_at: Timestamp,
}

/// General journal lines to read (accounting export): a date range, one ledger or all ledgers.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct JournalQuery {
    /// Only this branch's ledger; empty: every ledger of the tenant.
    pub book: Option<OrgId>,
    /// From posting date (inclusive).
    pub from: Date,
    /// To posting date (inclusive).
    pub to: Date,
    /// Page (by ascending line ID).
    pub page: autoxide_kernel::pagination::PageRequest,
}

impl JournalQuery {
    pub fn new(from: Date, to: Date) -> Self {
        JournalQuery {
            book: None,
            from,
            to,
            page: autoxide_kernel::pagination::PageRequest::first(
                500,
                autoxide_kernel::pagination::Order::Asc,
            ),
        }
    }

    #[must_use]
    pub fn book(mut self, org: OrgId) -> Self {
        self.book = Some(org);
        self
    }

    #[must_use]
    pub fn page(mut self, page: autoxide_kernel::pagination::PageRequest) -> Self {
        self.page = page;
        self
    }
}

/// One line of the general journal, with its entry and account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerJournalLine)]
pub struct JournalLine {
    pub line_id: Uuid,
    pub entry_id: Uuid,
    /// The unit the book belongs to.
    pub org_id: Uuid,
    pub effective_date: Date,
    pub posted_at: Timestamp,
    pub description: String,
    pub source_module: String,
    pub source_kind: String,
    pub source_id: Option<String>,
    pub reference: String,
    pub reverses_entry_id: Option<Uuid>,
    pub line_no: i32,
    pub account_id: Uuid,
    pub purpose: String,
    pub counterparty_kind: Option<String>,
    pub counterparty_id: Option<String>,
    /// Account number for accounting software: the account's own code, or the Vietnamese chart of
    /// accounts mapping of its purpose (`data/vn-accounts.json`), or the purpose itself.
    pub account_code: String,
    pub side: Side,
    pub amount: Money,
    /// The amount in the book currency.
    pub base_amount: Money,
    pub exchange_rate: Option<String>,
}

/// Writes and reads the ledger.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait Ledger: Send + Sync {
    /// Records an entry. The same `reference` + same content → `Replayed` (nothing more is written); the same
    /// `reference` with different content → `reference_reused`.
    async fn post(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        entry: NewEntry,
    ) -> Result<Submitted<Entry>, Error>;

    /// Reverses entry `entry` (once): `ledger.already_reversed` when already reversed.
    async fn reverse(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        entry: Uuid,
        reversal: NewReversal,
    ) -> Result<Submitted<Entry>, Error>;

    /// ID of the account with key `key`, opened if missing (base purposes only).
    async fn ensure_account(&self, tenant: &TenantContext, key: AccountKey) -> Result<Uuid, Error>;

    /// Opens (or returns) an account with a chosen normal side / floor / name.
    async fn open_account(
        &self,
        tenant: &TenantContext,
        account: OpenAccount,
    ) -> Result<Uuid, Error>;

    /// Balance (on the normal side) of the account with key `key`: current, or at the end of day `as_of`.
    /// No such account: 0.
    async fn balance(
        &self,
        tenant: &TenantContext,
        key: AccountKey,
        as_of: Option<Date>,
    ) -> Result<Money, Error>;

    /// An entry, or `None`.
    async fn entry(&self, tenant: &TenantContext, id: Uuid) -> Result<Option<Entry>, Error>;

    /// Creates a collection through a partner: returns `checkout_url` / `qr_payload` for the buyer. When the
    /// partner reports it collected (webhook, reconciliation), the ledger records the entry and emits the
    /// event `ledger.payment_succeeded` (`payment_id`, `source_module`, `source_id`, `reference`, `amount`)
    /// to the source module.
    async fn create_payment(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        payment: NewPayment,
    ) -> Result<Submitted<Payment>, Error>;

    /// Refunds (part of) a collected payment; not more than what remains.
    async fn refund(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        refund: NewRefund,
    ) -> Result<Submitted<Refund>, Error>;

    /// The source module counted `payment` towards its record (for example the order went on) although the
    /// ledger received the money late (after the payment's deadline, or after the partner reported it
    /// failed): closes the payment's open `late_payment` item (`resolved`, resolution `kept`), so nobody
    /// refunds money that is in use. The source module decides whether late money is kept or refunded; it
    /// calls this when it keeps it, and `refund` when it does not. Calling it again, or for a payment
    /// without such an item, does nothing.
    async fn keep_late_payment(&self, tenant: &TenantContext, payment: Uuid) -> Result<(), Error> {
        // Mặc định không làm gì: giữ tương thích cho bên ngoài tự cài `Ledger` (thêm hàm không phá vỡ).
        let _ = (tenant, payment);
        Ok(())
    }

    /// A payment, or `None`.
    async fn payment(&self, tenant: &TenantContext, id: Uuid) -> Result<Option<Payment>, Error>;

    /// Balances (on the normal side) of many accounts in one call (at most 500 keys), in the same order as
    /// `keys` — e.g. supplier payables (`payable.supplier`, subject `supplier`). Not yet existing: 0.
    async fn balances(
        &self,
        tenant: &TenantContext,
        keys: &[AccountKey],
        as_of: Option<Date>,
    ) -> Result<Vec<Money>, Error>;

    /// Effective cash-flow settings of branch `org` (inherited along the tree; not set: self-collect).
    async fn flow_settings(
        &self,
        tenant: &TenantContext,
        org: OrgId,
    ) -> Result<FlowSettings, Error>;

    /// Records a cash-flow task according to the seller's mode: incoming money, marketplace fee, withholding
    /// on behalf, refund, gateway fee. In collect-on-behalf mode, incoming money creates a held amount until
    /// the return window ends (the job `ledger.release_holds` moves it to the available balance). A task not
    /// suited to the mode or gateway → `ledger.flow_mode_unsupported`. The same `reference` + same content →
    /// `Replayed`.
    async fn settle(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        settlement: Settlement,
    ) -> Result<Submitted<Settled>, Error>;

    /// The held amount of a source document (`marketplace` asks before allowing a withdrawal).
    async fn hold_status(
        &self,
        tenant: &TenantContext,
        source_module: &str,
        source_id: &str,
    ) -> Result<Option<Hold>, Error>;

    /// Pays out the seller's available balance through `PayoutGateway` (collect-on-behalf mode). Deducts the
    /// balance first, calls the partner outside the transaction; if the partner rejects, the balance is
    /// recorded back. Never exceeds the available balance, even with concurrent sends
    /// (`ledger.insufficient_balance`).
    async fn create_payout(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        payout: NewPayout,
    ) -> Result<Submitted<Payout>, Error>;

    /// Creates a VietQR bank-transfer collection (the QR code carries the transfer code; money goes straight
    /// into the receiving branch's account). A balance movement (`BankFeed`) matching the code + amount
    /// records it as collected; mismatches go into a queue. Same payment deadline as every payment
    /// (`expires_at`).
    async fn create_transfer_payment(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        payment: NewTransferPayment,
    ) -> Result<Submitted<Payment>, Error>;

    /// Exchange rate 1 `base` = ? `quote` in effect on day `on` (D-237): the rate whose effective date is
    /// nearest ≤ `on`; on the same day a manually entered rate takes precedence over a source's rate. Not yet
    /// existing: `None`. Reads; do not call inside a write transaction. `invoices`, consolidated reports (38)
    /// and multi-currency (63) read through here.
    async fn exchange_rate(
        &self,
        tenant: &TenantContext,
        base: Currency,
        quote: Currency,
        on: Date,
    ) -> Result<Option<ExchangeRate>, Error>;

    /// Accounting currency of branch `org`'s ledger (ledger not opened: the branch's default currency, or VND).
    async fn book_currency(&self, tenant: &TenantContext, org: OrgId) -> Result<Currency, Error>;

    /// A page of the general journal (entry lines with entry and account) for accounting export.
    async fn journal(
        &self,
        tenant: &TenantContext,
        query: JournalQuery,
    ) -> Result<autoxide_kernel::pagination::Page<JournalLine>, Error>;
}
