//! Hợp đồng của module `ledger` cho module khác: ghi bút toán kép, đảo bút toán, mở tài khoản theo
//! mục đích, đọc số dư. Module khác lấy lúc khởi động: `context.require::<dyn Ledger>()` (đăng ký
//! SAU `ledger`).
//!
//! Sổ cái chỉ ghi **nghĩa vụ** (D-187): không có tài khoản ví, không giữ tiền của người mua. Số dư
//! suy từ bút toán; bút toán chỉ thêm, sửa sai bằng bút toán đảo.

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

/// Mục đích cơ bản của tài khoản (chuỗi ổn định — hợp đồng với module khác) và bên thường của nó.
/// Module khác thêm mục đích riêng có tiền tố `<module>.` (vd `customers.points_liability`) và mở
/// tài khoản bằng [`Ledger::open_account`] với bên thường tự chọn.
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

/// Bên thường của một mục đích cơ bản; `None` cho mục đích riêng của module.
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

/// Khoá tự nhiên của một tài khoản: sổ (nút), mục đích, đối tượng, loại tiền.
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

/// Mở tài khoản với bên thường, sàn và tên tự chọn (mục đích riêng của module, hay tài khoản có sàn).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct OpenAccount {
    pub key: AccountKey,
    /// Bỏ trống: theo [`PURPOSES`] (mục đích riêng thì bắt buộc).
    pub normal_side: Option<Side>,
    /// Số dư (theo bên thường) không được xuống dưới mức này; `None`: không giới hạn.
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

/// Tài khoản của một dòng bút toán: theo ID, hay theo khoá (mở nếu chưa có — chỉ mục đích cơ bản).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineAccount {
    Id(Uuid),
    Key(AccountKey),
}

/// Một dòng của bút toán mới.
#[derive(Debug, Clone)]
pub struct NewLine {
    pub account: LineAccount,
    pub side: Side,
    /// Lớn hơn 0, cùng loại tiền với mọi dòng khác và với tài khoản.
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

/// Một bút toán mới. `reference` bắt buộc (D-184): duy nhất theo (khách, `source_module`).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NewEntry {
    /// Nút sở hữu sổ (đơn vị kế toán).
    pub book: OrgId,
    /// Ngày hạch toán; bỏ trống: hôm nay theo múi giờ của nút.
    pub effective_date: Option<NaiveDate>,
    /// Chữ cố định của module (đi vào chuỗi băm, không xoá được): **không** chứa tên, email, số
    /// điện thoại — người và khách chỉ nêu bằng ID trong `source_id`.
    pub description: String,
    /// Module ghi (vd `orders`).
    pub source_module: String,
    /// Loại chứng từ nguồn (vd `order`).
    pub source_kind: String,
    /// ID chứng từ nguồn.
    pub source_id: Option<String>,
    pub reference: String,
    pub lines: Vec<NewLine>,
    /// Tỉ giá quy đổi sang tiền hạch toán của sổ (1 tiền của bút toán = `exchange_rate` tiền của sổ),
    /// khi bút toán khác tiền của sổ. Bỏ trống: lấy tỉ giá hiệu lực ngày hạch toán
    /// ([`Ledger::exchange_rate`]). Chứng từ ngoại tệ truyền đúng tỉ giá đã chụp trên chứng từ.
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
    pub fn on(mut self, day: NaiveDate) -> Self {
        self.effective_date = Some(day);
        self
    }

    #[must_use]
    pub fn source_id(mut self, id: impl Into<String>) -> Self {
        self.source_id = Some(id.into());
        self
    }

    /// Quy đổi sang tiền của sổ bằng đúng tỉ giá `rate` (ảnh chụp trên chứng từ).
    #[must_use]
    pub fn exchange_rate(mut self, rate: rust_decimal::Decimal) -> Self {
        self.exchange_rate = Some(rate);
        self
    }
}

