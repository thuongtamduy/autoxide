//! Kỳ kế toán (tháng), khoá sổ, số dư tại một ngày, bảng cân đối thử.
//!
//! Số dư suy từ bút toán: `ledger_account_balances` chỉ là bản cộng dồn để kiểm sàn nhanh. Số dư
//! cuối ngày D = số dư cuối kỳ khoá gần nhất (≤ D, ảnh chụp lúc khoá ở `ledger_period_balances`) +
//! tổng các dòng sau kỳ đó tới D. Tổng dùng `CAST(SUM(…) AS BIGINT)`: Postgres trả `numeric` cho
//! `SUM(bigint)`.

use autoxide_kernel::time::Date;
use std::collections::HashMap;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use sea_orm::sea_query::{Expr, Query, SimpleExpr};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde_json::json;
use uuid::Uuid;

use crate::api::Side;
use crate::entity::{account, book, entry, line, period, period_balance, seal};
use crate::events;
use crate::permissions::{PERIODS_CLOSE, READ};
use crate::service::{LedgerService, money, now};

fn period_not_found() -> Error {
    Error::not_found("ledger.period_not_found", "There is no such accounting period.")
}

fn period_order(previous: Date) -> Error {
    Error::conflict("ledger.period_order", "Close the earlier period ({previous}) before this one.")
        .with_param("previous", previous.format("%Y-%m-%d").to_string())
}

fn period_not_ended(ends_on: Date) -> Error {
    Error::conflict(
        "ledger.period_not_ended",
        "The period ends on {date}; close it after that day.",
    )
    .with_param("date", ends_on.format("%Y-%m-%d").to_string())
}

/// `SUM` có dấu theo bên nợ (nợ +, có −), ép về `BIGINT` để giải mã `i64` trên cả hai database.
fn signed_sum() -> SimpleExpr {
    Expr::cust(
        "CAST(SUM(CASE WHEN side = 'debit' THEN amount_minor ELSE 0 - amount_minor END) AS BIGINT)",
    )
}

fn side_sum(side: Side) -> SimpleExpr {
    Expr::cust(format!(
        "CAST(SUM(CASE WHEN side = '{}' THEN amount_minor ELSE 0 END) AS BIGINT)",
        side.as_str()
    ))
}

/// Đổi số có dấu theo bên nợ sang số theo bên thường của tài khoản.
pub(crate) fn to_normal(signed: i64, normal_side: &str) -> i64 {
    if normal_side == "credit" { signed.saturating_neg() } else { signed }
}

/// Tổng có dấu theo bên nợ của các dòng thuộc `accounts`, ngày trong `(after, through]`, theo tài
/// khoản. `after = None`: từ đầu.
pub(crate) async fn signed_sums<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    accounts: Option<&[Uuid]>,
    after: Option<Date>,
    through: Option<Date>,
) -> Result<HashMap<Uuid, i64>, Error> {
    let mut select = line::Entity::find()
        .scoped(tenant)
        .select_only()
        .column(line::Column::AccountId)
        .column_as(signed_sum(), "total")
        .group_by(line::Column::AccountId);
    if let Some(accounts) = accounts {
        select = select.filter(line::Column::AccountId.is_in(accounts.to_vec()));
    }
    if let Some(after) = after {
        select = select.filter(line::Column::EffectiveDate.gt(after));
    }
    if let Some(through) = through {
        select = select.filter(line::Column::EffectiveDate.lte(through));
    }
    let rows: Vec<(Uuid, Option<i64>)> = select.into_tuple().all(connection).await?;
    Ok(rows.into_iter().map(|(id, total)| (id, total.unwrap_or(0))).collect())
}

/// Kỳ khoá gần nhất của sổ kết thúc trước hoặc đúng ngày `on`.
async fn last_closed<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    book_id: Uuid,
    on: Date,
) -> Result<Option<period::Model>, Error> {
    Ok(period::Entity::find()
        .scoped(tenant)
        .filter(period::Column::BookId.eq(book_id))
        .filter(period::Column::Status.eq("closed"))
        .filter(period::Column::EndsOn.lte(on))
        .order_by_desc(period::Column::EndsOn)
        .one(connection)
        .await?)
}

