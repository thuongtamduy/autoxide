//! Pagination for every endpoint that returns a list: cursor-based, keyed on the ID.
//!
//! Every ID is a UUIDv7, so ID order is creation order. The next page is fetched as "the rows whose ID comes
//! after the last ID of the previous page"; the database goes straight to that spot using the primary key
//! index. Unlike `OFFSET n`:
//!
//! - the thousandth page is as fast as the first (no reading and discarding n rows);
//! - rows added while the client is paging do not cause duplicate or missing rows.
//!
//! Usage in a service:
//!
//! ```ignore
//! let page = fetch_page(note::Entity::find(), note::Column::Id, |row| row.id, &request, db.read())
//!     .await?;
//! ```
//!
//! Every list shares the three query parameters `limit`, `cursor`, `order` and the same response shape:
//! learn one endpoint and you know them all.
//!
//! A client that needs the total number of rows matching the filters (a counter tile, a tab badge) sends
//! `count=true`: the first page then also returns `pagination.total`. Counting is a separate `COUNT` query
//! on the same filters, run only when asked and only on the first page (later pages do not count again), so
//! paging stays cheap.
//!
//! A list sorted by another column (name, size...) uses [`fetch_page_by`]: the cursor then carries both the
//! value of that column and the ID ("keyset pagination"), and still goes straight to the spot to read using
//! an index on `(column, id)`, with the same two advantages. The endpoint declares its own `sort` parameter
//! (e.g. `sort=name`); the kernel handles the cursor.

use crate::time::Timestamp;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sea_orm::IdenStatic;
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    Select,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use axum::Json;
use axum::response::{IntoResponse, Response};

use crate::error::{Error, FieldError};

mod memory;
pub use memory::{SortedPageQuery, page_of};

/// Number of rows per page when the client gives no `limit`.
pub const DEFAULT_LIMIT: u32 = 50;
/// Maximum number of rows per page.
pub const MAX_LIMIT: u32 = 200;

/// Sort direction of a list. Lists are ordered by creation time, unless the endpoint offers
/// a `sort` parameter to order by something else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Order {
    /// Oldest first.
    Asc,
    /// Newest first.
    #[default]
    Desc,
}

// Tham số query của mọi endpoint danh sách, đúng như client gửi (toàn chuỗi).
// Nhận chuỗi rồi tự kiểm ở `PageRequest::try_from` để lỗi nói rõ từng tham số sai
// gì, thay vì một câu chung của bộ phân tích query.
/// Query parameters shared by every list endpoint.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[non_exhaustive]
pub struct PageQuery {
    /// Maximum number of items to return, 1 to 200. Defaults to 50.
    #[param(value_type = Option<u32>, minimum = 1, maximum = 200, example = 50)]
    pub limit: Option<String>,
    /// Where to continue: the `next_cursor` of the previous page. Omit for the
    /// first page. Treat the value as opaque.
    #[param(value_type = Option<String>)]
    pub cursor: Option<String>,
    /// `desc` (newest first, the default) or `asc` (oldest first). Keep the same
    /// order (and `sort`, where offered) while following `next_cursor`: changing it midway only
    /// changes where the next page starts.
    #[param(value_type = Option<Order>, inline)]
    pub order: Option<String>,
    /// `true` to also get `pagination.total`, the number of items matching the filters, on the
    /// first page (without `cursor`). Costs one more query; leave it out when not needed.
    #[param(value_type = Option<bool>, example = true)]
    pub count: Option<String>,
}

impl PageQuery {
    /// The query parameters exactly as a client sends them (all text, each optional).
    pub const fn new(limit: Option<String>, cursor: Option<String>, order: Option<String>) -> Self {
        PageQuery { limit, cursor, order, count: None }
    }

    /// Like [`PageQuery::new`], also asking for the total (`count=true`).
    #[must_use]
    pub fn with_count(mut self, count: bool) -> Self {
        self.count = Some(count.to_string());
        self
    }
}

