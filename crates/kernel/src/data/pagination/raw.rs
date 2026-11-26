//! Keyset paging of a hand-built `sea_query` select (the kernel's own tables have no SeaORM entity): the same
//! cursor, ordering and `count` as [`fetch_page`](super::fetch_page) / [`fetch_page_by`](super::fetch_page_by),
//! in the database (D-20261125-list-pages). Crate-internal: modules page with their entities.

use sea_orm::sea_query::{
    Alias, Condition, Expr, ExprTrait, IntoColumnRef, Query, SelectStatement,
};
use sea_orm::{ConnectionTrait, QueryResult};
use uuid::Uuid;

use super::{KeyCursor, Order, Page, PageRequest, Pagination, Position, SortValue, wrong_cursor};
use crate::error::Error;

/// One page of `select` (filters only: no ordering, no row limit), ordered by the UUIDv7 column `id_column`.
/// `id_of` reads that ID from a row, for the next page's cursor.
pub(crate) async fn fetch_statement_page<K, C>(
    mut select: SelectStatement,
    id_column: K,
    id_of: impl Fn(&QueryResult) -> Result<Uuid, Error>,
    request: &PageRequest,
    connection: &C,
) -> Result<Page<QueryResult>, Error>
where
    K: IntoColumnRef + Clone,
    C: ConnectionTrait,
{
    let total = total_of_statement(&select, request, connection).await?;
    match (&request.position, request.order) {
        (None, _) => {}
        (Some(Position::Id(cursor)), Order::Asc) => {
            select.and_where(Expr::col(id_column.clone()).gt(*cursor));
        }
        (Some(Position::Id(cursor)), Order::Desc) => {
            select.and_where(Expr::col(id_column.clone()).lt(*cursor));
        }
        (Some(Position::Key(_)), _) => return Err(wrong_cursor()),
    }
    select.order_by(id_column, sql_order(request.order));
    let (rows, has_more) = rows_of(select, request, connection).await?;
    let next_cursor = match (has_more, rows.last()) {
        (true, Some(row)) => Some(id_of(row)?.to_string()),
        _ => None,
    };
    Ok(Page { data: rows, pagination: Pagination { limit: request.limit, next_cursor, total } })
}

/// Like [`fetch_statement_page`], ordered by `sort_column` (NOT NULL) first, then by ID. `sort_name` names
/// the ordering in the cursor (a cursor of another ordering is a 422); `fits` tells whether a cursor's value
/// has the column's type (a tampered cursor must not reach the database: Postgres fails comparing text with a
/// number, SQLite does not — D-149); `sort_of` reads the column's value from a row. Needs an index on the
/// filter columns + `(sort_column, id)`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn fetch_statement_page_by<S, K, C>(
    mut select: SelectStatement,
    sort_column: S,
    sort_name: &str,
    fits: impl Fn(&SortValue) -> bool,
    sort_of: impl Fn(&QueryResult) -> Result<SortValue, Error>,
    id_column: K,
    id_of: impl Fn(&QueryResult) -> Result<Uuid, Error>,
    request: &PageRequest,
    connection: &C,
) -> Result<Page<QueryResult>, Error>
where
    S: IntoColumnRef + Clone,
    K: IntoColumnRef + Clone,
    C: ConnectionTrait,
{
    let total = total_of_statement(&select, request, connection).await?;
    match &request.position {
        None => {}
        Some(Position::Key(cursor)) if cursor.column == sort_name && fits(&cursor.value) => {
            let value = sea_orm::Value::from(cursor.value.clone());
            let sort = Expr::col(sort_column.clone());
            let id = Expr::col(id_column.clone());
            // `col >= v AND (col > v OR id > id0)`, như `fetch_page_by`: cận trước cho database nhảy thẳng tới
            // điểm trong index.
            let (bound, after, tie) = match request.order {
                Order::Asc => (sort.clone().gte(value.clone()), sort.gt(value), id.gt(cursor.id)),
                Order::Desc => (sort.clone().lte(value.clone()), sort.lt(value), id.lt(cursor.id)),
            };
            select.and_where(bound).cond_where(Condition::any().add(after).add(tie));
        }
        Some(_) => return Err(wrong_cursor()),
    }
    select
        .order_by(sort_column, sql_order(request.order))
        .order_by(id_column, sql_order(request.order));
    let (rows, has_more) = rows_of(select, request, connection).await?;
    let next_cursor = match (has_more, rows.last()) {
        (true, Some(row)) => Some(
            KeyCursor { column: sort_name.to_owned(), value: sort_of(row)?, id: id_of(row)? }
                .encode(),
        ),
        _ => None,
    };
    Ok(Page { data: rows, pagination: Pagination { limit: request.limit, next_cursor, total } })
}

