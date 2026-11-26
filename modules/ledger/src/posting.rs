//! Ghi bút toán và đảo bút toán (luật ghi ở đặc tả bước 36, mục "Sổ cái: luật ghi").
//!
//! Thứ tự trong transaction (giống nhau ở mọi đường ghi để không deadlock):
//!
//! 1. tìm bút toán đã có cùng `reference` (gửi lại → trả bản cũ);
//! 2. **khoá chia sẻ dòng sổ** (`rows::lock_one_shared`): nhiều bút toán cùng ghi một sổ được, còn
//!    khoá kỳ lấy khoá độc quyền trên dòng sổ nên phải chờ chúng — và bút toán đến sau chờ khoá kỳ
//!    xong rồi thấy `closed_through` mới. Ngày ≤ `closed_through` → `ledger.period_closed`;
//! 3. mở kỳ (tháng) nếu chưa có (`ON CONFLICT DO NOTHING`);
//! 4. tìm / mở tài khoản theo khoá, **theo thứ tự khoá** (hai bên mở cùng hai tài khoản mới theo
//!    thứ tự ngược nhau sẽ chờ nhau);
//! 5. cộng số dư từng tài khoản bằng `rows::adjust`, **theo `account_id` tăng dần**, chặn sàn;
//! 6. chèn bút toán, dòng, event; commit.

use autoxide_kernel::time::Date;
use std::collections::BTreeMap;

use autoxide_kernel::access::{OrgTarget, Permission, Principal};
use autoxide_kernel::decimal::{self, Decimal, Rounding};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::reference::{self, Reference, Submitted};
use autoxide_kernel::rows::{self, Bounds};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, Set};
use serde_json::json;
use uuid::Uuid;

use crate::api::{AccountKey, Entry, LineAccount, NewEntry, NewLine, NewReversal, Side};
use crate::entity::{account, balance, book, entry, line, period};
use crate::events;
use crate::permissions::{POST, REVERSE};
use crate::service::{
    LedgerService, account_closed, account_not_found, check_key, currency_mixed, describe_entry,
    entry_not_found, exchange_rate_missing, now, valid_id, valid_name,
};

pub(crate) fn period_closed(on: Date) -> Error {
    Error::conflict(
        "ledger.period_closed",
        "The accounting period holding {date} is closed. Post an adjusting entry in an open period.",
    )
    .with_param("date", on.format("%Y-%m-%d").to_string())
}

fn unbalanced(debit: Money, credit: Money) -> Error {
    Error::new(
        ErrorKind::Validation,
        "ledger.unbalanced",
        "The entry does not balance: debits total {debit} but credits total {credit}.",
    )
    .with_param("debit", debit)
    .with_param("credit", credit)
}

pub(crate) fn insufficient_balance() -> Error {
    Error::conflict(
        "ledger.insufficient_balance",
        "The account balance is not enough for this entry.",
    )
}

fn amount_too_large() -> Error {
    Error::new(
        ErrorKind::Validation,
        "ledger.amount_too_large",
        "The amounts are too large for the account balance.",
    )
}

fn already_reversed() -> Error {
    Error::conflict("ledger.already_reversed", "This entry was already reversed.")
}

/// Ngày đầu và ngày cuối của kỳ (tháng) chứa `day`.
pub(crate) fn month_of(day: Date) -> (Date, Date) {
    let first = day.first_of_month();
    let next = if first.month() == 12 {
        Date::new(first.year() + 1, 1, 1)
    } else {
        Date::new(first.year(), first.month() + 1, 1)
    };
    let last = next.and_then(|next| next.pred()).unwrap_or(first);
    (first, last)
}

/// Đầu vào đã kiểm của một bút toán.
struct Checked {
    description: String,
    reference: Reference,
    fingerprint: String,
    currency: Currency,
}

