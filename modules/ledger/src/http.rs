//! Route REST của module `ledger`, dưới `/api/v1/ledger`.

use autoxide_kernel::time::{Date, Timestamp};
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::{OrgId, TenantContext, root_of};
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::api::{
    AccountKey, Counterparty, Entry, LineAccount, NewEntry, NewLine, NewReversal, Side,
};
use crate::entity::{book, line};
use crate::periods::{PeriodView, TrialRow};
use crate::permissions::READ;
use crate::service::{
    AccountChange, AccountFilter, AccountView, EntryFilter, LedgerService, money,
};

pub(crate) fn routes(service: Arc<LedgerService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_books, create_book))
        .routes(routes!(get_book))
        .routes(routes!(list_accounts, open_account))
        .routes(routes!(get_account, update_account))
        .routes(routes!(account_lines))
        .routes(routes!(account_balance))
        .routes(routes!(list_entries, post_entry))
        .routes(routes!(get_entry))
        .routes(routes!(reverse_entry))
        .routes(routes!(list_periods))
        .routes(routes!(close_period))
        .routes(routes!(trial_balance))
        .with_state(service)
}

// Trường JSON có ba trạng thái: không gửi (`None`), `null` (`Some(None)`), có giá trị.
pub(crate) fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// 201 + `Location` lần đầu; 200 khi gửi lại cùng `reference` và cùng nội dung.
fn submitted(outcome: Submitted<Entry>) -> Response {
    match outcome {
        Submitted::Created(entry) => {
            Created::new(format!("/ledger/entries/{}", entry.id), entry).into_response()
        }
        Submitted::Replayed(entry) => Data { data: entry }.into_response(),
    }
}

// ---- Sổ -----------------------------------------------------------------------------------------

/// A book: the ledger of one organization unit (an accounting entity).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerBookResponse)]
pub(crate) struct BookResponse {
    id: Uuid,
    /// The unit that keeps the book.
    org_id: Uuid,
    /// Accounting currency of the book.
    #[schema(example = "VND")]
    base_currency: String,
    /// First month of the fiscal year, 1 to 12.
    fiscal_year_start_month: i32,
    /// Last day of the latest closed period: no entry may be dated on or before it.
    closed_through: Option<Date>,
    created_at: Timestamp,
}

impl From<book::Model> for BookResponse {
    fn from(row: book::Model) -> Self {
        BookResponse {
            id: row.id,
            org_id: row.org_id,
            base_currency: row.base_currency,
            fiscal_year_start_month: row.fiscal_year_start_month,
            closed_through: row.closed_through,
            created_at: row.created_at,
        }
    }
}

/// A book to open.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = LedgerBookRequest)]
pub(crate) struct BookRequest {
    /// The unit; default the tenant's root unit.
    org_id: Option<Uuid>,
    /// Accounting currency; default the unit's setting `organizations.currency`, else `VND`.
    #[schema(example = "VND")]
    base_currency: Option<String>,
    /// First month of the fiscal year, 1 to 12; default 1.
    fiscal_year_start_month: Option<i32>,
}

/// List the books in your scope.
///
/// Requires `ledger.read`.
#[utoipa::path(
    get,
    path = "/books",
    extensions(("x-permissions" = json!(["ledger.read"]))),
    operation_id = "list_ledger_books",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of books.", body = Page<BookResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_books(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<BookResponse>, Error> {
    let page = service.list_books(&tenant, &principal, &PageRequest::try_from(page)?).await?;
    Ok(page.map(BookResponse::from))
}

