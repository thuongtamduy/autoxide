//! Service của module `ledger`: sổ, tài khoản, đọc bút toán. Ghi bút toán ở [`crate::posting`], kỳ
//! và số dư ở [`crate::periods`], chuỗi băm ở [`crate::chain`]. Handler REST, tool MCP và bên cung
//! cấp `dyn Ledger` chỉ gọi vào đây.

use std::collections::HashMap;
use std::sync::Arc;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Outbox;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{DatabaseRegistry, OrgId, ScopeExt, TenantContext};
use autoxide_organizations::api::OrgDirectory;
use chrono::{DateTime, NaiveDate, SubsecRound, Utc};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set};

use crate::api::{AccountKey, Entry, EntryLine, Side, normal_side_of};
use crate::entity::{account, balance, book, entry, line};
use crate::permissions::{ACCOUNTS_MANAGE, READ};

pub(crate) fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

pub(crate) struct LedgerService {
    pub databases: DatabaseRegistry,
    pub outbox: Outbox,
    /// Múi giờ và tiền tệ mặc định của nút (module `organizations`); không có thì UTC và VND.
    pub directory: Option<Arc<dyn OrgDirectory>>,
    /// Cổng thanh toán các module bộ nối góp vào (`StartContext::contribute`).
    pub gateways: Vec<Arc<dyn autoxide_kernel::payment::PaymentGateway>>,
    /// Đối tác chi hộ (`StartContext::contribute`).
    pub payout_gateways: Vec<Arc<dyn autoxide_kernel::payment::PayoutGateway>>,
    /// Nguồn biến động số dư ngân hàng (`StartContext::contribute`).
    pub bank_feeds: Vec<Arc<dyn autoxide_kernel::payment::BankFeed>>,
    /// Engine thuế (module `tax`, đăng ký trước) cho khấu trừ thay; không có thì không khấu trừ được.
    pub tax: Option<Arc<dyn autoxide_tax::api::TaxEngine>>,
    pub jobs: autoxide_kernel::job::JobQueue,
    pub config: crate::config::LedgerConfig,
}

// ---- Lỗi ----------------------------------------------------------------------------------------

pub(crate) fn book_not_found() -> Error {
    Error::not_found(
        "ledger.book_not_found",
        "This unit has no book, or it is outside your scope. Open one with `POST /api/v1/ledger/books`.",
    )
}

pub(crate) fn account_not_found() -> Error {
    Error::not_found("ledger.account_not_found", "There is no such ledger account.")
}

pub(crate) fn entry_not_found() -> Error {
    Error::not_found("ledger.entry_not_found", "There is no such ledger entry.")
}

pub(crate) fn account_closed() -> Error {
    Error::conflict("ledger.account_closed", "The account is closed and takes no new entries.")
}

pub(crate) fn currency_mixed() -> Error {
    Error::new(
        ErrorKind::Validation,
        "ledger.currency_mixed",
        "Every line of an entry, and its account, must use the same currency.",
    )
}

pub(crate) fn exchange_rate_missing(base: Currency, quote: Currency, on: NaiveDate) -> Error {
    Error::new(
        ErrorKind::Validation,
        "ledger.exchange_rate_missing",
        "There is no {base}/{quote} exchange rate for {date}. Post in the book currency, or add \
         the rate first.",
    )
    .with_param("base", base.code())
    .with_param("quote", quote.code())
    .with_param("date", on.format("%Y-%m-%d").to_string())
}

// ---- Kiểm đầu vào -------------------------------------------------------------------------------

/// Mục đích: chữ thường, số, `_`, các phần cách nhau bằng `.`; tối đa 64 ký tự.
pub(crate) fn valid_purpose(purpose: &str) -> bool {
    let words_ok = purpose.split('.').all(|word| {
        let mut chars = word.chars();
        chars.next().is_some_and(|first| first.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    });
    (1..=64).contains(&purpose.len()) && words_ok
}

/// Mã module / loại chứng từ: như mục đích (vd `orders`, `order`).
pub(crate) fn valid_name(name: &str) -> bool {
    valid_purpose(name)
}

/// ID đối tượng: 1–100 ký tự ASCII nhìn thấy được.
pub(crate) fn valid_id(id: &str) -> bool {
    (1..=100).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_graphic())
}