/// Kiểm bút toán (không I/O): mọi lỗi đầu vào gom một lần, cân nợ / có, cùng loại tiền.
fn checked(entry: &NewEntry, reverses: Option<Uuid>) -> Result<Checked, Error> {
    let mut problems = Vec::new();
    let description = entry.description.trim().to_owned();
    if !(1..=200).contains(&description.chars().count()) {
        problems.push(FieldError::new(
            "description",
            "length",
            "must be 1 to 200 characters long (fixed text, no personal data)",
        ));
    }
    if !valid_name(&entry.source_module) {
        problems.push(FieldError::new("source_module", "format", "must be a module name"));
    }
    if !valid_name(&entry.source_kind) {
        problems.push(FieldError::new(
            "source_kind",
            "format",
            "must be lowercase words joined by `.`",
        ));
    }
    if entry.source_id.as_deref().is_some_and(|id| !valid_id(id)) {
        problems.push(FieldError::new(
            "source_id",
            "format",
            "must be 1 to 100 visible ASCII characters",
        ));
    }
    let reference = Reference::checked(&entry.reference, "reference", &mut problems);
    if entry.lines.len() < 2 {
        problems.push(FieldError::new("lines", "count", "an entry needs at least two lines"));
    }
    for (index, item) in entry.lines.iter().enumerate() {
        if !item.amount.is_positive() {
            problems.push(FieldError::new(
                format!("lines[{index}].amount"),
                "range",
                "must be greater than zero",
            ));
        }
        if let LineAccount::Key(key) = &item.account {
            check_key(key, &format!("lines[{index}].account."), &mut problems);
            if key.book != entry.book {
                problems.push(FieldError::new(
                    format!("lines[{index}].account.book"),
                    "book",
                    "must be the book of the entry",
                ));
            }
        }
    }
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    let Some(reference) = reference else {
        return Err(Error::internal("a checked reference went missing"));
    };
    let currency = entry.lines[0].amount.currency();
    let mixed = entry.lines.iter().any(|item| {
        item.amount.currency() != currency
            || matches!(&item.account, LineAccount::Key(key) if key.currency != currency)
    });
    if mixed {
        return Err(currency_mixed());
    }
    let total = |side: Side| {
        Money::sum(
            currency,
            entry.lines.iter().filter(|item| item.side == side).map(|item| item.amount),
        )
    };
    let (debit, credit) = (total(Side::Debit)?, total(Side::Credit)?);
    if debit != credit {
        return Err(unbalanced(debit, credit));
    }
    let fingerprint = fingerprint_of(entry, reverses)?;
    Ok(Checked { description, reference, fingerprint, currency })
}

/// Bút toán sẵn sàng ghi vào sổ `book` (đã có) ngày `effective_date` — không I/O, dùng được trong
/// transaction ghi của module (dòng tiền ghi khoản tạm giữ và bút toán cùng một transaction).
pub(crate) fn draft_for(
    entry: NewEntry,
    book: book::Model,
    effective_date: Date,
    reverses: Option<Uuid>,
    guard: Option<(Principal, Permission)>,
    rate: Option<Decimal>,
) -> Result<Draft, Error> {
    let Checked { description, reference, fingerprint, currency } = checked(&entry, reverses)?;
    let rate = if book.base_currency == currency.code() {
        None
    } else {
        let base = Currency::parse(&book.base_currency)?;
        Some(rate.ok_or_else(|| exchange_rate_missing(currency, base, effective_date))?)
    };
    Ok(Draft {
        book,
        effective_date,
        description,
        source_module: entry.source_module,
        source_kind: entry.source_kind,
        source_id: entry.source_id,
        reference,
        fingerprint,
        currency,
        lines: entry.lines,
        reverses,
        guard,
        rate,
    })
}

/// Ghi bút toán `draft` trong transaction ghi `transaction` của bên gọi (không commit).
pub(crate) async fn write_in(
    transaction: &DatabaseTransaction,
    tenant: &TenantContext,
    actor: Actor,
    draft: &Draft,
    outbox: &autoxide_kernel::event::Outbox,
) -> Result<Submitted<Entry>, Error> {
    write(transaction, tenant, actor, draft, outbox).await
}

/// Sàn mặc định khi mở tài khoản theo khoá: nghĩa vụ với người bán không bao giờ âm (không chi,
/// trừ phí, hoàn quá số đang có).
fn default_floor(purpose: &str) -> Option<i64> {
    matches!(purpose, "payable.seller_held" | "payable.seller_available").then_some(0)
}