/// A validated page request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRequest {
    limit: u32,
    position: Option<Position>,
    order: Order,
    /// The client asked for the total (`count=true`).
    count: bool,
}

/// Where the next page starts: after the row with this ID, or (for a list sorted by another column) after
/// the row with this column value and this ID.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Position {
    Id(Uuid),
    Key(KeyCursor),
}

/// Value of the sort column (other than the ID) that the cursor carries. The cursor records the value type
/// (`{"i": …}`, `{"s": …}`, `{"t": …}`) so that a string that looks like a date is not misread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SortValue {
    #[serde(rename = "i")]
    Integer(i64),
    #[serde(rename = "s")]
    Text(String),
    #[serde(rename = "t")]
    Time(Timestamp),
}

impl From<SortValue> for sea_orm::Value {
    fn from(value: SortValue) -> Self {
        match value {
            SortValue::Integer(number) => number.into(),
            SortValue::Text(text) => text.into(),
            SortValue::Time(time) => time.into(),
        }
    }
}

/// Cursor of a list sorted by column `column`: the column value and the ID of the last row of the previous
/// page. Sent to the client as base64url of JSON; the client treats it as an opaque string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct KeyCursor {
    #[serde(rename = "c")]
    column: String,
    #[serde(rename = "v")]
    value: SortValue,
    id: Uuid,
}

impl KeyCursor {
    fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).unwrap_or_default())
    }

    fn decode(raw: &str) -> Option<Self> {
        let bytes = URL_SAFE_NO_PAD.decode(raw).ok()?;
        // Giá trị chữ chứa NUL: Postgres lỗi 500, SQLite thì không — coi như con trỏ hỏng
        // (D-157).
        if crate::http::contains_nul_escape(&bytes) {
            return None;
        }
        serde_json::from_slice(&bytes).ok()
    }
}

fn wrong_cursor() -> Error {
    Error::validation(vec![FieldError::new(
        "cursor",
        "invalid",
        "must be the `next_cursor` value of a previous page of the same list and sort, unchanged",
    )])
}

impl PageRequest {
    /// The first page. `limit` is clamped to the range `1..=MAX_LIMIT`.
    pub fn first(limit: u32, order: Order) -> Self {
        PageRequest { limit: limit.clamp(1, MAX_LIMIT), position: None, order, count: false }
    }

    /// Also asks for the total number of rows matching the filters (`pagination.total`).
    #[must_use]
    pub fn counted(mut self) -> Self {
        self.count = true;
        self
    }

    /// Whether to count the total for this page: the client asked, and this is the first page.
    pub fn wants_total(&self) -> bool {
        self.count && self.position.is_none()
    }

    /// The page after the row with ID `cursor`.
    pub fn after(mut self, cursor: Uuid) -> Self {
        self.position = Some(Position::Id(cursor));
        self
    }

    pub fn limit(&self) -> u32 {
        self.limit
    }

    /// The cursor's ID (list sorted by ID). `None` on the first page, or when the cursor belongs to a list
    /// sorted by another column.
    pub fn cursor(&self) -> Option<Uuid> {
        match &self.position {
            Some(Position::Id(id)) => Some(*id),
            _ => None,
        }
    }

    pub fn order(&self) -> Order {
        self.order
    }
}

impl Default for PageRequest {
    fn default() -> Self {
        PageRequest::first(DEFAULT_LIMIT, Order::default())
    }
}

// `TryFrom`: phép đổi kiểu có thể thất bại. Handler viết
// `PageRequest::try_from(query)?` — tham số sai thành lỗi 422 có tên từng tham số.
impl TryFrom<PageQuery> for PageRequest {
    type Error = Error;