/// Converts every row of a page, keeping the page information; the first failing row fails the page.
pub(crate) fn try_map_page<T>(
    page: Page<QueryResult>,
    convert: impl Fn(&QueryResult) -> Result<T, Error>,
) -> Result<Page<T>, Error> {
    let data = page.data.iter().map(convert).collect::<Result<Vec<T>, Error>>()?;
    Ok(Page { data, pagination: page.pagination })
}

fn sql_order(order: Order) -> sea_orm::sea_query::Order {
    match order {
        Order::Asc => sea_orm::sea_query::Order::Asc,
        Order::Desc => sea_orm::sea_query::Order::Desc,
    }
}

async fn rows_of<C: ConnectionTrait>(
    mut select: SelectStatement,
    request: &PageRequest,
    connection: &C,
) -> Result<(Vec<QueryResult>, bool), Error> {
    // Lấy dư một dòng để biết còn trang sau mà không cần đếm.
    select.limit(u64::from(request.limit) + 1);
    let mut rows = connection.query_all(&select).await?;
    let limit = request.limit as usize;
    let has_more = rows.len() > limit;
    rows.truncate(limit);
    Ok((rows, has_more))
}

/// `COUNT(*)` over `select` (filters only), when the client asks and this is the first page.
async fn total_of_statement<C: ConnectionTrait>(
    select: &SelectStatement,
    request: &PageRequest,
    connection: &C,
) -> Result<Option<u64>, Error> {
    if !request.wants_total() {
        return Ok(None);
    }
    let statement = Query::select()
        .expr(Expr::cust("COUNT(*)"))
        .from_subquery(select.clone(), Alias::new("counted"))
        .to_owned();
    let total: i64 = match connection.query_one(&statement).await? {
        Some(row) => row.try_get_by_index(0)?,
        None => 0,
    };
    Ok(Some(u64::try_from(total).unwrap_or_default()))
}

/// The page request, or one 422 with every problem of the filters (`problems`) and of the paging parameters
/// together (D-083).
pub(crate) fn with_filter_problems(
    mut problems: Vec<crate::error::FieldError>,
    page: Result<PageRequest, Error>,
) -> Result<PageRequest, Error> {
    match page {
        Ok(page) if problems.is_empty() => Ok(page),
        Ok(_) => Err(Error::validation(problems)),
        Err(error) if error.fields().is_empty() => Err(error),
        Err(error) => {
            problems.extend_from_slice(error.fields());
            Err(Error::validation(problems))
        }
    }
}

/// The sort column of [`fetch_statement_page_filtered`] (as in [`fetch_statement_page_by`]).
pub(crate) struct RawSort<'a> {
    pub column: sea_orm::sea_query::ColumnRef,
    /// Name of the ordering in the cursor.
    pub name: &'a str,
    pub fits: fn(&SortValue) -> bool,
    pub value_of: fn(&QueryResult) -> Result<SortValue, Error>,
}

/// At most this many batches of [`MAX_LIMIT`](super::MAX_LIMIT) rows are read for one page of a filtered list
/// ([`fetch_statement_page_filtered`]), and again at most this many to count it.
pub(crate) const SCAN_BATCHES: usize = 10;