/// Open the book of a unit.
///
/// Requires `ledger.accounts_manage` on the unit. A unit has at most one book; modules that post
/// entries open it on first use.
#[utoipa::path(
    post,
    path = "/books",
    extensions(("x-permissions" = json!(["ledger.accounts_manage"]))),
    operation_id = "create_ledger_book",
    security(("bearer" = [])),
    request_body = BookRequest,
    responses(
        (status = 201, description = "The book.", body = Data<BookResponse>, headers(("Location" = String, description = "Path of the new book."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.accounts_manage` (`permission_denied`), or the unit is outside your scope (`org_out_of_scope`) or frozen (`org_frozen`).", body = Problem),
        (status = 409, description = "The unit already has a book; code `ledger.book_exists`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn create_book(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<BookRequest>,
) -> Result<Created<BookResponse>, Error> {
    let org = input.org_id.map_or_else(|| root_of(tenant.id()), OrgId::from_uuid);
    let row = service
        .create_book(&tenant, &principal, org, input.base_currency, input.fiscal_year_start_month)
        .await?;
    Ok(Created::new(format!("/ledger/books/{}", row.id), row.into()))
}

/// Get a book.
///
/// Requires `ledger.read` on the book's unit.
#[utoipa::path(
    get,
    path = "/books/{id}",
    extensions(("x-permissions" = json!(["ledger.read"]))),
    operation_id = "get_ledger_book",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the book.")),
    responses(
        (status = 200, description = "The book.", body = Data<BookResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such book in your scope; code `ledger.book_not_found`.", body = Problem),
    )
)]
async fn get_book(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<BookResponse>, Error> {
    Ok(Data { data: service.book_for(&tenant, &principal, &READ, id).await?.into() })
}

// ---- Tài khoản ------------------------------------------------------------------------------------

/// A ledger account: one purpose, for one counterparty, in one currency, in one book.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerAccountResponse)]
pub(crate) struct AccountResponse {
    id: Uuid,
    book_id: Uuid,
    org_id: Uuid,
    /// E.g. `revenue.sales`, `payable.seller_held`, or `<module>.<name>`.
    #[schema(example = "revenue.sales")]
    purpose: String,
    counterparty: Option<Counterparty>,
    #[schema(example = "VND")]
    currency: String,
    /// The side that increases the balance.
    normal_side: Side,
    /// The balance may not go below this; `null`: no limit.
    floor: Option<Money>,
    /// Account number in the accounting software; `null`: from the mapping of the book.
    external_code: Option<String>,
    name: Option<String>,
    /// `active` or `closed` (takes no new entries).
    status: String,
    /// Current balance on the normal side (entries dated in the future included).
    balance: Money,
    created_at: Timestamp,
    updated_at: Timestamp,
}

fn describe_account(view: AccountView) -> Result<AccountResponse, Error> {
    let row = view.account;
    let counterparty = (!row.counterparty_kind.is_empty())
        .then(|| Counterparty::new(row.counterparty_kind.clone(), row.counterparty_id.clone()));
    let floor = match row.floor_minor {
        Some(minor) => Some(money(minor, &row.currency)?),
        None => None,
    };
    Ok(AccountResponse {
        id: row.id,
        book_id: row.book_id,
        org_id: row.org_id,
        purpose: row.purpose,
        counterparty,
        currency: row.currency,
        normal_side: Side::parse(&row.normal_side)
            .ok_or_else(|| Error::internal("ledger account has a bad side"))?,
        floor,
        external_code: row.external_code,
        name: row.name,
        status: row.status,
        balance: view.balance,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// Which accounts.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct AccountsQuery {
    /// The book (required).
    book_id: Uuid,
    /// Only this purpose.
    purpose: Option<String>,
    /// Only this kind of counterparty (`org`, `customer`…).
    counterparty_kind: Option<String>,
    /// Only this counterparty.
    counterparty_id: Option<String>,
}

/// An account to open.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = LedgerAccountRequest)]
pub(crate) struct AccountRequest {
    /// The unit of the book; default the tenant's root. The book is opened if missing.
    org_id: Option<Uuid>,
    /// A basic purpose (`cash.bank`, `revenue.sales`…) or `<module>.<name>`.
    #[schema(example = "cash.bank")]
    purpose: String,
    /// Kind of counterparty, with `counterparty_id`.
    counterparty_kind: Option<String>,
    counterparty_id: Option<String>,
    #[schema(example = "VND")]
    currency: String,
    /// Required for a purpose outside the basic set.
    normal_side: Option<Side>,
    /// Lowest balance allowed.
    floor: Option<Money>,
    /// Up to 200 characters.
    name: Option<String>,
}

/// Changes to an account. Omitted fields stay; `null` clears.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = LedgerAccountChange)]
pub(crate) struct AccountChangeRequest {
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    name: Option<Option<String>>,
    /// Must not be above the current balance; `null` removes the limit.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Money>, nullable)]
    floor: Option<Option<Money>>,
    /// Account number in the accounting software, up to 32 characters.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    external_code: Option<Option<String>>,
    /// `active` or `closed`.
    status: Option<String>,
}

/// List the accounts of a book.
///
/// Requires `ledger.read` on the book's unit.
#[utoipa::path(
    get,
    path = "/accounts",
    extensions(("x-permissions" = json!(["ledger.read"]))),
    operation_id = "list_ledger_accounts",
    security(("bearer" = [])),
    params(AccountsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of accounts with their balances.", body = Page<AccountResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such book in your scope; code `ledger.book_not_found`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_accounts(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(filter): Query<AccountsQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<AccountResponse>, Error> {
    let found = AccountFilter {
        purpose: filter.purpose,
        counterparty_kind: filter.counterparty_kind,
        counterparty_id: filter.counterparty_id,
    };
    let page = service
        .list_accounts(&tenant, &principal, filter.book_id, found, &PageRequest::try_from(page)?)
        .await?;
    let Page { data, pagination } = page;
    let data = data.into_iter().map(describe_account).collect::<Result<Vec<_>, _>>()?;
    Ok(Page { data, pagination })
}

/// Open an account.
///
/// Requires `ledger.accounts_manage` on the unit. The same purpose, counterparty and currency
/// again gives back the existing account (`200`).
#[utoipa::path(
    post,
    path = "/accounts",
    extensions(("x-permissions" = json!(["ledger.accounts_manage"]))),
    operation_id = "open_ledger_account",
    security(("bearer" = [])),
    request_body = AccountRequest,
    responses(
        (status = 200, description = "The account existed already.", body = Data<AccountResponse>),
        (status = 201, description = "The new account.", body = Data<AccountResponse>, headers(("Location" = String, description = "Path of the new account."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.accounts_manage` (`permission_denied`), or the unit is outside your scope (`org_out_of_scope`) or frozen (`org_frozen`).", body = Problem),
        (status = 422, description = "A value is invalid (e.g. `normal_side` missing for a module purpose); every problem in `errors`.", body = Problem),
    )
)]
async fn open_account(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<AccountRequest>,
) -> Result<Response, Error> {
    let org = input.org_id.map_or_else(|| root_of(tenant.id()), OrgId::from_uuid);
    let currency = Currency::parse(&input.currency).map_err(|_| {
        Error::validation(vec![autoxide_kernel::error::FieldError::new(
            "currency",
            "invalid",
            "must be an ISO 4217 currency code, e.g. `VND`",
        )])
    })?;
    let mut key = AccountKey::new(org, input.purpose, currency);
    match (input.counterparty_kind, input.counterparty_id) {
        (Some(kind), Some(id)) => key = key.with_counterparty(Counterparty::new(kind, id)),
        (None, None) => {}
        _ => {
            return Err(Error::validation(vec![autoxide_kernel::error::FieldError::new(
                "counterparty_id",
                "pair",
                "send both `counterparty_kind` and `counterparty_id`, or neither",
            )]));
        }
    }
    let (view, created) = service
        .open_account_manual(&tenant, &principal, &key, input.normal_side, input.floor, input.name)
        .await?;
    let id = view.account.id;
    let data = describe_account(view)?;
    Ok(if created {
        Created::new(format!("/ledger/accounts/{id}"), data).into_response()
    } else {
        Data { data }.into_response()
    })
}

/// Get an account with its balance.
///
/// Requires `ledger.read` on the book's unit.
#[utoipa::path(
    get,
    path = "/accounts/{id}",
    extensions(("x-permissions" = json!(["ledger.read"]))),
    operation_id = "get_ledger_account",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the account.")),
    responses(
        (status = 200, description = "The account.", body = Data<AccountResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such account in your scope; code `ledger.account_not_found`.", body = Problem),
    )
)]
async fn get_account(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<AccountResponse>, Error> {
    let view = service.account_detail(&tenant, &principal, id).await?;
    Ok(Data { data: describe_account(view)? })
}

/// Change an account: name, floor, external code, status.
///
/// Requires `ledger.accounts_manage` on the book's unit. Purpose, counterparty and currency never
/// change.
#[utoipa::path(
    patch,
    path = "/accounts/{id}",
    extensions(("x-permissions" = json!(["ledger.accounts_manage"]))),
    operation_id = "update_ledger_account",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the account.")),
    request_body = AccountChangeRequest,
    responses(
        (status = 200, description = "The account after the change.", body = Data<AccountResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.accounts_manage` (`permission_denied`) or the unit is frozen (`org_frozen`).", body = Problem),
        (status = 404, description = "No such account in your scope; code `ledger.account_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid (e.g. the floor is above the balance); every problem in `errors`.", body = Problem),
    )
)]
async fn update_account(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<AccountChangeRequest>,
) -> Result<Data<AccountResponse>, Error> {
    let change = AccountChange {
        name: input.name,
        floor: input.floor,
        external_code: input.external_code,
        status: input.status,
    };
    let view = service.update_account(&tenant, &principal, id, change).await?;
    Ok(Data { data: describe_account(view)? })
}