/// Kiểm khoá tài khoản, thêm lỗi vào `problems` với tiền tố `field`.
pub(crate) fn check_key(key: &AccountKey, field: &str, problems: &mut Vec<FieldError>) {
    if !valid_purpose(&key.purpose) {
        problems.push(FieldError::new(
            format!("{field}purpose"),
            "format",
            "must be lowercase words joined by `.`, e.g. `revenue.sales`",
        ));
    }
    if let Some(party) = &key.counterparty {
        if !valid_name(&party.kind) {
            problems.push(FieldError::new(
                format!("{field}counterparty_kind"),
                "format",
                "must be lowercase words joined by `.`, e.g. `org` or `customer`",
            ));
        }
        if !valid_id(&party.id) {
            problems.push(FieldError::new(
                format!("{field}counterparty_id"),
                "format",
                "must be 1 to 100 visible ASCII characters",
            ));
        }
    }
}

// ---- Mô tả ----------------------------------------------------------------------------------------

pub(crate) fn money(minor: i64, currency: &str) -> Result<Money, Error> {
    Money::from_columns(minor, currency)
}

fn describe_line(row: &line::Model) -> Result<EntryLine, Error> {
    Ok(EntryLine {
        line_no: row.line_no,
        account_id: row.account_id,
        side: Side::parse(&row.side)
            .ok_or_else(|| Error::internal("ledger line has a bad side"))?,
        amount: money(row.amount_minor, &row.amount_currency)?,
        base_amount: money(row.base_minor, &row.base_currency)?,
        exchange_rate: row.exchange_rate.clone(),
    })
}

/// Mô tả các bút toán `rows` kèm dòng và bút toán đảo chúng: hai câu cho cả trang.
pub(crate) async fn describe_entries<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    rows: Vec<entry::Model>,
) -> Result<Vec<Entry>, Error> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<uuid::Uuid> = rows.iter().map(|row| row.id).collect();
    let lines = line::Entity::find()
        .scoped(tenant)
        .filter(line::Column::EntryId.is_in(ids.clone()))
        .order_by_asc(line::Column::EntryId)
        .order_by_asc(line::Column::LineNo)
        .all(connection)
        .await?;
    let mut by_entry: HashMap<uuid::Uuid, Vec<EntryLine>> = HashMap::new();
    for row in &lines {
        by_entry.entry(row.entry_id).or_default().push(describe_line(row)?);
    }
    let reversals = entry::Entity::find()
        .scoped(tenant)
        .filter(entry::Column::ReversesEntryId.is_in(ids.clone()))
        .all(connection)
        .await?;
    let seals: HashMap<uuid::Uuid, (i64, DateTime<Utc>)> = crate::entity::seal::Entity::find()
        .scoped(tenant)
        .filter(crate::entity::seal::Column::EntryId.is_in(ids.clone()))
        .all(connection)
        .await?
        .into_iter()
        .map(|row| (row.entry_id, (row.sequence, row.sealed_at)))
        .collect();
    let reversed_by: HashMap<uuid::Uuid, uuid::Uuid> = reversals
        .iter()
        .filter_map(|row| row.reverses_entry_id.map(|original| (original, row.id)))
        .collect();
    Ok(rows
        .into_iter()
        .map(|row| Entry {
            lines: by_entry.remove(&row.id).unwrap_or_default(),
            sequence: seals.get(&row.id).map(|seal| seal.0),
            sealed_at: seals.get(&row.id).map(|seal| seal.1),
            reversed_by_entry_id: reversed_by.get(&row.id).copied(),
            id: row.id,
            book_id: row.book_id,
            org_id: row.org_id,
            period_id: row.period_id,
            effective_date: row.effective_date,
            posted_at: row.posted_at,
            description: row.description,
            source_module: row.source_module,
            source_kind: row.source_kind,
            source_id: row.source_id,
            reference: row.reference,
            reverses_entry_id: row.reverses_entry_id,
            actor_user_id: row.actor_user_id,
        })
        .collect())
}

pub(crate) async fn describe_entry<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    row: entry::Model,
) -> Result<Entry, Error> {
    describe_entries(connection, tenant, vec![row])
        .await?
        .pop()
        .ok_or_else(|| Error::internal("a described ledger entry vanished"))
}

/// Một tài khoản kèm số dư hiện tại.
#[derive(Debug, Clone)]
pub(crate) struct AccountView {
    pub account: account::Model,
    pub balance: Money,
}

/// Cách sửa một tài khoản (`PATCH`): trường `None` thì giữ nguyên; `Some(None)` thì bỏ giá trị.
#[derive(Debug, Clone, Default)]
pub(crate) struct AccountChange {
    pub name: Option<Option<String>>,
    pub floor: Option<Option<Money>>,
    pub external_code: Option<Option<String>>,
    pub status: Option<String>,
}