/// Like [`fetch_statement_page`] (`sort: None`) or [`fetch_statement_page_by`], keeping only the rows `keep`
/// accepts — for a rule SQL cannot test (who may see a row depends on its permissions; push as much of the rule
/// as possible into `select` first). The rows are read in batches of [`MAX_LIMIT`](super::MAX_LIMIT) from the
/// cursor on, keyset in the database, at most [`SCAN_BATCHES`] batches per request:
///
/// - the page fills and one more kept row is found: `next_cursor` is the last row of the page;
/// - the list ends: no `next_cursor`;
/// - the scan budget runs out first: the rows found so far (maybe none) and a `next_cursor` at the LAST SCANNED
///   row, so the next request goes on from there. A page shorter than `limit`, even empty, with a `next_cursor`
///   is therefore normal for these lists: keep following the cursor until it is `null`.
///
/// With `count=true` the first page also counts the kept rows while walking from the start, within the same
/// budget; `pagination.total` is left out when the list is longer than the budget.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn fetch_statement_page_filtered<C: ConnectionTrait>(
    select: SelectStatement,
    sort: Option<&RawSort<'_>>,
    id_column: sea_orm::sea_query::ColumnRef,
    id_of: fn(&QueryResult) -> Result<Uuid, Error>,
    keep: impl Fn(&QueryResult) -> Result<bool, Error>,
    request: &PageRequest,
    connection: &C,
) -> Result<Page<QueryResult>, Error> {
    let walker = Walker {
        select: &select,
        sort,
        id_column: &id_column,
        id_of,
        keep: &keep,
        order: request.order,
    };
    let (found, resume, _) =
        walker.walk(request.position.clone(), Some(request.limit as usize), connection).await?;
    let next_cursor = resume.map(|position| match position {
        Position::Id(id) => id.to_string(),
        Position::Key(cursor) => cursor.encode(),
    });
    let total = if request.wants_total() {
        let (_, resume, kept) = walker.walk(None, None, connection).await?;
        // Danh sách dài hơn ngân sách quét: không biết tổng, bỏ `total` thay vì báo sai.
        resume.is_none().then(|| u64::try_from(kept).unwrap_or(u64::MAX))
    } else {
        None
    };
    Ok(Page { data: found, pagination: Pagination { limit: request.limit, next_cursor, total } })
}

/// What one walk over a filtered list needs.
struct Walker<'a, K> {
    select: &'a SelectStatement,
    sort: Option<&'a RawSort<'a>>,
    id_column: &'a sea_orm::sea_query::ColumnRef,
    id_of: fn(&QueryResult) -> Result<Uuid, Error>,
    keep: &'a K,
    order: Order,
}

impl<K: Fn(&QueryResult) -> Result<bool, Error>> Walker<'_, K> {
    /// The position right after `row` (its cursor).
    fn position_of(&self, row: &QueryResult) -> Result<Position, Error> {
        Ok(match self.sort {
            None => Position::Id((self.id_of)(row)?),
            Some(sort) => Position::Key(KeyCursor {
                column: sort.name.to_owned(),
                value: (sort.value_of)(row)?,
                id: (self.id_of)(row)?,
            }),
        })
    }

    /// Reads batches from `start` on, at most [`SCAN_BATCHES`]. `wanted`: keep at most that many rows (`None`:
    /// keep none, only count). Returns the kept rows, where the next page starts (`None`: the list is over) and
    /// how many rows were kept in all (with `wanted: None`, the count).
    async fn walk<C: ConnectionTrait>(
        &self,
        start: Option<Position>,
        wanted: Option<usize>,
        connection: &C,
    ) -> Result<(Vec<QueryResult>, Option<Position>, usize), Error> {
        let mut found: Vec<QueryResult> = Vec::new();
        let mut kept = 0usize;
        let mut batch = PageRequest {
            limit: super::MAX_LIMIT,
            position: start,
            order: self.order,
            count: false,
        };
        for _ in 0..SCAN_BATCHES {
            let rows = match self.sort {
                None => {
                    fetch_statement_page(
                        self.select.clone(),
                        self.id_column.clone(),
                        self.id_of,
                        &batch,
                        connection,
                    )
                    .await?
                }
                Some(sort) => {
                    fetch_statement_page_by(
                        self.select.clone(),
                        sort.column.clone(),
                        sort.name,
                        sort.fits,
                        sort.value_of,
                        self.id_column.clone(),
                        self.id_of,
                        &batch,
                        connection,
                    )
                    .await?
                }
            };
            let more = rows.pagination.next_cursor.is_some();
            let last = match rows.data.last() {
                Some(last) => Some(self.position_of(last)?),
                None => None,
            };
            for row in rows.data {
                if !(self.keep)(&row)? {
                    continue;
                }
                kept += 1;
                if let Some(wanted) = wanted {
                    if found.len() == wanted {
                        // Trang đã đủ và còn một dòng được xem nữa: trang sau bắt đầu sau dòng cuối của trang.
                        let resume = match found.last() {
                            Some(row) => Some(self.position_of(row)?),
                            None => None,
                        };
                        return Ok((found, resume, kept));
                    }
                    found.push(row);
                }
            }
            match last {
                Some(position) if more => batch.position = Some(position),
                _ => return Ok((found, None, kept)),
            }
        }
        // Hết ngân sách quét: trang sau đi tiếp từ dòng cuối đã quét (có thể trả trang ngắn, kể cả rỗng).
        Ok((found, batch.position, kept))
    }
}