/// One line on an account.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerAccountLine)]
pub(crate) struct LineResponse {
    id: Uuid,
    entry_id: Uuid,
    line_no: i32,
    side: Side,
    amount: Money,
    base_amount: Money,
    exchange_rate: Option<String>,
    effective_date: Date,
}

fn describe_line(row: line::Model) -> Result<LineResponse, Error> {
    Ok(LineResponse {
        id: row.id,
        entry_id: row.entry_id,
        line_no: row.line_no,
        side: Side::parse(&row.side)
            .ok_or_else(|| Error::internal("ledger line has a bad side"))?,
        amount: money(row.amount_minor, &row.amount_currency)?,
        base_amount: money(row.base_minor, &row.base_currency)?,
        exchange_rate: row.exchange_rate,
        effective_date: row.effective_date,
    })
}

/// List the entry lines on an account.
///
/// Requires `ledger.read` on the book's unit.
#[utoipa::path(
    get,
    path = "/accounts/{id}/lines",
    extensions(("x-permissions" = json!(["ledger.read"]))),
    operation_id = "list_ledger_account_lines",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the account."), PageQuery),
    responses(
        (status = 200, description = "One page of lines, newest first by default.", body = Page<LineResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such account in your scope; code `ledger.account_not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn account_lines(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> Result<Page<LineResponse>, Error> {
    let page =
        service.account_lines(&tenant, &principal, id, &PageRequest::try_from(page)?).await?;
    let Page { data, pagination } = page;
    let data = data.into_iter().map(describe_line).collect::<Result<Vec<_>, _>>()?;
    Ok(Page { data, pagination })
}

/// The balance of an account.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerBalanceResponse)]
pub(crate) struct BalanceResponse {
    account_id: Uuid,
    /// The day asked for; `null`: now.
    as_of: Option<Date>,
    /// Balance on the normal side of the account.
    balance: Money,
}

/// Which day.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct BalanceQuery {
    /// End of this day, `YYYY-MM-DD`; omit for the current balance.
    as_of: Option<Date>,
}

/// Get the balance of an account, now or at the end of a day.
///
/// Requires `ledger.read` on the book's unit.
#[utoipa::path(
    get,
    path = "/accounts/{id}/balance",
    extensions(("x-permissions" = json!(["ledger.read"]))),
    operation_id = "get_ledger_account_balance",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the account."), BalanceQuery),
    responses(
        (status = 200, description = "The balance.", body = Data<BalanceResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such account in your scope; code `ledger.account_not_found`.", body = Problem),
        (status = 422, description = "`as_of` is not a date; every problem in `errors`.", body = Problem),
    )
)]
async fn account_balance(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(query): Query<BalanceQuery>,
) -> Result<Data<BalanceResponse>, Error> {
    let (found, balance) = service.balance_for(&tenant, &principal, id, query.as_of).await?;
    Ok(Data { data: BalanceResponse { account_id: found.id, as_of: query.as_of, balance } })
}

// ---- Bút toán ------------------------------------------------------------------------------------

/// One line of a manual entry.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = LedgerLineRequest)]
pub(crate) struct LineRequest {
    account_id: Uuid,
    side: Side,
    /// Greater than zero; every line in the same currency as its account.
    amount: Money,
}

/// A manual entry to post.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = LedgerEntryRequest)]
pub(crate) struct EntryRequest {
    /// Unit of the book; default the tenant's root.
    org_id: Option<Uuid>,
    /// Accounting day; default today in the unit's time zone. Must be in an open period.
    effective_date: Option<Date>,
    /// Fixed text, 1 to 200 characters. It is kept for good in the hash chain: no names, emails or
    /// phone numbers.
    #[schema(example = "Opening balance")]
    description: String,
    /// Your id of this entry (1 to 100 visible ASCII characters): sending it again returns the
    /// entry already posted.
    #[schema(example = "JE-2026-0001")]
    reference: String,
    /// Kind of source document, e.g. `manual`, `opening`; default `manual`.
    source_kind: Option<String>,
    /// Id of the source document.
    source_id: Option<String>,
    /// Two or more lines; debits must equal credits.
    lines: Vec<LineRequest>,
}

/// Which entries.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct EntriesQuery {
    /// Only the book of this unit.
    org_id: Option<Uuid>,
    /// Only entries written by this module (`ledger` for manual entries, `orders`…).
    source_module: Option<String>,
    /// Only entries for this source document.
    source_id: Option<String>,
    /// Only entries dated on or after this day.
    from: Option<Date>,
    /// Only entries dated on or before this day.
    to: Option<Date>,
}

/// A reversal.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = LedgerReversalRequest)]
pub(crate) struct ReversalRequest {
    /// Your id of the reversal: sending it again returns the reversal already posted.
    reference: String,
    /// Accounting day of the reversal; default today in the unit's time zone (an open period).
    effective_date: Option<Date>,
}

/// List entries in your scope.
///
/// Requires `ledger.read`. Each entry comes with its lines.
#[utoipa::path(
    get,
    path = "/entries",
    extensions(("x-permissions" = json!(["ledger.read"]))),
    operation_id = "list_ledger_entries",
    security(("bearer" = [])),
    params(EntriesQuery, PageQuery),
    responses(
        (status = 200, description = "One page of entries.", body = Page<Entry>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_entries(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<EntriesQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<Entry>, Error> {
    let filter = EntryFilter {
        book_org: query.org_id.map(OrgId::from_uuid),
        source_module: query.source_module,
        source_id: query.source_id,
        from: query.from,
        to: query.to,
    };
    service.list_entries(&tenant, &principal, filter, &PageRequest::try_from(page)?).await
}

/// Post a manual entry.
///
/// Requires `ledger.post` on the book's unit. Entries are never changed or deleted: correct one
/// with `POST /entries/{id}/reverse`. Send the same `reference` and content again to get the entry
/// already posted (`200`).
#[utoipa::path(
    post,
    path = "/entries",
    extensions(("x-permissions" = json!(["ledger.post"]))),
    operation_id = "post_ledger_entry",
    security(("bearer" = [])),
    request_body = EntryRequest,
    responses(
        (status = 200, description = "The entry was posted before with this `reference` and content.", body = Data<Entry>),
        (status = 201, description = "The entry.", body = Data<Entry>, headers(("Location" = String, description = "Path of the new entry."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.post` (`permission_denied`), or the unit is outside your scope (`org_out_of_scope`) or frozen (`org_frozen`).", body = Problem),
        (status = 404, description = "An account is not in this book; code `ledger.account_not_found`.", body = Problem),
        (status = 409, description = "The day is in a closed period (`ledger.period_closed`), an account is closed (`ledger.account_closed`) or would go below its floor (`ledger.insufficient_balance`).", body = Problem),
        (status = 422, description = "Debits differ from credits (`ledger.unbalanced`), currencies differ (`ledger.currency_mixed`), not the book currency (`ledger.exchange_rate_missing`), the reference was used for other content (`reference_reused`), or a value is invalid.", body = Problem),
    )
)]
async fn post_entry(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<EntryRequest>,
) -> Result<Response, Error> {
    let org = input.org_id.map_or_else(|| root_of(tenant.id()), OrgId::from_uuid);
    let lines = input
        .lines
        .into_iter()
        .map(|item| NewLine {
            account: LineAccount::Id(item.account_id),
            side: item.side,
            amount: item.amount,
        })
        .collect();
    let mut entry = NewEntry::new(
        org,
        "ledger",
        input.source_kind.unwrap_or_else(|| "manual".to_owned()),
        input.reference,
        input.description,
        lines,
    );
    entry.effective_date = input.effective_date;
    entry.source_id = input.source_id;
    Ok(submitted(service.post_manual(&tenant, &principal, entry).await?))
}