/// Số dư theo bên thường cuối ngày `on` của các tài khoản (của một sổ): ảnh chụp kỳ khoá gần
/// nhất + các dòng sau đó.
pub(crate) async fn balances_as_of<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    book_id: Uuid,
    accounts: &[account::Model],
    on: Date,
) -> Result<HashMap<Uuid, i64>, Error> {
    let ids: Vec<Uuid> = accounts.iter().map(|row| row.id).collect();
    let snapshot = last_closed(connection, tenant, book_id, on).await?;
    let mut totals: HashMap<Uuid, i64> = HashMap::new();
    let after = match &snapshot {
        Some(closed) => {
            let rows = period_balance::Entity::find()
                .scoped(tenant)
                .filter(period_balance::Column::PeriodId.eq(closed.id))
                .filter(period_balance::Column::AccountId.is_in(ids.clone()))
                .all(connection)
                .await?;
            totals.extend(rows.into_iter().map(|row| (row.account_id, row.closing_minor)));
            Some(closed.ends_on)
        }
        None => None,
    };
    let sums = signed_sums(connection, tenant, Some(&ids), after, Some(on)).await?;
    for found in accounts {
        let movement = to_normal(sums.get(&found.id).copied().unwrap_or(0), &found.normal_side);
        let slot = totals.entry(found.id).or_insert(0);
        *slot = slot.saturating_add(movement);
    }
    Ok(totals)
}

/// Một dòng của bảng cân đối thử.
#[derive(Debug, Clone)]
pub(crate) struct TrialRow {
    pub account: account::Model,
    pub opening: Money,
    pub debit: Money,
    pub credit: Money,
    pub closing: Money,
}

impl LedgerService {
    pub(crate) async fn list_periods(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        book_id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<PeriodView>, Error> {
        let book = self.book_for(tenant, principal, &READ, book_id).await?;
        let select =
            period::Entity::find().scoped(tenant).filter(period::Column::BookId.eq(book.id));
        let read = tenant.database().read();
        let rows = fetch_page(select, period::Column::Id, |row| row.id, page, read).await?;
        let ids: Vec<Uuid> = rows.data.iter().map(|row| row.id).collect();
        let counts = period_counts(read, tenant, book.id, &ids).await?;
        Ok(rows.map(|row| {
            let (entries, sealed) = counts.get(&row.id).copied().unwrap_or_default();
            PeriodView { period: row, entries, sealed }
        }))
    }

    /// Một kỳ kèm số bút toán (dùng sau khi khoá kỳ).
    pub(crate) async fn period_view(
        tenant: &TenantContext,
        row: period::Model,
    ) -> Result<PeriodView, Error> {
        let counts =
            period_counts(tenant.database().read(), tenant, row.book_id, &[row.id]).await?;
        let (entries, sealed) = counts.get(&row.id).copied().unwrap_or_default();
        Ok(PeriodView { period: row, entries, sealed })
    }

    /// Số dư (theo bên thường) của tài khoản: hiện tại, hay cuối ngày `as_of`.
    pub(crate) async fn account_balance<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        found: &account::Model,
        as_of: Option<Date>,
    ) -> Result<Money, Error> {
        match as_of {
            None => Ok(Self::account_view(connection, tenant, found.clone()).await?.balance),
            Some(on) => {
                let totals = balances_as_of(
                    connection,
                    tenant,
                    found.book_id,
                    std::slice::from_ref(found),
                    on,
                )
                .await?;
                money(totals.get(&found.id).copied().unwrap_or(0), &found.currency)
            }
        }
    }