/// Bộ lọc danh sách tài khoản.
#[derive(Debug, Clone, Default)]
pub(crate) struct AccountFilter {
    pub purpose: Option<String>,
    pub counterparty_kind: Option<String>,
    pub counterparty_id: Option<String>,
}

/// Bộ lọc danh sách bút toán.
#[derive(Debug, Clone, Default)]
pub(crate) struct EntryFilter {
    pub book_org: Option<OrgId>,
    pub source_module: Option<String>,
    pub source_id: Option<String>,
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
}

impl LedgerService {
    // ---- Cài đặt của nút --------------------------------------------------------------------

    async fn setting(&self, tenant: &TenantContext, org: OrgId, key: &str) -> Option<String> {
        let directory = self.directory.as_ref()?;
        match directory.setting(tenant, org, key).await {
            Ok(found) => found?.value?.as_str().map(str::to_owned),
            Err(error) => {
                tracing::warn!(error = %error, key, "cannot read a unit setting; using the default");
                None
            }
        }
    }

    /// Hôm nay theo múi giờ của nút (`organizations.timezone`; mặc định UTC).
    pub(crate) async fn today(&self, tenant: &TenantContext, org: OrgId) -> NaiveDate {
        let zone = self
            .setting(tenant, org, "organizations.timezone")
            .await
            .and_then(|name| name.parse::<chrono_tz::Tz>().ok());
        match zone {
            Some(zone) => Utc::now().with_timezone(&zone).date_naive(),
            None => Utc::now().date_naive(),
        }
    }

    /// Tiền hạch toán mặc định của sổ mới: `organizations.currency` của nút, hay VND.
    async fn default_currency(&self, tenant: &TenantContext, org: OrgId) -> Currency {
        let code = self.setting(tenant, org, "organizations.currency").await;
        code.and_then(|code| Currency::parse(&code).ok()).unwrap_or(Currency::VND)
    }

    // ---- Sổ ------------------------------------------------------------------------------------