/// Get an entry with its lines.
///
/// Requires `ledger.read` on the book's unit.
#[utoipa::path(
    get,
    path = "/entries/{id}",
    extensions(("x-permissions" = json!(["ledger.read"]))),
    operation_id = "get_ledger_entry",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry.")),
    responses(
        (status = 200, description = "The entry.", body = Data<Entry>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such entry in your scope; code `ledger.entry_not_found`.", body = Problem),
    )
)]
async fn get_entry(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<Entry>, Error> {
    Ok(Data { data: service.entry_for(&tenant, &principal, id).await? })
}

/// Reverse an entry.
///
/// Requires `ledger.reverse` on the book's unit. Posts a new entry with debits and credits swapped,
/// dated in an open period. An entry is reversed at most once (a reversal may itself be reversed).
#[utoipa::path(
    post,
    path = "/entries/{id}/reverse",
    extensions(("x-permissions" = json!(["ledger.reverse"]))),
    operation_id = "reverse_ledger_entry",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry to reverse.")),
    request_body = ReversalRequest,
    responses(
        (status = 200, description = "The reversal was posted before with this `reference`.", body = Data<Entry>),
        (status = 201, description = "The reversal.", body = Data<Entry>, headers(("Location" = String, description = "Path of the reversal."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.reverse` (`permission_denied`) or the unit is frozen (`org_frozen`).", body = Problem),
        (status = 404, description = "No such entry in your scope; code `ledger.entry_not_found`.", body = Problem),
        (status = 409, description = "Already reversed (`ledger.already_reversed`), the day is in a closed period (`ledger.period_closed`), an account is closed or would go below its floor.", body = Problem),
        (status = 422, description = "A value is invalid, or the reference was used for other content (`reference_reused`).", body = Problem),
    )
)]
async fn reverse_entry(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ReversalRequest>,
) -> Result<Response, Error> {
    let mut reversal = NewReversal::new("ledger", input.reference);
    reversal.effective_date = input.effective_date;
    Ok(submitted(service.reverse_manual(&tenant, &principal, id, reversal).await?))
}

// ---- Kỳ, bảng cân đối thử -------------------------------------------------------------------------

/// An accounting period (a calendar month).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerPeriodResponse)]
pub(crate) struct PeriodResponse {
    id: Uuid,
    book_id: Uuid,
    starts_on: Date,
    ends_on: Date,
    /// `open` or `closed`.
    status: String,
    closed_at: Option<Timestamp>,
    closed_by: Option<Uuid>,
    /// Number of entries dated in the period.
    #[schema(example = 120)]
    entries: i64,
    /// How many of them are already sealed into the book's hash chain (sealing runs in the
    /// background shortly after posting).
    #[schema(example = 118)]
    sealed_entries: i64,
}

impl From<PeriodView> for PeriodResponse {
    fn from(view: PeriodView) -> Self {
        let row = view.period;
        PeriodResponse {
            id: row.id,
            book_id: row.book_id,
            starts_on: row.starts_on,
            ends_on: row.ends_on,
            status: row.status,
            closed_at: row.closed_at,
            closed_by: row.closed_by,
            entries: view.entries,
            sealed_entries: view.sealed,
        }
    }
}

/// Which book.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct BookQuery {
    /// The book (required).
    book_id: Uuid,
}

/// List the periods of a book.
///
/// Requires `ledger.read` on the book's unit. A period exists once an entry is dated in it.
#[utoipa::path(
    get,
    path = "/periods",
    extensions(("x-permissions" = json!(["ledger.read"]))),
    operation_id = "list_ledger_periods",
    security(("bearer" = [])),
    params(BookQuery, PageQuery),
    responses(
        (status = 200, description = "One page of periods.", body = Page<PeriodResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such book in your scope; code `ledger.book_not_found`.", body = Problem),
        (status = 422, description = "A parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_periods(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<BookQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<PeriodResponse>, Error> {
    let page = service
        .list_periods(&tenant, &principal, query.book_id, &PageRequest::try_from(page)?)
        .await?;
    Ok(page.map(PeriodResponse::from))
}

/// Close a period.
///
/// Requires `ledger.periods_close` on the book's unit. The period must have ended and every
/// earlier period be closed. Closing records each account's closing balance; no entry can be dated
/// in it afterwards. Cannot be undone; closing a closed period again returns it.
#[utoipa::path(
    post,
    path = "/periods/{id}/close",
    extensions(("x-permissions" = json!(["ledger.periods_close"]))),
    operation_id = "close_ledger_period",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the period.")),
    responses(
        (status = 200, description = "The closed period.", body = Data<PeriodResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.periods_close` (`permission_denied`) or the unit is frozen (`org_frozen`).", body = Problem),
        (status = 404, description = "No such period in your scope; code `ledger.period_not_found`.", body = Problem),
        (status = 409, description = "An earlier period is open (`ledger.period_order`) or the period has not ended (`ledger.period_not_ended`).", body = Problem),
    )
)]
async fn close_period(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<PeriodResponse>, Error> {
    let row = service.close_period(&tenant, &principal, id).await?;
    Ok(Data { data: LedgerService::period_view(&tenant, row).await?.into() })
}

/// One account in a trial balance.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerTrialBalanceRow)]
pub(crate) struct TrialRowResponse {
    account_id: Uuid,
    purpose: String,
    counterparty: Option<Counterparty>,
    external_code: Option<String>,
    normal_side: Side,
    /// Balance at the end of the day before `from`.
    opening: Money,
    /// Debits dated in the range.
    debit: Money,
    /// Credits dated in the range.
    credit: Money,
    /// Balance at the end of `to`.
    closing: Money,
}

/// Debits and credits in one currency: always equal.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerTrialBalanceTotal)]
pub(crate) struct TrialTotal {
    currency: String,
    debit: Money,
    credit: Money,
}

/// A trial balance.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerTrialBalanceResponse)]
pub(crate) struct TrialBalanceResponse {
    book_id: Uuid,
    org_id: Uuid,
    from: Date,
    to: Date,
    accounts: Vec<TrialRowResponse>,
    totals: Vec<TrialTotal>,
}

/// Which book and days.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct TrialQuery {
    /// The book.
    book_id: Uuid,
    /// First day, `YYYY-MM-DD`.
    from: Date,
    /// Last day, `YYYY-MM-DD`.
    to: Date,
}

pub(crate) fn describe_trial(
    book: book::Model,
    from: Date,
    to: Date,
    rows: Vec<TrialRow>,
) -> Result<TrialBalanceResponse, Error> {
    let mut totals: Vec<TrialTotal> = Vec::new();
    let mut accounts = Vec::with_capacity(rows.len());
    for row in rows {
        let account = row.account;
        match totals.iter_mut().find(|total| total.currency == account.currency) {
            Some(total) => {
                total.debit = total.debit.checked_add(row.debit)?;
                total.credit = total.credit.checked_add(row.credit)?;
            }
            None => totals.push(TrialTotal {
                currency: account.currency.clone(),
                debit: row.debit,
                credit: row.credit,
            }),
        }
        let counterparty = (!account.counterparty_kind.is_empty()).then(|| {
            Counterparty::new(account.counterparty_kind.clone(), account.counterparty_id.clone())
        });
        accounts.push(TrialRowResponse {
            account_id: account.id,
            purpose: account.purpose,
            counterparty,
            external_code: account.external_code,
            normal_side: Side::parse(&account.normal_side)
                .ok_or_else(|| Error::internal("ledger account has a bad side"))?,
            opening: row.opening,
            debit: row.debit,
            credit: row.credit,
            closing: row.closing,
        });
    }
    Ok(TrialBalanceResponse { book_id: book.id, org_id: book.org_id, from, to, accounts, totals })
}

/// Get the trial balance of a book for a range of days.
///
/// Requires `ledger.read` on the book's unit. Per account: opening balance, debits, credits and
/// closing balance; per currency, total debits equal total credits.
#[utoipa::path(
    get,
    path = "/trial-balance",
    extensions(("x-permissions" = json!(["ledger.read"]))),
    operation_id = "get_ledger_trial_balance",
    security(("bearer" = [])),
    params(TrialQuery),
    responses(
        (status = 200, description = "The trial balance.", body = Data<TrialBalanceResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such book in your scope; code `ledger.book_not_found`.", body = Problem),
        (status = 422, description = "A parameter is invalid (e.g. `from` after `to`); every problem in `errors`.", body = Problem),
    )
)]
async fn trial_balance(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<TrialQuery>,
) -> Result<Data<TrialBalanceResponse>, Error> {
    let (book, rows) =
        service.trial_balance(&tenant, &principal, query.book_id, query.from, query.to).await?;
    Ok(Data { data: describe_trial(book, query.from, query.to, rows)? })
}