    pub(crate) async fn balance_for(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        as_of: Option<Date>,
    ) -> Result<(account::Model, Money), Error> {
        let found = self.account_for(tenant, principal, &READ, id).await?;
        let amount = Self::account_balance(tenant.database().read(), tenant, &found, as_of).await?;
        Ok((found, amount))
    }

    /// Bảng cân đối thử của sổ `book_id` cho các ngày `[from, to]`: số dư đầu, phát sinh nợ / có,
    /// số dư cuối của từng tài khoản.
    pub(crate) async fn trial_balance(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        book_id: Uuid,
        from: Date,
        to: Date,
    ) -> Result<(book::Model, Vec<TrialRow>), Error> {
        if from > to {
            return Err(Error::validation(vec![autoxide_kernel::error::FieldError::new(
                "from",
                "range",
                "must not be after `to`",
            )]));
        }
        let book = self.book_for(tenant, principal, &READ, book_id).await?;
        let read = tenant.database().read();
        let accounts = account::Entity::find()
            .scoped(tenant)
            .filter(account::Column::BookId.eq(book.id))
            .order_by_asc(account::Column::Purpose)
            .order_by_asc(account::Column::Id)
            .all(read)
            .await?;
        let opening = match from.pred() {
            Some(day) => balances_as_of(read, tenant, book.id, &accounts, day).await?,
            None => HashMap::new(),
        };
        let movements: Vec<(Uuid, Option<i64>, Option<i64>)> = line::Entity::find()
            .scoped(tenant)
            .select_only()
            .column(line::Column::AccountId)
            .column_as(side_sum(Side::Debit), "debit")
            .column_as(side_sum(Side::Credit), "credit")
            .filter(
                line::Column::AccountId
                    .is_in(accounts.iter().map(|row| row.id).collect::<Vec<_>>()),
            )
            .filter(line::Column::EffectiveDate.gte(from))
            .filter(line::Column::EffectiveDate.lte(to))
            .group_by(line::Column::AccountId)
            .into_tuple()
            .all(read)
            .await?;
        let movements: HashMap<Uuid, (i64, i64)> = movements
            .into_iter()
            .map(|(id, debit, credit)| (id, (debit.unwrap_or(0), credit.unwrap_or(0))))
            .collect();
        let mut rows = Vec::with_capacity(accounts.len());
        for found in accounts {
            let start = opening.get(&found.id).copied().unwrap_or(0);
            let (debit, credit) = movements.get(&found.id).copied().unwrap_or((0, 0));
            let net = to_normal(debit.saturating_sub(credit), &found.normal_side);
            let closing = start.saturating_add(net);
            rows.push(TrialRow {
                opening: money(start, &found.currency)?,
                debit: money(debit, &found.currency)?,
                credit: money(credit, &found.currency)?,
                closing: money(closing, &found.currency)?,
                account: found,
            });
        }
        Ok((book, rows))
    }