/// Đảo một bút toán: bút toán mới đổi nợ / có, ở kỳ đang mở.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NewReversal {
    pub source_module: String,
    pub reference: String,
    /// Bỏ trống: hôm nay theo múi giờ của nút.
    pub effective_date: Option<NaiveDate>,
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
    pub fn on(mut self, day: NaiveDate) -> Self {
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
    pub effective_date: NaiveDate,
    pub posted_at: DateTime<Utc>,
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
    pub sealed_at: Option<DateTime<Utc>>,
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
    pub expires_at: DateTime<Utc>,
    pub captured_at: Option<DateTime<Utc>>,
    /// Module that asked for the payment (`plans`, `orders`…) and its document.
    pub source_module: String,
    pub source_id: Option<String>,
    pub reference: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
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
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Một lượt thu mới. `reference` bắt buộc (D-184): duy nhất theo (khách, `source_module`); gửi lại
/// sau lỗi mạng thì gọi lại đối tác với cùng thanh toán.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NewPayment {
    /// Nút nhận tiền.
    pub org: OrgId,
    /// Đối tác; bỏ trống khi máy chủ chỉ có một cổng.
    pub provider: Option<String>,
    pub method: String,
    pub amount: Money,
    /// Chữ ngắn cho người mua (tối đa 140 ký tự), không dữ liệu cá nhân.
    pub description: String,
    pub return_url: Option<String>,
    pub source_module: String,
    pub source_id: Option<String>,
    pub reference: String,
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
        }
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

/// Một lượt hoàn. `reference` duy nhất theo thanh toán.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NewRefund {
    pub payment: Uuid,
    pub amount: Money,
    /// Chữ ngắn (tối đa 200 ký tự), không dữ liệu cá nhân.
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
    pub updated_at: Option<DateTime<Utc>>,
}

/// Đường tiền của một lượt thu / hoàn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Collection {
    /// Qua cổng thanh toán `provider` (tài khoản `clearing.gateway` của đối tác).
    Gateway { provider: String },
    /// Tiền thu hộ khi giao hàng (COD) qua đơn vị vận chuyển `carrier`.
    Cod { carrier: String },
    /// Chuyển khoản vào tài khoản ngân hàng của người bán / khách (chế độ tự thu).
    BankTransfer { bank_account: Uuid },
}

/// Số tiền của một lượt bán: `gross` = `net` + `vat` (thuế do `TaxEngine` tính, ngoài sổ).
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

/// Một việc của dòng tiền; mỗi việc sinh (tối đa) một bút toán theo chế độ của người bán.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FlowEvent {
    /// Tiền của một lượt bán đã về. Chế độ chia tại cổng: `operator_share` là phần cổng chia cho
    /// sàn (bắt buộc).
    Collected { amounts: SaleAmounts, via: Collection, operator_share: Option<Money> },
    /// Phí sàn `fee` (gồm GTGT `vat` của phí — tách riêng trên sổ và hoá đơn phí).
    FeeCharged { fee: Money, vat: Money },
    /// Khấu trừ, nộp thay thuế của người bán trên doanh thu `revenue` (NĐ 117/2025/NĐ-CP — cần
    /// đối chiếu): sổ hỏi `TaxEngine`; chỉ hộ / cá nhân kinh doanh bị khấu trừ, doanh nghiệp tự kê
    /// khai (không sinh bút toán). `business_line`: `goods`, `services`, `production_transport`,
    /// `other`.
    TaxWithheld { revenue: Money, business_line: String },
    /// Hoàn cho người mua. Trước khi hết hạn đổi trả: giảm khoản tạm giữ. Chế độ chia tại cổng:
    /// `amounts.gross` là phần trả lại từ phần của sàn.
    Refunded { amounts: SaleAmounts, via: Collection },
    /// Phí đối tác thanh toán.
    GatewayFeeCharged { provider: String, fee: Money },
}

/// Một việc của dòng tiền gửi [`Ledger::settle`]. `reference` duy nhất theo (khách,
/// `source_module`) — dùng chung không gian với [`Ledger::post`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Settlement {
    /// Người bán (nút); chế độ đọc theo cài đặt của nút này hay nút trên gần nhất.
    pub seller: OrgId,
    pub event: FlowEvent,
    pub source_module: String,
    /// Chứng từ nguồn (đơn hàng…): khoản tạm giữ gắn theo nó.
    pub source_id: String,
    pub reference: String,
    /// Bỏ trống: hôm nay theo múi giờ của sổ.
    pub effective_date: Option<NaiveDate>,
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
    pub fn on(mut self, day: NaiveDate) -> Self {
        self.effective_date = Some(day);
        self
    }
}