/// Bút toán đã kiểm, sẵn sàng ghi.
pub(crate) struct Draft {
    book: book::Model,
    effective_date: Date,
    description: String,
    source_module: String,
    source_kind: String,
    source_id: Option<String>,
    reference: Reference,
    fingerprint: String,
    currency: Currency,
    lines: Vec<NewLine>,
    reverses: Option<Uuid>,
    /// REST / MCP: người gọi phải có permission này ở nút của sổ, nút không bị đóng băng — kiểm
    /// TRONG transaction ghi (`require_write_at` khoá dòng của cây tổ chức).
    guard: Option<(Principal, Permission)>,
    /// Bút toán ngoại tệ: 1 tiền của bút toán = `rate` tiền của sổ (ảnh chụp lưu trên từng dòng).
    rate: Option<Decimal>,
}

/// Tiền của sổ cho từng dòng của bút toán ngoại tệ: quy đổi TỔNG nợ (= tổng có) một lần, làm tròn
/// `HalfUp` tới đơn vị nhỏ nhất của tiền sổ, rồi chia về các dòng mỗi bên theo tỉ lệ bằng
/// `Money::allocate` — bút toán cân ở cả tiền gốc và tiền sổ, không làm tròn hai lần.
fn base_amounts(lines: &[NewLine], rate: Decimal, base: Currency) -> Result<Vec<Money>, Error> {
    let mut out = vec![Money::zero(base); lines.len()];
    for side in [Side::Debit, Side::Credit] {
        let picked: Vec<usize> =
            (0..lines.len()).filter(|index| lines[*index].side == side).collect();
        let Some(first) = picked.first() else { continue };
        let total = Money::sum(
            lines[*first].amount.currency(),
            picked.iter().map(|index| lines[*index].amount),
        )?;
        let converted = total.convert(rate, base, Rounding::HalfUp)?;
        let ratios: Vec<u64> =
            picked.iter().map(|index| lines[*index].amount.minor().unsigned_abs()).collect();
        for (index, share) in picked.iter().zip(converted.allocate(&ratios)?) {
            out[*index] = share;
        }
    }
    Ok(out)
}

/// Khoá sắp xếp của một khoá tài khoản (thứ tự mở tài khoản trong transaction).
fn key_order(key: &AccountKey) -> (String, String, String, &'static str) {
    let (kind, id) = key
        .counterparty
        .as_ref()
        .map_or((String::new(), String::new()), |party| (party.kind.clone(), party.id.clone()));
    (key.purpose.clone(), kind, id, key.currency.code())
}

fn actor_kind(actor: &Actor) -> &'static str {
    match (actor.user_id, actor.api_key_id) {
        (_, Some(_)) => "api_key",
        (Some(_), None) => "user",
        (None, None) => "system",
    }
}

/// Nội dung quyết định kết quả của lệnh (cho dấu vân tay của `reference`).
fn fingerprint_of(entry: &NewEntry, reverses: Option<Uuid>) -> Result<String, Error> {
    let lines: Vec<serde_json::Value> = entry
        .lines
        .iter()
        .map(|line| {
            let account = match &line.account {
                LineAccount::Id(id) => json!({"id": id}),
                LineAccount::Key(key) => json!({
                    "purpose": key.purpose,
                    "counterparty": key.counterparty.as_ref().map(|party| json!([party.kind, party.id])),
                    "currency": key.currency.code(),
                }),
            };
            json!({"account": account, "side": line.side.as_str(), "amount": line.amount.minor(),
                   "currency": line.amount.currency().code()})
        })
        .collect();
    let mut content = json!({
        "book": entry.book.as_uuid(),
        "effective_date": entry.effective_date,
        "description": entry.description,
        "source_kind": entry.source_kind,
        "source_id": entry.source_id,
        "reverses": reverses,
        "lines": lines,
    });
    // Chỉ thêm khi có: dấu vân tay của bút toán cũ (không tỉ giá) giữ nguyên.
    if let Some(rate) = entry.exchange_rate {
        content["exchange_rate"] = json!(decimal::to_column(rate));
    }
    reference::fingerprint(&content)
}