/// Merges one page of each of two lists keyed on UUIDv7 IDs (each read with the same `page`: the same cursor
/// ID, order and limit) into one page of the combined list. Each list gave its first `limit` rows after the
/// cursor, so the first `limit` of the merge are the combined list's; more follow when either list had more or
/// the merge has more than `limit`. The total (with `count=true`) is the sum.
pub(crate) fn merge_id_pages<T>(
    first: Page<T>,
    second: Option<Page<T>>,
    page: &PageRequest,
    id_of: impl Fn(&T) -> Uuid,
) -> Page<T> {
    let Some(second) = second else { return first };
    let mut more =
        first.pagination.next_cursor.is_some() || second.pagination.next_cursor.is_some();
    let total = match (first.pagination.total, second.pagination.total) {
        (Some(a), Some(b)) => Some(a.saturating_add(b)),
        _ => None,
    };
    let mut items: Vec<T> = first.data.into_iter().chain(second.data).collect();
    match page.order() {
        Order::Asc => items.sort_by_key(|item| id_of(item)),
        Order::Desc => items.sort_by_key(|item| std::cmp::Reverse(id_of(item))),
    }
    let limit = page.limit() as usize;
    more |= items.len() > limit;
    items.truncate(limit);
    let next_cursor = more.then(|| items.last().map(|item| id_of(item).to_string())).flatten();
    Page { data: items, pagination: Pagination::new(page.limit(), next_cursor).with_total(total) }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Hai nguồn (ghi chú, event) đọc cùng con trỏ rồi gộp theo id: đi hết bằng `next_cursor` ra đúng danh sách
    // gộp, không trùng không sót, cả hai chiều; tổng là tổng hai nguồn.
    #[test]
    fn gop_hai_nguon_theo_id_ra_dung_trang() {
        let ids: Vec<Uuid> = (0..7).map(|_| Uuid::now_v7()).collect();
        let notes: Vec<Uuid> = ids.iter().copied().step_by(2).collect();
        let events: Vec<Uuid> = ids.iter().copied().skip(1).step_by(2).collect();
        // Một trang của một nguồn sau con trỏ, như `fetch_page` (thứ tự theo id, dư một dòng thì có trang sau).
        let page_of_source = |source: &[Uuid], page: &PageRequest| -> Page<Uuid> {
            let mut rows: Vec<Uuid> = source.to_vec();
            if page.order() == Order::Desc {
                rows.reverse();
            }
            if let Some(cursor) = page.cursor() {
                rows.retain(|id| match page.order() {
                    Order::Asc => *id > cursor,
                    Order::Desc => *id < cursor,
                });
            }
            let limit = page.limit() as usize;
            let more = rows.len() > limit;
            rows.truncate(limit);
            let next = more.then(|| rows.last().map(Uuid::to_string)).flatten();
            let total = page.wants_total().then_some(source.len() as u64);
            Page { data: rows, pagination: Pagination::new(page.limit(), next).with_total(total) }
        };
        for order in [Order::Asc, Order::Desc] {
            let mut seen = Vec::new();
            let mut page = PageRequest::first(2, order).counted();
            loop {
                let merged = merge_id_pages(
                    page_of_source(&notes, &page),
                    Some(page_of_source(&events, &page)),
                    &page,
                    |id| *id,
                );
                if page.wants_total() {
                    assert_eq!(merged.pagination.total, Some(7));
                }
                assert!(merged.data.len() <= 2);
                seen.extend(merged.data);
                match merged.pagination.next_cursor {
                    Some(next) => page = PageRequest::first(2, order).after(next.parse().unwrap()),
                    None => break,
                }
            }
            let mut expected = ids.clone();
            if order == Order::Desc {
                expected.reverse();
            }
            assert_eq!(seen, expected, "{order:?}");
        }
    }
}