/// Kết quả của [`Ledger::settle`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settled {
    pub mode: FlowMode,
    /// Nút của sổ nhận bút toán (người bán ở chế độ tự thu, sàn ở hai chế độ kia).
    pub book_org: OrgId,
    /// `None`: việc này không sinh bút toán (vd khấu trừ thay với người bán là doanh nghiệp).
    pub entry: Option<Entry>,
    /// Khoản tạm giữ của lượt bán, sau việc này.
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
    pub release_at: DateTime<Utc>,
    /// `held` or `released`.
    pub status: String,
    pub source_module: String,
    pub source_id: String,
    /// The entry that moved the money to the available balance.
    pub release_entry_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
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
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Một lượt chi trả số dư khả dụng của người bán. `reference` duy nhất theo khách.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NewPayout {
    pub payee: OrgId,
    pub amount: Money,
    /// Chữ ngắn (tối đa 140 ký tự), không dữ liệu cá nhân.
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

/// Một lượt thu bằng chuyển khoản VietQR vào tài khoản ngân hàng của nút nhận (chế độ tự thu).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NewTransferPayment {
    pub org: OrgId,
    /// Tài khoản nhận; bỏ trống khi nút chỉ có một tài khoản đang dùng.
    pub bank_account: Option<Uuid>,
    /// Chỉ VND.
    pub amount: Money,
    pub description: String,
    pub source_module: String,
    pub source_id: Option<String>,
    pub reference: String,
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
        }
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
    pub effective_date: NaiveDate,
    /// `manual` (entered by a person) or `provider:<name>` (fetched from a rate source).
    pub source: String,
    pub created_at: DateTime<Utc>,
}

/// Dòng sổ nhật ký chung cần đọc (xuất kế toán): một khoảng ngày, một sổ hay mọi sổ.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct JournalQuery {
    /// Chỉ sổ của nút này; bỏ trống: mọi sổ của khách.
    pub book: Option<OrgId>,
    /// Ngày hạch toán từ (gồm).
    pub from: NaiveDate,
    /// Ngày hạch toán tới (gồm).
    pub to: NaiveDate,
    /// Trang (theo ID dòng tăng dần).
    pub page: autoxide_kernel::pagination::PageRequest,
}