    fn try_from(query: PageQuery) -> Result<Self, Self::Error> {
        let mut problems = Vec::new();

        let limit = match query.limit.as_deref().map(str::trim) {
            None | Some("") => DEFAULT_LIMIT,
            Some(raw) => match raw.parse::<u32>() {
                Ok(limit) if (1..=MAX_LIMIT).contains(&limit) => limit,
                _ => {
                    problems.push(
                        FieldError::new(
                            "limit",
                            "range",
                            "must be a whole number between 1 and {max_limit}",
                        )
                        .with_param("max_limit", MAX_LIMIT),
                    );
                    DEFAULT_LIMIT
                }
            },
        };
        let position = match query.cursor.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(raw) => match Uuid::parse_str(raw)
                .ok()
                .map(Position::Id)
                .or_else(|| KeyCursor::decode(raw).map(Position::Key))
            {
                Some(position) => Some(position),
                None => {
                    problems.push(FieldError::new(
                        "cursor",
                        "invalid",
                        "must be the `next_cursor` value of a previous page, unchanged",
                    ));
                    None
                }
            },
        };
        let order = match query.order.as_deref().map(str::trim) {
            None | Some("") => Order::default(),
            Some("asc") => Order::Asc,
            Some("desc") => Order::Desc,
            Some(_) => {
                problems.push(FieldError::new("order", "invalid", "must be `asc` or `desc`"));
                Order::default()
            }
        };

        let count = match query.count.as_deref().map(str::trim) {
            None | Some("" | "false") => false,
            Some("true") => true,
            Some(_) => {
                problems.push(FieldError::new("count", "invalid", "must be `true` or `false`"));
                false
            }
        };

        if problems.is_empty() {
            Ok(PageRequest { limit, position, order, count })
        } else {
            Err(Error::validation(problems))
        }
    }
}

/// One page of a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Page<T> {
    /// The items of this page, in the requested order.
    pub data: Vec<T>,
    pub pagination: Pagination,
}

/// How to get the next page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct Pagination {
    /// The limit that was applied to this page.
    #[schema(example = 50)]
    pub limit: u32,
    /// Pass this as `cursor` to get the next page. `null` means this is the last
    /// page.
    pub next_cursor: Option<String>,
    /// Number of items matching the filters, over all pages. Only on the first page of a request
    /// with `count=true`; left out otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 1234)]
    pub total: Option<u64>,
}

impl Pagination {
    /// The applied limit and the cursor of the next page (`None` on the last page).
    pub const fn new(limit: u32, next_cursor: Option<String>) -> Self {
        Pagination { limit, next_cursor, total: None }
    }

    /// Also carries the total number of rows matching the filters.
    #[must_use]
    pub const fn with_total(mut self, total: Option<u64>) -> Self {
        self.total = total;
        self
    }
}

impl<T> Page<T> {
    /// Converts each row to another type (e.g. from a database row to the API's return type), keeping the
    /// page information.
    pub fn map<U>(self, convert: impl FnMut(T) -> U) -> Page<U> {
        Page { data: self.data.into_iter().map(convert).collect(), pagination: self.pagination }
    }
}

impl<T: Serialize> IntoResponse for Page<T> {
    fn into_response(self) -> Response {
        Json(self).into_response()
    }
}

/// Fetches one page of `select`, ordered by the ID column.
///
/// - `id_column`: the table's primary key column (UUIDv7).
/// - `id_of`: gets the ID from a row, to compute the next page's cursor.
/// - `select` may already carry filter conditions; it must NOT already carry an ordering or a row
///   limit.
pub async fn fetch_page<E, C>(
    select: Select<E>,
    id_column: E::Column,
    id_of: impl Fn(&E::Model) -> Uuid,
    request: &PageRequest,
    connection: &C,
) -> Result<Page<E::Model>, Error>
where
    E: EntityTrait,
    C: ConnectionTrait,
{
    let total = total_of(&select, request, connection).await?;
    let select = match (&request.position, request.order) {
        (None, _) => select,
        (Some(Position::Id(cursor)), Order::Asc) => select.filter(id_column.gt(*cursor)),
        (Some(Position::Id(cursor)), Order::Desc) => select.filter(id_column.lt(*cursor)),
        (Some(Position::Key(_)), _) => return Err(wrong_cursor()),
    };
    let select = match request.order {
        Order::Asc => select.order_by_asc(id_column),
        Order::Desc => select.order_by_desc(id_column),
    };
    let (rows, has_more) = fetch_rows(select, request, connection).await?;
    let next_cursor = if has_more { rows.last().map(|row| id_of(row).to_string()) } else { None };

    Ok(Page { data: rows, pagination: Pagination { limit: request.limit, next_cursor, total } })
}