impl LedgerService {
    /// Ghi bút toán `entry` (REST, MCP, `dyn Ledger`).
    pub(crate) async fn post(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        entry: NewEntry,
    ) -> Result<Submitted<Entry>, Error> {
        self.post_inner(tenant, actor, entry, None, None).await
    }

    /// Ghi bút toán thủ công qua REST / MCP: cần `ledger.post` ở nút của sổ.
    pub(crate) async fn post_manual(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        entry: NewEntry,
    ) -> Result<Submitted<Entry>, Error> {
        principal.require(&POST)?;
        principal
            .require_at(tenant.database().read(), &POST, OrgTarget::new(entry.book, None))
            .await?;
        let guard = Some((principal.clone(), POST));
        self.post_inner(tenant, Actor::from(principal), entry, None, guard).await
    }

    async fn post_inner(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        entry: NewEntry,
        reverses: Option<Uuid>,
        guard: Option<(Principal, Permission)>,
    ) -> Result<Submitted<Entry>, Error> {
        // Kiểm trước khi mở sổ: đầu vào sai không để lại sổ rỗng.
        checked(&entry, reverses)?;
        let book = self.ensure_book(tenant, entry.book).await?;
        let effective_date = match entry.effective_date {
            Some(day) => day,
            None => self.today(tenant, entry.book).await,
        };
        // Bút toán ngoại tệ: tỉ giá của bên gọi (ảnh chụp trên chứng từ) hay tỉ giá hiệu lực ngày
        // hạch toán — đọc TRƯỚC transaction ghi.
        let currency = entry.lines.first().map(|item| item.amount.currency());
        let rate = match (entry.exchange_rate, currency) {
            (Some(rate), _) => Some(rate),
            (None, Some(currency)) if currency.code() != book.base_currency => {
                let base = Currency::parse(&book.base_currency)?;
                self.exchange_rate(tenant, currency, base, effective_date)
                    .await?
                    .map(|found| decimal::from_column(&found.rate))
                    .transpose()?
            }
            _ => None,
        };
        let draft = draft_for(entry, book, effective_date, reverses, guard, rate)?;
        let (database, draft) = (tenant.database(), &draft);
        reference::submit_once(database, move || async move {
            let transaction = database.begin_write().await?;
            let outcome = write(&transaction, tenant, actor, draft, &self.outbox).await?;
            transaction.commit().await?;
            Ok(outcome)
        })
        .await
    }

    /// Đảo bút toán `id` (`dyn Ledger`).
    pub(crate) async fn reverse(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
        reversal: NewReversal,
    ) -> Result<Submitted<Entry>, Error> {
        self.reverse_inner(tenant, actor, id, reversal, None).await
    }

    async fn reverse_inner(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
        reversal: NewReversal,
        guard: Option<(Principal, Permission)>,
    ) -> Result<Submitted<Entry>, Error> {
        let original = entry::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(entry_not_found)?;
        let lines = line::Entity::find()
            .scoped(tenant)
            .filter(line::Column::EntryId.eq(original.id))
            .all(tenant.database().read())
            .await?;
        let mut flipped = Vec::with_capacity(lines.len());
        for row in &lines {
            let side = Side::parse(&row.side)
                .ok_or_else(|| Error::internal("ledger line has a bad side"))?;
            flipped.push(NewLine {
                account: LineAccount::Id(row.account_id),
                side: side.opposite(),
                amount: Money::from_columns(row.amount_minor, &row.amount_currency)?,
            });
        }
        let mut entry = NewEntry::new(
            OrgId::from_uuid(original.org_id),
            reversal.source_module,
            "reversal",
            reversal.reference,
            "Reversal of an earlier entry",
            flipped,
        )
        .source_id(original.id.to_string());
        entry.effective_date = reversal.effective_date;
        // Bút toán ngoại tệ đảo bằng đúng tỉ giá của bút toán gốc: hai bút toán triệt tiêu nhau ở cả
        // tiền sổ.
        entry.exchange_rate = lines
            .iter()
            .find_map(|row| row.exchange_rate.as_deref())
            .map(decimal::from_column)
            .transpose()?;
        self.post_inner(tenant, actor, entry, Some(original.id), guard).await
    }