    /// Khoá kỳ `id`: kỳ trước phải đã khoá; ghi số dư cuối kỳ; không mở lại được. Khoá lại kỳ đã
    /// khoá trả kỳ đó (gọi lại an toàn).
    pub(crate) async fn close_period(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<period::Model, Error> {
        principal.require(&PERIODS_CLOSE)?;
        let read = tenant.database().read();
        let found = period::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(period_not_found)?;
        let owner = book::Entity::find_by_id(found.book_id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(period_not_found)?;
        let org = OrgId::from_uuid(owner.org_id);
        principal.require_at(read, &PERIODS_CLOSE, OrgTarget::new(org, None)).await.map_err(
            |error| {
                if error.code() == "org_out_of_scope" { period_not_found() } else { error }
            },
        )?;
        let today = self.today(tenant, org).await;
        let (database, outbox) = (tenant.database(), &self.outbox);
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                principal
                    .require_write_at(&transaction, &PERIODS_CLOSE, OrgTarget::new(org, None))
                    .await?;
                // Khoá độc quyền dòng sổ: chờ mọi bút toán đang giữ khoá chia sẻ trên nó.
                let book = rows::lock_one(
                    &transaction,
                    book::Entity::find_by_id(found.book_id).scoped(tenant),
                )
                .await?
                .ok_or_else(period_not_found)?;
                let current = period::Entity::find_by_id(found.id)
                    .scoped(tenant)
                    .one(&transaction)
                    .await?
                    .ok_or_else(period_not_found)?;
                if current.status == "closed" {
                    return Ok(current);
                }
                if current.ends_on >= today {
                    return Err(period_not_ended(current.ends_on));
                }
                let earlier = period::Entity::find()
                    .scoped(tenant)
                    .filter(period::Column::BookId.eq(book.id))
                    .filter(period::Column::Status.eq("open"))
                    .filter(period::Column::StartsOn.lt(current.starts_on))
                    .order_by_asc(period::Column::StartsOn)
                    .one(&transaction)
                    .await?;
                if let Some(earlier) = earlier {
                    return Err(period_order(earlier.starts_on));
                }
                let accounts = account::Entity::find()
                    .scoped(tenant)
                    .filter(account::Column::BookId.eq(book.id))
                    .all(&transaction)
                    .await?;
                let closing =
                    balances_as_of(&transaction, tenant, book.id, &accounts, current.ends_on)
                        .await?;
                let snapshot: Vec<period_balance::ActiveModel> = accounts
                    .iter()
                    .map(|found| period_balance::ActiveModel {
                        period_id: Set(current.id),
                        account_id: Set(found.id),
                        tenant_id: Set(tenant.id().as_uuid()),
                        closing_minor: Set(closing.get(&found.id).copied().unwrap_or(0)),
                        currency: Set(found.currency.clone()),
                    })
                    .collect();
                if !snapshot.is_empty() {
                    period_balance::Entity::insert_many(snapshot)
                        .exec_without_returning(&transaction)
                        .await?;
                }
                let at = now();
                let mut row: period::ActiveModel = current.clone().into();
                row.status = Set("closed".to_owned());
                row.closed_at = Set(Some(at));
                row.closed_by = Set(Some(principal.user_id()));
                let saved = period::Entity::update(row).exec(&transaction).await?;
                let through = book
                    .closed_through
                    .map_or(current.ends_on, |through| through.max(current.ends_on));
                let mut owner: book::ActiveModel = book.clone().into();
                owner.closed_through = Set(Some(through));
                book::Entity::update(owner).exec(&transaction).await?;
                let event = events::PeriodClosed(json!({
                    "period_id": saved.id, "book_id": book.id, "org_id": book.org_id,
                    "starts_on": saved.starts_on.format("%Y-%m-%d").to_string(),
                    "ends_on": saved.ends_on.format("%Y-%m-%d").to_string(),
                }));
                outbox
                    .record(
                        &transaction,
                        tenant,
                        Actor::from(principal),
                        Some(format!("ledger.period:{}", saved.id)),
                        &event,
                    )
                    .await?;
                transaction.commit().await?;
                Ok(saved)
            })
            .await
    }

    /// Số dư (theo bên thường) của tài khoản mang khoá `key` (`dyn Ledger`); chưa có: 0.
    pub(crate) async fn balance_of_key(
        &self,
        tenant: &TenantContext,
        key: &crate::api::AccountKey,
        as_of: Option<Date>,
    ) -> Result<Money, Error> {
        let read = tenant.database().read();
        let Some(book) = Self::find_book(read, tenant, key.book).await? else {
            return Ok(Money::zero(key.currency));
        };
        let (kind, id) = key
            .counterparty
            .as_ref()
            .map_or((String::new(), String::new()), |party| (party.kind.clone(), party.id.clone()));
        let found = account::Entity::find()
            .scoped(tenant)
            .filter(account::Column::BookId.eq(book.id))
            .filter(account::Column::Purpose.eq(key.purpose.as_str()))
            .filter(account::Column::CounterpartyKind.eq(kind))
            .filter(account::Column::CounterpartyId.eq(id))
            .filter(account::Column::Currency.eq(key.currency.code()))
            .one(read)
            .await?;
        match found {
            Some(found) => Self::account_balance(read, tenant, &found, as_of).await,
            None => Ok(Money::zero(key.currency)),
        }
    }