/// Like [`fetch_page`], but ordered by column `sort_column` first, then by ID (so two rows with the same
/// value still have a fixed order).
///
/// - `sort_value`: that column's value for a row, to compute the next page's cursor. A column that can be
///   `NULL` cannot be used here.
/// - Needs an index on `(sort_column, id)` to be fast.
/// - Both databases must compare the column values the same way: a text column should be stored in a
///   byte-comparable form (e.g. lowercase, collation `C` on Postgres) so the order is identical.
pub async fn fetch_page_by<E, C>(
    select: Select<E>,
    sort_column: E::Column,
    sort_value: impl Fn(&E::Model) -> SortValue,
    id_column: E::Column,
    id_of: impl Fn(&E::Model) -> Uuid,
    request: &PageRequest,
    connection: &C,
) -> Result<Page<E::Model>, Error>
where
    E: EntityTrait,
    C: ConnectionTrait,
{
    let total = total_of(&select, request, connection).await?;
    let column_name = sort_column.as_str();
    let select = match &request.position {
        None => select,
        Some(Position::Key(cursor))
            if cursor.column == column_name && fits(&sort_column, &cursor.value) =>
        {
            let value = sea_orm::Value::from(cursor.value.clone());
            let (after, tie) = match request.order {
                Order::Asc => (sort_column.gt(value.clone()), id_column.gt(cursor.id)),
                Order::Desc => (sort_column.lt(value.clone()), id_column.lt(cursor.id)),
            };
            select.filter(
                Condition::any()
                    .add(after)
                    .add(Condition::all().add(sort_column.eq(value)).add(tie)),
            )
        }
        Some(_) => return Err(wrong_cursor()),
    };
    let select = match request.order {
        Order::Asc => select.order_by_asc(sort_column).order_by_asc(id_column),
        Order::Desc => select.order_by_desc(sort_column).order_by_desc(id_column),
    };
    let (rows, has_more) = fetch_rows(select, request, connection).await?;
    let next_cursor = has_more.then(|| rows.last()).flatten().map(|row| {
        KeyCursor { column: column_name.to_owned(), value: sort_value(row), id: id_of(row) }
            .encode()
    });

    Ok(Page { data: rows, pagination: Pagination { limit: request.limit, next_cursor, total } })
}

/// The "column" name in the cursor of [`fetch_page_tiers`].
const TIER: &str = "tier";