    /// Đảo bút toán qua REST / MCP: cần `ledger.reverse` ở nút của sổ.
    pub(crate) async fn reverse_manual(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        reversal: NewReversal,
    ) -> Result<Submitted<Entry>, Error> {
        principal.require(&REVERSE)?;
        let read = tenant.database().read();
        let original = entry::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(entry_not_found)?;
        principal
            .require_at(read, &REVERSE, OrgTarget::new(OrgId::from_uuid(original.org_id), None))
            .await
            .map_err(
                |error| if error.code() == "org_out_of_scope" { entry_not_found() } else { error },
            )?;
        let guard = Some((principal.clone(), REVERSE));
        self.reverse_inner(tenant, Actor::from(principal), id, reversal, guard).await
    }
}

/// Mở kỳ chứa `day` của sổ `book` nếu chưa có, trả dòng kỳ.
pub(crate) async fn ensure_period(
    transaction: &DatabaseTransaction,
    tenant: &TenantContext,
    book: &book::Model,
    day: Date,
) -> Result<period::Model, Error> {
    let (starts_on, ends_on) = month_of(day);
    let find = || {
        period::Entity::find()
            .scoped(tenant)
            .filter(period::Column::BookId.eq(book.id))
            .filter(period::Column::StartsOn.eq(starts_on))
    };
    if let Some(found) = find().one(transaction).await? {
        return Ok(found);
    }
    period::Entity::insert(period::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant.id().as_uuid()),
        book_id: Set(book.id),
        starts_on: Set(starts_on),
        ends_on: Set(ends_on),
        status: Set("open".to_owned()),
        closed_at: Set(None),
        closed_by: Set(None),
    })
    .on_conflict(
        OnConflict::columns([
            period::Column::TenantId,
            period::Column::BookId,
            period::Column::StartsOn,
        ])
        .do_nothing()
        .to_owned(),
    )
    .exec_without_returning(transaction)
    .await?;
    find().one(transaction).await?.ok_or_else(|| Error::internal("a ledger period vanished"))
}