    pub(crate) async fn find_book<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        org: OrgId,
    ) -> Result<Option<book::Model>, Error> {
        Ok(book::Entity::find()
            .scoped(tenant)
            .filter(book::Column::OrgId.eq(org.as_uuid()))
            .one(connection)
            .await?)
    }

    /// Sổ của nút `org`, mở (tiền hạch toán mặc định) nếu chưa có. Cho module khác qua `dyn Ledger`.
    pub(crate) async fn ensure_book(
        &self,
        tenant: &TenantContext,
        org: OrgId,
    ) -> Result<book::Model, Error> {
        if let Some(found) = Self::find_book(tenant.database().read(), tenant, org).await? {
            return Ok(found);
        }
        let currency = self.default_currency(tenant, org).await;
        self.insert_book(tenant, org, currency, 1, None).await?;
        Self::find_book(tenant.database().write(), tenant, org)
            .await?
            .ok_or_else(|| Error::internal("a ledger book vanished after it was written"))
    }

    async fn insert_book(
        &self,
        tenant: &TenantContext,
        org: OrgId,
        currency: Currency,
        fiscal_year_start_month: i32,
        guard: Option<&Principal>,
    ) -> Result<u64, Error> {
        let row = book::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(org.as_uuid()),
            base_currency: Set(currency.code().to_owned()),
            fiscal_year_start_month: Set(fiscal_year_start_month),
            closed_through: Set(None),
            created_at: Set(now()),
        };
        let transaction = tenant.database().begin_write().await?;
        if let Some(principal) = guard {
            principal
                .require_write_at(&transaction, &ACCOUNTS_MANAGE, OrgTarget::new(org, None))
                .await?;
        }
        // `ON CONFLICT DO NOTHING`: hai bên cùng mở sổ cho một nút thì một bên thắng, không lỗi
        // giữa transaction (D-158).
        let inserted = book::Entity::insert(row)
            .on_conflict(
                OnConflict::columns([book::Column::TenantId, book::Column::OrgId])
                    .do_nothing()
                    .to_owned(),
            )
            .exec_without_returning(&transaction)
            .await?;
        transaction.commit().await?;
        Ok(inserted)
    }

    /// Mở sổ cho nút `org` (REST): `ledger.book_exists` khi đã có.
    pub(crate) async fn create_book(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: OrgId,
        base_currency: Option<String>,
        fiscal_year_start_month: Option<i32>,
    ) -> Result<book::Model, Error> {
        principal.require(&ACCOUNTS_MANAGE)?;
        let mut problems = Vec::new();
        let currency = match base_currency {
            Some(code) => match Currency::parse(&code) {
                Ok(currency) => Some(currency),
                Err(_) => {
                    problems.push(FieldError::new(
                        "base_currency",
                        "invalid",
                        "must be an ISO 4217 currency code, e.g. `VND`",
                    ));
                    None
                }
            },
            None => None,
        };
        let month = fiscal_year_start_month.unwrap_or(1);
        if !(1..=12).contains(&month) {
            problems.push(FieldError::new("fiscal_year_start_month", "range", "must be 1 to 12"));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        principal
            .require_at(tenant.database().read(), &ACCOUNTS_MANAGE, OrgTarget::new(org, None))
            .await?;
        self.require_unit(tenant, org).await?;
        let currency = match currency {
            Some(currency) => currency,
            None => self.default_currency(tenant, org).await,
        };
        if self.insert_book(tenant, org, currency, month, Some(principal)).await? == 0 {
            return Err(Error::conflict(
                "ledger.book_exists",
                "This unit already has a book. Each unit has at most one.",
            ));
        }
        Self::find_book(tenant.database().write(), tenant, org)
            .await?
            .ok_or_else(|| Error::internal("a ledger book vanished after it was written"))
    }

    pub(crate) async fn list_books(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<book::Model>, Error> {
        let condition = principal.scope_condition(&READ, book::Column::OrgId, None)?;
        let select = book::Entity::find().scoped(tenant).filter(condition);
        fetch_page(select, book::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Sổ `id` cho người có `permission` ở nút của sổ; ngoài phạm vi: như không có.
    pub(crate) async fn book_for(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        permission: &autoxide_kernel::access::Permission,
        id: uuid::Uuid,
    ) -> Result<book::Model, Error> {
        principal.require(permission)?;
        let read = tenant.database().read();
        let found = book::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(book_not_found)?;
        principal
            .require_at(read, permission, OrgTarget::new(OrgId::from_uuid(found.org_id), None))
            .await
            .map_err(
                |error| if error.code() == "org_out_of_scope" { book_not_found() } else { error },
            )?;
        Ok(found)
    }

    /// Sổ của nút `org` cho người có `permission` ở nút đó.
    pub(crate) async fn book_of(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        permission: &autoxide_kernel::access::Permission,
        org: OrgId,
    ) -> Result<book::Model, Error> {
        principal.require(permission)?;
        let read = tenant.database().read();
        principal.require_at(read, permission, OrgTarget::new(org, None)).await.map_err(
            |error| if error.code() == "org_out_of_scope" { book_not_found() } else { error },
        )?;
        Self::find_book(read, tenant, org).await?.ok_or_else(book_not_found)
    }

    // ---- Tài khoản -----------------------------------------------------------------------------

    /// Tìm hay mở tài khoản mang khoá `key` trong sổ `book`, trong `connection` (thường là
    /// transaction ghi của bút toán). Mở mới cần `normal_side` (mặc định theo mục đích cơ bản).
    pub(crate) async fn find_or_open<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        book: &book::Model,
        key: &AccountKey,
        normal_side: Option<Side>,
        floor_minor: Option<i64>,
        name: Option<String>,
    ) -> Result<(account::Model, bool), Error> {
        let (kind, id) = key
            .counterparty
            .as_ref()
            .map_or((String::new(), String::new()), |party| (party.kind.clone(), party.id.clone()));
        let find = || {
            account::Entity::find()
                .scoped(tenant)
                .filter(account::Column::BookId.eq(book.id))
                .filter(account::Column::Purpose.eq(key.purpose.as_str()))
                .filter(account::Column::CounterpartyKind.eq(kind.as_str()))
                .filter(account::Column::CounterpartyId.eq(id.as_str()))
                .filter(account::Column::Currency.eq(key.currency.code()))
        };
        if let Some(found) = find().one(connection).await? {
            return Ok((found, false));
        }
        let Some(side) = normal_side.or_else(|| normal_side_of(&key.purpose)) else {
            return Err(Error::validation(vec![FieldError::new(
                "normal_side",
                "required",
                "is required for a purpose outside the basic set; open the account first",
            )]));
        };
        let at = now();
        let account_id = uuid::Uuid::now_v7();
        let row = account::ActiveModel {
            id: Set(account_id),
            tenant_id: Set(tenant.id().as_uuid()),
            book_id: Set(book.id),
            org_id: Set(book.org_id),
            purpose: Set(key.purpose.clone()),
            counterparty_kind: Set(kind.clone()),
            counterparty_id: Set(id.clone()),
            currency: Set(key.currency.code().to_owned()),
            normal_side: Set(side.as_str().to_owned()),
            floor_minor: Set(floor_minor),
            external_code: Set(None),
            name: Set(name),
            status: Set("active".to_owned()),
            created_at: Set(at),
            updated_at: Set(at),
        };
        let inserted = account::Entity::insert(row)
            .on_conflict(
                OnConflict::columns([
                    account::Column::TenantId,
                    account::Column::BookId,
                    account::Column::Purpose,
                    account::Column::CounterpartyKind,
                    account::Column::CounterpartyId,
                    account::Column::Currency,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(connection)
            .await?;
        if inserted == 1 {
            balance::Entity::insert(balance::ActiveModel {
                account_id: Set(account_id),
                tenant_id: Set(tenant.id().as_uuid()),
                balance_minor: Set(0),
                currency: Set(key.currency.code().to_owned()),
            })
            .exec_without_returning(connection)
            .await?;
        }
        let found = find()
            .one(connection)
            .await?
            .ok_or_else(|| Error::internal("a ledger account vanished"))?;
        Ok((found, inserted == 1))
    }

    /// Mở tài khoản (REST, `dyn Ledger`): trả tài khoản đã có nếu cùng khoá (`false`: đã có).
    pub(crate) async fn open_account(
        &self,
        tenant: &TenantContext,
        key: &AccountKey,
        normal_side: Option<Side>,
        floor_minor: Option<i64>,
        name: Option<String>,
        guard: Option<&Principal>,
    ) -> Result<(account::Model, bool), Error> {
        let mut problems = Vec::new();
        check_key(key, "", &mut problems);
        check_name(name.as_deref(), &mut problems);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let book = self.ensure_book(tenant, key.book).await?;
        let (book, key) = (&book, key);
        tenant
            .database()
            .write_with_retry(move || {
                let name = name.clone();
                async move {
                    let transaction = tenant.database().begin_write().await?;
                    if let Some(principal) = guard {
                        let target = OrgTarget::new(key.book, None);
                        principal.require_write_at(&transaction, &ACCOUNTS_MANAGE, target).await?;
                    }
                    let found = Self::find_or_open(
                        &transaction,
                        tenant,
                        book,
                        key,
                        normal_side,
                        floor_minor,
                        name,
                    )
                    .await?;
                    transaction.commit().await?;
                    Ok(found)
                }
            })
            .await
    }

    /// Mở tài khoản qua REST / MCP: cần `ledger.accounts_manage` ở nút của sổ.
    pub(crate) async fn open_account_manual(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        key: &AccountKey,
        normal_side: Option<Side>,
        floor: Option<Money>,
        name: Option<String>,
    ) -> Result<(AccountView, bool), Error> {
        principal.require(&ACCOUNTS_MANAGE)?;
        if floor.is_some_and(|floor| floor.currency() != key.currency) {
            return Err(Error::validation(vec![FieldError::new(
                "floor",
                "currency",
                "must be in the currency of the account",
            )]));
        }
        let floor_minor = floor.map(Money::minor);
        principal
            .require_at(tenant.database().read(), &ACCOUNTS_MANAGE, OrgTarget::new(key.book, None))
            .await?;
        self.require_unit(tenant, key.book).await?;
        let (row, created) =
            self.open_account(tenant, key, normal_side, floor_minor, name, Some(principal)).await?;
        Ok((Self::account_view(tenant.database().write(), tenant, row).await?, created))
    }

    /// Nút `org` có trong cây tổ chức (khi có module `organizations`).
    async fn require_unit(&self, tenant: &TenantContext, org: OrgId) -> Result<(), Error> {
        if let Some(directory) = &self.directory
            && directory.unit(tenant, org).await?.is_none()
        {
            return Err(Error::validation(vec![FieldError::new(
                "org_id",
                "unknown",
                "there is no organization unit with this id",
            )]));
        }
        Ok(())
    }

    pub(crate) async fn account_view<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        row: account::Model,
    ) -> Result<AccountView, Error> {
        let found = balance::Entity::find_by_id(row.id).scoped(tenant).one(connection).await?;
        let balance = money(found.map_or(0, |found| found.balance_minor), &row.currency)?;
        Ok(AccountView { account: row, balance })
    }

    /// Tài khoản `id` cho người có `permission` ở nút của sổ; ngoài phạm vi: như không có.
    pub(crate) async fn account_for(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        permission: &autoxide_kernel::access::Permission,
        id: uuid::Uuid,
    ) -> Result<account::Model, Error> {
        principal.require(permission)?;
        let read = tenant.database().read();
        let found = account::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(account_not_found)?;
        principal
            .require_at(read, permission, OrgTarget::new(OrgId::from_uuid(found.org_id), None))
            .await
            .map_err(|error| {
                if error.code() == "org_out_of_scope" { account_not_found() } else { error }
            })?;
        Ok(found)
    }

    /// Tài khoản `id` kèm số dư hiện tại, cho người có `ledger.read` ở nút của sổ.
    pub(crate) async fn account_detail(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: uuid::Uuid,
    ) -> Result<AccountView, Error> {
        let found = self.account_for(tenant, principal, &READ, id).await?;
        Self::account_view(tenant.database().read(), tenant, found).await
    }

    pub(crate) async fn list_accounts(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        book_id: uuid::Uuid,
        filter: AccountFilter,
        page: &PageRequest,
    ) -> Result<Page<AccountView>, Error> {
        let book = self.book_for(tenant, principal, &READ, book_id).await?;
        let mut select =
            account::Entity::find().scoped(tenant).filter(account::Column::BookId.eq(book.id));
        if let Some(purpose) = filter.purpose {
            select = select.filter(account::Column::Purpose.eq(purpose));
        }
        if let Some(kind) = filter.counterparty_kind {
            select = select.filter(account::Column::CounterpartyKind.eq(kind));
        }
        if let Some(id) = filter.counterparty_id {
            select = select.filter(account::Column::CounterpartyId.eq(id));
        }
        let read = tenant.database().read();
        let page = fetch_page(select, account::Column::Id, |row| row.id, page, read).await?;
        let ids: Vec<uuid::Uuid> = page.data.iter().map(|row| row.id).collect();
        let balances: HashMap<uuid::Uuid, i64> = balance::Entity::find()
            .scoped(tenant)
            .filter(balance::Column::AccountId.is_in(ids))
            .all(read)
            .await?
            .into_iter()
            .map(|row| (row.account_id, row.balance_minor))
            .collect();
        let Page { data, pagination } = page;
        let mut views = Vec::with_capacity(data.len());
        for row in data {
            let balance = money(balances.get(&row.id).copied().unwrap_or(0), &row.currency)?;
            views.push(AccountView { account: row, balance });
        }
        Ok(Page { data: views, pagination })
    }

    /// Sửa tên, sàn, mã ngoài, trạng thái của tài khoản `id`.
    pub(crate) async fn update_account(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: uuid::Uuid,
        change: AccountChange,
    ) -> Result<AccountView, Error> {
        let found = self.account_for(tenant, principal, &ACCOUNTS_MANAGE, id).await?;
        let mut problems = Vec::new();
        if let Some(Some(name)) = &change.name {
            check_name(Some(name), &mut problems);
        }
        if let Some(Some(code)) = &change.external_code
            && !(1..=32).contains(&code.len())
        {
            problems.push(FieldError::new("external_code", "length", "must be 1 to 32 characters"));
        }
        if let Some(Some(floor)) = change.floor
            && floor.currency().code() != found.currency
        {
            problems.push(FieldError::new(
                "floor",
                "currency",
                "must be in the currency of the account",
            ));
        }
        if let Some(status) = &change.status
            && !["active", "closed"].contains(&status.as_str())
        {
            problems.push(FieldError::new("status", "invalid", "must be `active` or `closed`"));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let transaction = tenant.database().begin_write().await?;
        principal
            .require_write_at(
                &transaction,
                &ACCOUNTS_MANAGE,
                OrgTarget::new(OrgId::from_uuid(found.org_id), None),
            )
            .await?;
        // Sàn mới phải đúng với số dư hiện tại: khoá dòng số dư rồi so.
        if let Some(Some(floor)) = change.floor.map(|floor| floor.map(Money::minor)) {
            let current = autoxide_kernel::rows::lock_one(
                &transaction,
                balance::Entity::find_by_id(found.id).scoped(tenant),
            )
            .await?
            .map_or(0, |row| row.balance_minor);
            if current < floor {
                return Err(Error::validation(vec![
                    FieldError::new(
                        "floor",
                        "above_balance",
                        "must not be above the current balance ({balance})",
                    )
                    .with_param("balance", money(current, &found.currency)?),
                ]));
            }
        }
        let mut row: account::ActiveModel = found.into();
        if let Some(name) = change.name {
            row.name = Set(name);
        }
        if let Some(floor) = change.floor {
            row.floor_minor = Set(floor.map(Money::minor));
        }
        if let Some(code) = change.external_code {
            row.external_code = Set(code);
        }
        if let Some(status) = change.status {
            row.status = Set(status);
        }
        row.updated_at = Set(now());
        let saved = account::Entity::update(row).exec(&transaction).await?;
        let view = Self::account_view(&transaction, tenant, saved).await?;
        transaction.commit().await?;
        Ok(view)
    }

    // ---- Bút toán (đọc) ------------------------------------------------------------------------

    pub(crate) async fn entry_for(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: uuid::Uuid,
    ) -> Result<Entry, Error> {
        principal.require(&READ)?;
        let read = tenant.database().read();
        let found = entry::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(entry_not_found)?;
        principal
            .require_at(read, &READ, OrgTarget::new(OrgId::from_uuid(found.org_id), None))
            .await
            .map_err(
                |error| if error.code() == "org_out_of_scope" { entry_not_found() } else { error },
            )?;
        describe_entry(read, tenant, found).await
    }

    pub(crate) async fn list_entries(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: EntryFilter,
        page: &PageRequest,
    ) -> Result<Page<Entry>, Error> {
        let condition = principal.scope_condition(&READ, entry::Column::OrgId, None)?;
        let mut select = entry::Entity::find().scoped(tenant).filter(condition);
        if let Some(org) = filter.book_org {
            select = select.filter(entry::Column::OrgId.eq(org.as_uuid()));
        }
        if let Some(module) = filter.source_module {
            select = select.filter(entry::Column::SourceModule.eq(module));
        }
        if let Some(id) = filter.source_id {
            select = select.filter(entry::Column::SourceId.eq(id));
        }
        if let Some(from) = filter.from {
            select = select.filter(entry::Column::EffectiveDate.gte(from));
        }
        if let Some(to) = filter.to {
            select = select.filter(entry::Column::EffectiveDate.lte(to));
        }
        let read = tenant.database().read();
        let page = fetch_page(select, entry::Column::Id, |row| row.id, page, read).await?;
        let Page { data, pagination } = page;
        Ok(Page { data: describe_entries(read, tenant, data).await?, pagination })
    }

    /// Các dòng của tài khoản `id`, mới nhất trước theo mặc định.
    pub(crate) async fn account_lines(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: uuid::Uuid,
        page: &PageRequest,
    ) -> Result<Page<line::Model>, Error> {
        let found = self.account_for(tenant, principal, &READ, id).await?;
        let select =
            line::Entity::find().scoped(tenant).filter(line::Column::AccountId.eq(found.id));
        fetch_page(select, line::Column::Id, |row| row.id, page, tenant.database().read()).await
    }
}

fn check_name(name: Option<&str>, problems: &mut Vec<FieldError>) {
    if name.is_some_and(|name| !(1..=200).contains(&name.chars().count())) {
        problems.push(FieldError::new("name", "length", "must be 1 to 200 characters long"));
    }
}

/// Số đếm niêm phong của một sổ (hay một kỳ của sổ).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SealStatus {
    pub total: u64,
    pub sealed: u64,
    pub oldest_unsealed_at: Option<DateTime<Utc>>,
}

/// Khoá tối đa của một lần [`LedgerService::balances_of_keys`].
pub(crate) const MAX_BALANCE_KEYS: usize = 500;

impl LedgerService {
    /// Số dư của nhiều khoá tài khoản, cùng thứ tự (`dyn Ledger::balances`). Hiện tại: hai câu cho
    /// mọi khoá (tài khoản, số dư cộng dồn); tại ngày `as_of`: mỗi tài khoản tìm thấy một lần tính.
    pub(crate) async fn balances_of_keys(
        &self,
        tenant: &TenantContext,
        keys: &[AccountKey],
        as_of: Option<NaiveDate>,
    ) -> Result<Vec<Money>, Error> {
        use sea_orm::Condition;
        if keys.len() > MAX_BALANCE_KEYS {
            return Err(Error::validation(vec![
                FieldError::new("keys", "count", "must be at most {max} keys")
                    .with_param("max", MAX_BALANCE_KEYS),
            ]));
        }
        let read = tenant.database().read();
        let orgs: Vec<uuid::Uuid> = keys.iter().map(|key| key.book.as_uuid()).collect();
        let books = book::Entity::find()
            .scoped(tenant)
            .filter(book::Column::OrgId.is_in(orgs))
            .all(read)
            .await?;
        let book_of =
            |org: OrgId| books.iter().find(|row| row.org_id == org.as_uuid()).map(|row| row.id);
        let party = |key: &AccountKey| {
            key.counterparty.as_ref().map_or((String::new(), String::new()), |found| {
                (found.kind.clone(), found.id.clone())
            })
        };
        let mut any = Condition::any();
        let mut wanted = false;
        for key in keys {
            let Some(book_id) = book_of(key.book) else { continue };
            let (kind, id) = party(key);
            wanted = true;
            any = any.add(
                Condition::all()
                    .add(account::Column::BookId.eq(book_id))
                    .add(account::Column::Purpose.eq(key.purpose.as_str()))
                    .add(account::Column::CounterpartyKind.eq(kind))
                    .add(account::Column::CounterpartyId.eq(id))
                    .add(account::Column::Currency.eq(key.currency.code())),
            );
        }
        let accounts = if wanted {
            account::Entity::find().scoped(tenant).filter(any).all(read).await?
        } else {
            Vec::new()
        };
        let current: HashMap<uuid::Uuid, i64> = if as_of.is_none() && !accounts.is_empty() {
            balance::Entity::find()
                .scoped(tenant)
                .filter(balance::Column::AccountId.is_in(accounts.iter().map(|row| row.id)))
                .all(read)
                .await?
                .into_iter()
                .map(|row| (row.account_id, row.balance_minor))
                .collect()
        } else {
            HashMap::new()
        };
        let mut found = Vec::with_capacity(keys.len());
        for key in keys {
            let (kind, id) = party(key);
            let matched = book_of(key.book).and_then(|book_id| {
                accounts.iter().find(|row| {
                    row.book_id == book_id
                        && row.purpose == key.purpose
                        && row.counterparty_kind == kind
                        && row.counterparty_id == id
                        && row.currency == key.currency.code()
                })
            });
            let amount = match (matched, as_of) {
                (None, _) => Money::zero(key.currency),
                (Some(row), None) => {
                    money(current.get(&row.id).copied().unwrap_or(0), &row.currency)?
                }
                (Some(row), Some(_)) => Self::account_balance(read, tenant, row, as_of).await?,
            };
            found.push(amount);
        }
        Ok(found)
    }

    /// Bút toán của sổ `book_id` (hay kỳ `period_id`) đã vào chuỗi băm chưa: tổng, đã niêm phong,
    /// bút toán chưa niêm phong lâu nhất. Đếm theo index (sổ, ngày) và (bút toán) — không quét bảng.
    pub(crate) async fn seal_status(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        book_id: uuid::Uuid,
        period_id: Option<uuid::Uuid>,
    ) -> Result<SealStatus, Error> {
        use crate::entity::{period, seal};
        use sea_orm::{PaginatorTrait, QuerySelect, sea_query::Query};
        let book = self.book_for(tenant, principal, &READ, book_id).await?;
        let read = tenant.database().read();
        let mut entries =
            entry::Entity::find().scoped(tenant).filter(entry::Column::BookId.eq(book.id));
        if let Some(period_id) = period_id {
            let found = period::Entity::find_by_id(period_id)
                .scoped(tenant)
                .filter(period::Column::BookId.eq(book.id))
                .one(read)
                .await?
                .ok_or_else(|| {
                    Error::not_found(
                        "ledger.period_not_found",
                        "There is no such accounting period.",
                    )
                })?;
            entries = entries
                .filter(entry::Column::EffectiveDate.gte(found.starts_on))
                .filter(entry::Column::EffectiveDate.lte(found.ends_on));
        }
        let total = entries.clone().count(read).await?;
        let sealed_ids = Query::select()
            .column(seal::Column::EntryId)
            .from(seal::Entity)
            .and_where(seal::Column::TenantId.eq(tenant.id().as_uuid()))
            .and_where(seal::Column::BookId.eq(book.id))
            .to_owned();
        let unsealed = entries.filter(entry::Column::Id.not_in_subquery(sealed_ids));
        let pending = unsealed.clone().count(read).await?;
        let oldest = unsealed
            .order_by_asc(entry::Column::Id)
            .select_only()
            .column(entry::Column::PostedAt)
            .into_tuple::<DateTime<Utc>>()
            .one(read)
            .await?;
        Ok(SealStatus { total, sealed: total.saturating_sub(pending), oldest_unsealed_at: oldest })
    }
}