/// Like [`fetch_page`], but splits rows into ranked tiers: every row matching `tiers[0]` comes first,
/// then the rows matching `tiers[1]` (and not the earlier tier)... Within each tier rows are ordered by ID
/// as in [`fetch_page`]; a row matching no tier is dropped. Used for search where "exact accent match ranks
/// before accent-insensitive match" (D-202): the rank depends on what the user typed, so it cannot be stored
/// as a column.
///
/// The cursor carries the tier and the ID of the last row. A page runs at most `tiers.len()` queries, each
/// using the index of the shared filter as in [`fetch_page`].
pub async fn fetch_page_tiers<E, C>(
    select: Select<E>,
    tiers: &[Condition],
    id_column: E::Column,
    id_of: impl Fn(&E::Model) -> Uuid,
    request: &PageRequest,
    connection: &C,
) -> Result<Page<E::Model>, Error>
where
    E: EntityTrait,
    C: ConnectionTrait,
{
    let (start, after) = match &request.position {
        None => (0, None),
        Some(Position::Key(KeyCursor { column, value: SortValue::Integer(tier), id }))
            if column == TIER && usize::try_from(*tier).is_ok_and(|tier| tier < tiers.len()) =>
        {
            (usize::try_from(*tier).unwrap_or_default(), Some(*id))
        }
        Some(_) => return Err(wrong_cursor()),
    };
    let total = if request.wants_total() {
        let any = tiers.iter().fold(Condition::any(), |any, tier| any.add(tier.clone()));
        Some(count_rows(select.clone().filter(any), connection).await?)
    } else {
        None
    };
    let wanted = request.limit as usize + 1;
    let mut rows: Vec<(usize, E::Model)> = Vec::new();
    for tier in start..tiers.len() {
        let mut condition = Condition::all().add(tiers[tier].clone());
        for earlier in &tiers[..tier] {
            condition = condition.add(earlier.clone().not());
        }
        let mut page = select.clone().filter(condition);
        if let (true, Some(id)) = (tier == start, after) {
            page = match request.order {
                Order::Asc => page.filter(id_column.gt(id)),
                Order::Desc => page.filter(id_column.lt(id)),
            };
        }
        let page = match request.order {
            Order::Asc => page.order_by_asc(id_column),
            Order::Desc => page.order_by_desc(id_column),
        };
        let missing = (wanted - rows.len()) as u64;
        let found = page.limit(missing).all(connection).await?;
        rows.extend(found.into_iter().map(|row| (tier, row)));
        if rows.len() >= wanted {
            break;
        }
    }
    let has_more = rows.len() >= wanted;
    rows.truncate(request.limit as usize);
    let next_cursor = has_more.then(|| rows.last()).flatten().map(|(tier, row)| {
        KeyCursor {
            column: TIER.to_owned(),
            value: SortValue::Integer(i64::try_from(*tier).unwrap_or_default()),
            id: id_of(row),
        }
        .encode()
    });
    Ok(Page {
        data: rows.into_iter().map(|(_, row)| row).collect(),
        pagination: Pagination { limit: request.limit, next_cursor, total },
    })
}

/// Total number of rows of `select` (filters only, no cursor) when the client asks and this is the first page.
pub async fn total_of<E, C>(
    select: &Select<E>,
    request: &PageRequest,
    connection: &C,
) -> Result<Option<u64>, Error>
where
    E: EntityTrait,
    C: ConnectionTrait,
{
    if !request.wants_total() {
        return Ok(None);
    }
    Ok(Some(count_rows(select.clone(), connection).await?))
}

/// `SELECT COUNT(*)` wrapped around the `select` query (keeping every filter and join). Built by hand
/// instead of `PaginatorTrait::count` so the paging functions do not need an extra `Sync` bound on the row
/// type.
async fn count_rows<E, C>(select: Select<E>, connection: &C) -> Result<u64, Error>
where
    E: EntityTrait,
    C: ConnectionTrait,
{
    use sea_orm::QueryTrait as _;
    use sea_orm::sea_query::{Alias, Expr, Query};
    let statement = Query::select()
        .expr(Expr::cust("COUNT(*)"))
        .from_subquery(select.into_query(), Alias::new("counted"))
        .to_owned();
    let total: i64 = match connection.query_one(&statement).await? {
        Some(row) => row.try_get_by_index(0)?,
        None => 0,
    };
    Ok(u64::try_from(total).unwrap_or_default())
}

/// Whether the value in the cursor has the column's right type. The cursor comes back from the client, so
/// it may have been tampered with: comparing text with a numeric column makes Postgres fail (500) but not
/// SQLite, so check first so both return `invalid_cursor` (D-149).
fn fits<C: ColumnTrait>(column: &C, value: &SortValue) -> bool {
    use sea_orm::ColumnType;
    match column.def().get_column_type() {
        ColumnType::TinyInteger
        | ColumnType::SmallInteger
        | ColumnType::Integer
        | ColumnType::BigInteger
        | ColumnType::TinyUnsigned
        | ColumnType::SmallUnsigned
        | ColumnType::Unsigned
        | ColumnType::BigUnsigned => matches!(value, SortValue::Integer(_)),
        ColumnType::Char(_) | ColumnType::String(_) | ColumnType::Text => {
            matches!(value, SortValue::Text(_))
        }
        ColumnType::DateTime | ColumnType::Timestamp | ColumnType::TimestampWithTimeZone => {
            matches!(value, SortValue::Time(_))
        }
        // Kiểu khác không dùng để sắp; nếu có thì để database phán.
        _ => true,
    }
}