/// Một lần chạy của lệnh ghi (chịu được chạy lại: mọi thứ đọc lại trong transaction mới).
async fn write(
    transaction: &DatabaseTransaction,
    tenant: &TenantContext,
    actor: Actor,
    draft: &Draft,
    outbox: &autoxide_kernel::event::Outbox,
) -> Result<Submitted<Entry>, Error> {
    // 1. Đã có cùng `reference`?
    let stored = entry::Entity::find()
        .scoped(tenant)
        .filter(entry::Column::SourceModule.eq(draft.source_module.as_str()))
        .filter(entry::Column::Reference.eq(draft.reference.as_str()))
        .one(transaction)
        .await?
        .map(|row| {
            let fingerprint = row.reference_fingerprint.clone();
            (row, fingerprint)
        });
    if let Some(existing) = reference::replayed(stored, &draft.reference, &draft.fingerprint)? {
        return Ok(Submitted::Replayed(describe_entry(transaction, tenant, existing).await?));
    }
    // Đảo: mỗi bút toán một lần.
    if let Some(original) = draft.reverses {
        let taken = entry::Entity::find()
            .scoped(tenant)
            .filter(entry::Column::ReversesEntryId.eq(original))
            .one(transaction)
            .await?;
        if taken.is_some() {
            return Err(already_reversed());
        }
    }
    if let Some((principal, permission)) = &draft.guard {
        let target = OrgTarget::new(OrgId::from_uuid(draft.book.org_id), None);
        principal.require_write_at(transaction, permission, target).await?;
    }
    // 2. Khoá chia sẻ dòng sổ, đọc lại `closed_through`.
    let book =
        rows::lock_one_shared(transaction, book::Entity::find_by_id(draft.book.id).scoped(tenant))
            .await?
            .ok_or_else(|| Error::internal("a ledger book vanished"))?;
    if book.closed_through.is_some_and(|through| draft.effective_date <= through) {
        return Err(period_closed(draft.effective_date));
    }
    // 3. Kỳ.
    let period = ensure_period(transaction, tenant, &book, draft.effective_date).await?;
    if period.status != "open" {
        return Err(period_closed(draft.effective_date));
    }
    // 4. Tài khoản: theo ID, hay theo khoá (mở theo thứ tự khoá).
    let mut keys: Vec<&AccountKey> = draft
        .lines
        .iter()
        .filter_map(|item| match &item.account {
            LineAccount::Key(key) => Some(key),
            LineAccount::Id(_) => None,
        })
        .collect();
    keys.sort_by_key(|key| key_order(key));
    keys.dedup();
    let mut by_key = Vec::with_capacity(keys.len());
    for key in keys {
        let (found, _) = LedgerService::find_or_open(
            transaction,
            tenant,
            &book,
            key,
            None,
            default_floor(&key.purpose),
            None,
        )
        .await?;
        by_key.push((key.clone(), found));
    }
    let ids: Vec<Uuid> = draft
        .lines
        .iter()
        .filter_map(|item| match item.account {
            LineAccount::Id(id) => Some(id),
            LineAccount::Key(_) => None,
        })
        .collect();
    let by_id: Vec<account::Model> = if ids.is_empty() {
        Vec::new()
    } else {
        account::Entity::find()
            .scoped(tenant)
            .filter(account::Column::Id.is_in(ids))
            .all(transaction)
            .await?
    };
    // Khoá các tài khoản (ID tăng dần, chống deadlock) rồi mới xét trạng thái và sàn (D-181):
    // đóng tài khoản hay nâng sàn cùng lúc (`update_account` khoá dòng tài khoản trước) thì bút toán
    // này chờ và thấy giá trị mới — không ghi lọt vào tài khoản vừa đóng, không trừ dưới sàn mới.
    let mut account_ids: Vec<Uuid> =
        by_id.iter().map(|row| row.id).chain(by_key.iter().map(|(_, row)| row.id)).collect();
    account_ids.sort_unstable();
    account_ids.dedup();
    let locked = rows::lock(
        transaction,
        account::Entity::find().scoped(tenant).filter(account::Column::Id.is_in(account_ids)),
        account::Column::Id,
    )
    .await?;
    let mut resolved = Vec::with_capacity(draft.lines.len());
    for item in &draft.lines {
        let found = match &item.account {
            LineAccount::Id(id) => Some(*id),
            LineAccount::Key(key) => {
                by_key.iter().find(|(stored, _)| stored == key).map(|(_, row)| row.id)
            }
        };
        let found = found.and_then(|id| locked.iter().find(|row| row.id == id));
        let Some(found) = found.filter(|row| row.book_id == book.id) else {
            return Err(account_not_found());
        };
        if found.status != "active" {
            return Err(account_closed());
        }
        if found.currency != draft.currency.code() {
            return Err(currency_mixed());
        }
        resolved.push(found.clone());
    }
    // 5. Số dư theo bên thường, theo `account_id` tăng dần (`BTreeMap` giữ thứ tự khoá).
    let mut deltas: BTreeMap<Uuid, (i64, Option<i64>)> = BTreeMap::new();
    for (item, found) in draft.lines.iter().zip(&resolved) {
        let normal = Side::parse(&found.normal_side)
            .ok_or_else(|| Error::internal("ledger account has a bad side"))?;
        let signed = if item.side == normal { item.amount.minor() } else { -item.amount.minor() };
        let slot = deltas.entry(found.id).or_insert((0, found.floor_minor));
        slot.0 = slot.0.checked_add(signed).ok_or_else(amount_too_large)?;
    }
    for (id, (delta, floor)) in deltas {
        if delta == 0 {
            continue;
        }
        let bounds = Bounds { floor: floor.unwrap_or(i64::MIN), ceiling: i64::MAX };
        let changed = rows::adjust::<balance::Entity, _>(
            transaction,
            balance::Column::BalanceMinor,
            balance::Column::AccountId.eq(id),
            delta,
            bounds,
        )
        .await?;
        if changed == 0 {
            return Err(if floor.is_some() { insufficient_balance() } else { amount_too_large() });
        }
    }
    // 6. Bút toán, dòng, event.
    let entry_id = Uuid::now_v7();
    let row = entry::ActiveModel {
        id: Set(entry_id),
        tenant_id: Set(tenant.id().as_uuid()),
        book_id: Set(book.id),
        org_id: Set(book.org_id),
        period_id: Set(period.id),
        effective_date: Set(draft.effective_date),
        posted_at: Set(now()),
        description: Set(draft.description.clone()),
        source_module: Set(draft.source_module.clone()),
        source_kind: Set(draft.source_kind.clone()),
        source_id: Set(draft.source_id.clone()),
        reference: Set(draft.reference.as_str().to_owned()),
        reference_fingerprint: Set(draft.fingerprint.clone()),
        reverses_entry_id: Set(draft.reverses),
        actor_user_id: Set(actor.user_id),
        actor_kind: Set(actor_kind(&actor).to_owned()),
    };
    entry::Entity::insert(row).exec_without_returning(transaction).await?;
    let base_currency = Currency::parse(&book.base_currency)?;
    let bases = match draft.rate {
        Some(rate) => base_amounts(&draft.lines, rate, base_currency)?,
        None => draft.lines.iter().map(|item| item.amount).collect(),
    };
    let rate_text = draft.rate.map(decimal::to_column);
    let lines: Vec<line::ActiveModel> = draft
        .lines
        .iter()
        .zip(&resolved)
        .zip(&bases)
        .enumerate()
        .map(|(index, ((item, found), base))| line::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            entry_id: Set(entry_id),
            line_no: Set(i32::try_from(index + 1).unwrap_or(i32::MAX)),
            account_id: Set(found.id),
            side: Set(item.side.as_str().to_owned()),
            amount_minor: Set(item.amount.minor()),
            amount_currency: Set(draft.currency.code().to_owned()),
            base_minor: Set(base.minor()),
            base_currency: Set(book.base_currency.clone()),
            exchange_rate: Set(rate_text.clone()),
            effective_date: Set(draft.effective_date),
        })
        .collect();
    line::Entity::insert_many(lines).exec_without_returning(transaction).await?;
    let total = Money::sum(
        draft.currency,
        draft.lines.iter().filter(|item| item.side == Side::Debit).map(|item| item.amount),
    )?;
    let subject = Some(format!("ledger.entry:{entry_id}"));
    let day = draft.effective_date.format("%Y-%m-%d").to_string();
    // Dòng theo mục đích tài khoản (không có đối tượng): báo cáo gộp (bước 38) tính doanh thu, thuế.
    let event_lines: Vec<serde_json::Value> = draft
        .lines
        .iter()
        .zip(&resolved)
        .map(|(item, found)| {
            json!({"purpose": found.purpose, "side": item.side.as_str(), "amount": item.amount})
        })
        .collect();
    match draft.reverses {
        Some(original) => {
            let event = events::EntryReversed(json!({
                "entry_id": entry_id, "reversed_entry_id": original, "book_id": book.id,
                "org_id": book.org_id, "effective_date": day, "lines": event_lines,
            }));
            outbox.record(transaction, tenant, actor, subject, &event).await?;
        }
        None => {
            let event = events::EntryPosted(json!({
                "entry_id": entry_id, "book_id": book.id, "org_id": book.org_id,
                "effective_date": day, "source_module": draft.source_module,
                "source_kind": draft.source_kind, "source_id": draft.source_id,
                "reference": draft.reference.as_str(), "total": total, "lines": event_lines,
            }));
            outbox.record(transaction, tenant, actor, subject, &event).await?;
        }
    }
    let saved = entry::Entity::find_by_id(entry_id)
        .scoped(tenant)
        .one(transaction)
        .await?
        .ok_or_else(|| Error::internal("a ledger entry vanished after it was written"))?;
    Ok(Submitted::Created(describe_entry(transaction, tenant, saved).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ky_la_thang_chua_ngay() {
        let day = |y, m, d| Date::new(y, m, d).unwrap();
        assert_eq!(month_of(day(2026, 2, 14)), (day(2026, 2, 1), day(2026, 2, 28)));
        assert_eq!(month_of(day(2028, 2, 29)), (day(2028, 2, 1), day(2028, 2, 29)));
        assert_eq!(month_of(day(2026, 12, 31)), (day(2026, 12, 1), day(2026, 12, 31)));
        assert_eq!(month_of(day(2027, 1, 1)), (day(2027, 1, 1), day(2027, 1, 31)));
    }
}

#[cfg(test)]
#[path = "posting_properties.rs"]
mod properties;