impl JournalQuery {
    pub fn new(from: NaiveDate, to: NaiveDate) -> Self {
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
    pub effective_date: NaiveDate,
    pub posted_at: DateTime<Utc>,
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

/// Ghi và đọc sổ cái.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait Ledger: Send + Sync {
    /// Ghi một bút toán. Cùng `reference` + cùng nội dung → `Replayed` (không ghi thêm); cùng
    /// `reference`, nội dung khác → `reference_reused`.
    async fn post(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        entry: NewEntry,
    ) -> Result<Submitted<Entry>, Error>;

    /// Đảo bút toán `entry` (một lần): `ledger.already_reversed` khi đã đảo.
    async fn reverse(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        entry: Uuid,
        reversal: NewReversal,
    ) -> Result<Submitted<Entry>, Error>;

    /// ID tài khoản mang khoá `key`, mở nếu chưa có (chỉ mục đích cơ bản).
    async fn ensure_account(&self, tenant: &TenantContext, key: AccountKey) -> Result<Uuid, Error>;

    /// Mở (hay trả lại) tài khoản với bên thường / sàn / tên tự chọn.
    async fn open_account(
        &self,
        tenant: &TenantContext,
        account: OpenAccount,
    ) -> Result<Uuid, Error>;

    /// Số dư (theo bên thường) của tài khoản mang khoá `key`: hiện tại, hay cuối ngày `as_of`.
    /// Chưa có tài khoản: 0.
    async fn balance(
        &self,
        tenant: &TenantContext,
        key: AccountKey,
        as_of: Option<NaiveDate>,
    ) -> Result<Money, Error>;

    /// Một bút toán, hoặc `None`.
    async fn entry(&self, tenant: &TenantContext, id: Uuid) -> Result<Option<Entry>, Error>;

    /// Tạo lượt thu qua đối tác: trả `checkout_url` / `qr_payload` cho người mua. Đối tác báo đã
    /// thu (webhook, đối soát) thì sổ ghi bút toán và phát event `ledger.payment_succeeded`
    /// (`payment_id`, `source_module`, `source_id`, `reference`, `amount`) cho module nguồn.
    async fn create_payment(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        payment: NewPayment,
    ) -> Result<Submitted<Payment>, Error>;

    /// Hoàn (một phần) một thanh toán đã thu; không vượt số còn lại.
    async fn refund(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        refund: NewRefund,
    ) -> Result<Submitted<Refund>, Error>;

    /// Một thanh toán, hoặc `None`.
    async fn payment(&self, tenant: &TenantContext, id: Uuid) -> Result<Option<Payment>, Error>;

    /// Số dư (theo bên thường) của nhiều tài khoản một lượt (tối đa 500 khoá), cùng thứ tự với
    /// `keys` — vd công nợ nhà cung cấp (`payable.supplier`, đối tượng `supplier`). Chưa có: 0.
    async fn balances(
        &self,
        tenant: &TenantContext,
        keys: &[AccountKey],
        as_of: Option<NaiveDate>,
    ) -> Result<Vec<Money>, Error>;

    /// Cài đặt dòng tiền hiệu lực của nút `org` (kế thừa theo cây; chưa đặt: tự thu).
    async fn flow_settings(
        &self,
        tenant: &TenantContext,
        org: OrgId,
    ) -> Result<FlowSettings, Error>;

    /// Ghi một việc của dòng tiền theo chế độ của người bán: tiền vào, phí sàn, khấu trừ thay, hoàn,
    /// phí cổng. Ở chế độ thu hộ, tiền vào tạo khoản tạm giữ tới hết hạn đổi trả (job
    /// `ledger.release_holds` chuyển sang số dư khả dụng). Việc không hợp với chế độ hay cổng →
    /// `ledger.flow_mode_unsupported`. Cùng `reference` + cùng nội dung → `Replayed`.
    async fn settle(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        settlement: Settlement,
    ) -> Result<Submitted<Settled>, Error>;

    /// Khoản tạm giữ của chứng từ nguồn (`marketplace` hỏi trước khi cho rút tiền).
    async fn hold_status(
        &self,
        tenant: &TenantContext,
        source_module: &str,
        source_id: &str,
    ) -> Result<Option<Hold>, Error>;

    /// Chi trả số dư khả dụng của người bán qua `PayoutGateway` (chế độ thu hộ). Trừ số dư trước,
    /// gọi đối tác ngoài transaction; đối tác từ chối → ghi lại số dư. Không vượt số dư khả dụng,
    /// kể cả khi gửi đồng thời (`ledger.insufficient_balance`).
    async fn create_payout(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        payout: NewPayout,
    ) -> Result<Submitted<Payout>, Error>;

    /// Tạo lượt thu chuyển khoản VietQR (mã QR mang mã chuyển khoản; tiền vào thẳng tài khoản của
    /// nút nhận). Biến động số dư (`BankFeed`) khớp mã + số tiền thì ghi đã thu; sai lệch vào hàng
    /// chờ. Cùng hạn thanh toán với mọi thanh toán (`expires_at`).
    async fn create_transfer_payment(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        payment: NewTransferPayment,
    ) -> Result<Submitted<Payment>, Error>;

    /// Tỉ giá 1 `base` = ? `quote` hiệu lực ngày `on` (D-237): tỉ giá có ngày hiệu lực gần nhất
    /// ≤ `on`; cùng ngày thì tỉ giá nhập tay trước tỉ giá của nguồn. Chưa có: `None`. Đọc, không
    /// gọi trong transaction ghi. `invoices`, báo cáo gộp (38) và đa tiền tệ (63) đọc qua đây.
    async fn exchange_rate(
        &self,
        tenant: &TenantContext,
        base: Currency,
        quote: Currency,
        on: NaiveDate,
    ) -> Result<Option<ExchangeRate>, Error>;

    /// Tiền hạch toán của sổ của nút `org` (sổ chưa mở: tiền mặc định của nút, hay VND).
    async fn book_currency(&self, tenant: &TenantContext, org: OrgId) -> Result<Currency, Error>;

    /// Một trang sổ nhật ký chung (dòng bút toán kèm bút toán và tài khoản) cho xuất kế toán.
    async fn journal(
        &self,
        tenant: &TenantContext,
        query: JournalQuery,
    ) -> Result<autoxide_kernel::pagination::Page<JournalLine>, Error>;
}