    /// Tài khoản lệch giữa bảng số dư cộng dồn và tổng các dòng (doctor `ledger.balances`), trên
    /// `database`: (khách, tài khoản, số trong bảng, số tính lại).
    pub(crate) async fn drifted(
        database: &autoxide_kernel::database::Database,
    ) -> Result<Vec<(Uuid, Uuid, i64, i64)>, Error> {
        use crate::entity::balance;
        let snapshot = database.begin_snapshot().await?;
        let accounts: Vec<(Uuid, Uuid, String)> = account::Entity::find()
            .select_only()
            .column(account::Column::Id)
            .column(account::Column::TenantId)
            .column(account::Column::NormalSide)
            .into_tuple()
            .all(&snapshot)
            .await?;
        let stored: HashMap<Uuid, i64> = balance::Entity::find()
            .all(&snapshot)
            .await?
            .into_iter()
            .map(|row| (row.account_id, row.balance_minor))
            .collect();
        let sums: Vec<(Uuid, Option<i64>)> = line::Entity::find()
            .select_only()
            .column(line::Column::AccountId)
            .column_as(signed_sum(), "total")
            .group_by(line::Column::AccountId)
            .into_tuple()
            .all(&snapshot)
            .await?;
        let sums: HashMap<Uuid, i64> =
            sums.into_iter().map(|(id, total)| (id, total.unwrap_or(0))).collect();
        let mut drifted = Vec::new();
        for (id, tenant, side) in accounts {
            let expected = to_normal(sums.get(&id).copied().unwrap_or(0), &side);
            let actual = stored.get(&id).copied().unwrap_or(0);
            if expected != actual {
                drifted.push((tenant, id, actual, expected));
            }
        }
        Ok(drifted)
    }
}

/// Một kỳ kèm số bút toán ghi vào kỳ và số đã niêm phong vào chuỗi băm.
#[derive(Debug, Clone)]
pub(crate) struct PeriodView {
    pub period: period::Model,
    pub entries: i64,
    pub sealed: i64,
}

/// Số bút toán và số đã niêm phong của từng kỳ trong `periods` (cùng sổ `book_id`): hai câu đếm
/// gộp theo kỳ cho cả trang, không đếm từng kỳ một.
async fn period_counts<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    book_id: Uuid,
    periods: &[Uuid],
) -> Result<HashMap<Uuid, (i64, i64)>, Error> {
    if periods.is_empty() {
        return Ok(HashMap::new());
    }
    let count = |sealed_only: bool| {
        let mut select = entry::Entity::find()
            .scoped(tenant)
            .select_only()
            .column(entry::Column::PeriodId)
            .column_as(entry::Column::Id.count(), "count")
            .filter(entry::Column::BookId.eq(book_id))
            .filter(entry::Column::PeriodId.is_in(periods.to_vec()))
            .group_by(entry::Column::PeriodId);
        if sealed_only {
            // Câu con lọc theo khách và sổ để đi theo khoá chính của bảng niêm phong.
            let sealed = Query::select()
                .column(seal::Column::EntryId)
                .from(seal::Entity)
                .and_where(seal::Column::TenantId.eq(tenant.id().as_uuid()))
                .and_where(seal::Column::BookId.eq(book_id))
                .to_owned();
            select = select.filter(entry::Column::Id.in_subquery(sealed));
        }
        select.into_tuple::<(Uuid, i64)>()
    };
    let mut counts: HashMap<Uuid, (i64, i64)> = HashMap::new();
    for (period, total) in count(false).all(connection).await? {
        counts.entry(period).or_default().0 = total;
    }
    for (period, sealed) in count(true).all(connection).await? {
        counts.entry(period).or_default().1 = sealed;
    }
    Ok(counts)
}
