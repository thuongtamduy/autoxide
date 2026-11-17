//! Hợp đồng của module `ledger` cho module khác: ghi bút toán kép, đảo bút toán, mở tài khoản theo
//! mục đích, đọc số dư. Module khác lấy lúc khởi động: `context.require::<dyn Ledger>()` (đăng ký
//! SAU `ledger`).
//!
//! Sổ cái chỉ ghi **nghĩa vụ** (D-187): không có tài khoản ví, không giữ tiền của người mua. Số dư
//! suy từ bút toán; bút toán chỉ thêm, sửa sai bằng bút toán đảo.

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
pub const PURPOSES: [(&str, Side); 21] = [
    ("cash.bank", Side::Debit),
    ("cash.on_hand", Side::Debit),
    ("clearing.gateway", Side::Debit),
    ("clearing.carrier_cod", Side::Debit),
    ("receivable.customer", Side::Debit),
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
    /// `org`, `customer`, `carrier`, `partner`, or `<module>.<kind>`.
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
}