/// Reads at most `limit` rows, and also learns whether more rows follow.
async fn fetch_rows<E, C>(
    select: Select<E>,
    request: &PageRequest,
    connection: &C,
) -> Result<(Vec<E::Model>, bool), Error>
where
    E: EntityTrait,
    C: ConnectionTrait,
{
    // Lấy dư MỘT dòng để biết còn trang sau hay không mà không cần câu lệnh đếm.
    let mut rows = select.limit(u64::from(request.limit) + 1).all(connection).await?;
    let limit = request.limit as usize;
    let has_more = rows.len() > limit;
    rows.truncate(limit);
    Ok((rows, has_more))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;

    fn query(limit: Option<&str>, cursor: Option<&str>, order: Option<&str>) -> PageQuery {
        PageQuery {
            limit: limit.map(str::to_owned),
            cursor: cursor.map(str::to_owned),
            order: order.map(str::to_owned),
            count: None,
        }
    }

    #[test]
    fn khong_neu_gi_thi_la_trang_dau_moi_nhat_truoc() {
        let request = PageRequest::try_from(PageQuery::default()).unwrap();
        assert_eq!((request.limit(), request.cursor(), request.order()), (50, None, Order::Desc));
        assert_eq!(request, PageRequest::default());
        assert_eq!(PageRequest::try_from(query(Some(""), Some(" "), Some(""))).unwrap(), request);
    }

    #[test]
    fn con_tro_theo_cot_doc_lai_duoc() {
        let cursor = KeyCursor {
            column: "name".to_owned(),
            value: SortValue::Text("ảnh 1".to_owned()),
            id: Uuid::now_v7(),
        };
        let raw = cursor.encode();
        let request = PageRequest::try_from(query(None, Some(&raw), None)).unwrap();
        assert_eq!(request.position, Some(Position::Key(cursor)));
        assert_eq!(request.cursor(), None);
        let number =
            KeyCursor { column: "size".to_owned(), value: SortValue::Integer(-5), id: Uuid::nil() };
        assert_eq!(KeyCursor::decode(&number.encode()), Some(number));
        let time = Timestamp::from_unix_micros(1_700_000_000_123_456).unwrap();
        let time =
            KeyCursor { column: "at".to_owned(), value: SortValue::Time(time), id: Uuid::nil() };
        assert_eq!(KeyCursor::decode(&time.encode()), Some(time));
        // Chuỗi trông như ngày vẫn là chuỗi.
        let text = SortValue::Text("2024-01-01T00:00:00Z".to_owned());
        let text = KeyCursor { column: "name".to_owned(), value: text, id: Uuid::nil() };
        assert_eq!(KeyCursor::decode(&text.encode()), Some(text));
        assert!(PageRequest::try_from(query(None, Some("e30"), None)).is_err());
    }

    #[test]
    fn tham_so_hop_le_duoc_nhan() {
        let id = Uuid::now_v7();
        let request =
            PageRequest::try_from(query(Some("200"), Some(&id.to_string()), Some("asc"))).unwrap();
        assert_eq!(
            (request.limit(), request.cursor(), request.order()),
            (200, Some(id), Order::Asc)
        );
        assert_eq!(PageRequest::try_from(query(Some("1"), None, Some("desc"))).unwrap().limit(), 1);
    }

    #[test]
    fn tham_so_sai_thanh_loi_422_neu_ten_tung_tham_so() {
        let error = PageRequest::try_from(query(Some("0"), Some("not-a-cursor"), Some("newest")))
            .unwrap_err();

        assert_eq!(error.kind(), ErrorKind::Validation);
        let fields: Vec<(&str, &str)> =
            error.fields().iter().map(|f| (f.field.as_str(), f.message.as_str())).collect();
        assert_eq!(
            fields,
            [
                ("limit", "must be a whole number between 1 and 200"),
                ("cursor", "must be the `next_cursor` value of a previous page, unchanged"),
                ("order", "must be `asc` or `desc`"),
            ]
        );
        for limit in ["201", "-1", "ten", "1.5"] {
            assert!(PageRequest::try_from(query(Some(limit), None, None)).is_err(), "{limit}");
        }
    }

    #[test]
    fn count_chi_dem_o_trang_dau() {
        let first = PageRequest::try_from(PageQuery::new(None, None, None).with_count(true));
        assert!(first.unwrap().wants_total());
        let next = PageQuery::new(None, Some(Uuid::now_v7().to_string()), None).with_count(true);
        assert!(!PageRequest::try_from(next).unwrap().wants_total());
        assert!(!PageRequest::default().wants_total());
        assert!(PageRequest::default().counted().wants_total());
        let wrong = PageQuery { count: Some("yes".to_owned()), ..PageQuery::default() };
        let error = PageRequest::try_from(wrong).unwrap_err();
        assert_eq!(error.fields()[0].field, "count");
    }

    #[test]
    fn trang_dau_tao_bang_code_bi_kep_vao_khoang_hop_le() {
        assert_eq!(PageRequest::first(0, Order::Asc).limit(), 1);
        assert_eq!(PageRequest::first(10_000, Order::Asc).limit(), MAX_LIMIT);
        let id = Uuid::now_v7();
        assert_eq!(PageRequest::first(10, Order::Asc).after(id).cursor(), Some(id));
    }

    #[test]
    fn page_thanh_json_va_doi_kieu_duoc() {
        let page = Page { data: vec![1, 2], pagination: Pagination::new(2, Some("c".to_owned())) };

        let mapped = page.map(|number| number.to_string());

        assert_eq!(
            serde_json::to_value(&mapped).unwrap(),
            serde_json::json!({"data": ["1", "2"], "pagination": {"limit": 2, "next_cursor": "c"}})
        );
        let last =
            Page::<u8> { data: vec![], pagination: Pagination::new(2, None).with_total(Some(0)) };
        assert_eq!(
            serde_json::to_value(&last).unwrap()["pagination"],
            serde_json::json!({"limit": 2, "next_cursor": null, "total": 0})
        );
    }

    // Property test (đặc tả 31 B3): con trỏ đi một vòng mã hoá / giải mã nguyên vẹn; chuỗi bất kỳ
    // làm con trỏ không bao giờ panic — hoặc là con trỏ hợp lệ, hoặc lỗi 422 `cursor`.
    fn sort_value() -> impl proptest::strategy::Strategy<Value = SortValue> {
        use proptest::prelude::*;
        prop_oneof![
            any::<i64>().prop_map(SortValue::Integer),
            "[^\\x00]{0,40}".prop_map(SortValue::Text),
            (0i64..4_102_444_800_000_000).prop_map(|micros| {
                SortValue::Time(Timestamp::from_unix_micros(micros).unwrap())
            }),
        ]
    }

    proptest::proptest! {
        #[test]
        fn con_tro_ma_hoa_roi_giai_ma_nguyen_ven(
            column in "[a-z_]{1,20}",
            value in sort_value(),
            id in proptest::prelude::any::<u128>(),
        ) {
            let cursor = KeyCursor { column, value, id: Uuid::from_u128(id) };
            proptest::prop_assert_eq!(KeyCursor::decode(&cursor.encode()), Some(cursor));
        }

        #[test]
        fn con_tro_rac_khong_panic(raw in "\\PC{0,120}") {
            let _ = KeyCursor::decode(&raw);
            let query = PageQuery::new(None, Some(raw.clone()), None);
            if let Err(error) = PageRequest::try_from(query) {
                proptest::prop_assert_eq!(error.fields()[0].field.as_str(), "cursor");
            }
        }
    }
}
