//! Pagination cho mọi endpoint trả danh sách: theo con trỏ (cursor), dựa trên ID.
//!
//! Mọi ID là UUIDv7 nên thứ tự ID chính là thứ tự tạo. Trang kế tiếp được lấy
//! bằng "các dòng có ID đứng sau ID cuối của trang trước" — database đi thẳng tới
//! chỗ đó bằng index của khoá chính. Khác với kiểu `OFFSET n`:
//!
//! - trang thứ một nghìn nhanh như trang đầu (không phải đọc rồi bỏ n dòng);
//! - dữ liệu được thêm trong lúc client đang lật trang không làm trùng hay sót dòng.
//!
//! Cách dùng trong service:
//!
//! ```ignore
//! let page = fetch_page(note::Entity::find(), note::Column::Id, |row| row.id, &request, db.read())
//!     .await?;
//! ```
//!
//! Mọi danh sách dùng chung ba tham số query `limit`, `cursor`, `order` và cùng
//! hình dạng response — học một endpoint là biết mọi endpoint.
//!
//! Danh sách sắp theo một cột khác (tên, kích thước…) dùng [`fetch_page_by`]: con trỏ
//! khi đó mang cả giá trị của cột đó lẫn ID ("keyset pagination"), vẫn đi thẳng tới chỗ
//! cần đọc bằng index `(cột, id)` — cùng hai ưu điểm trên. Endpoint tự khai tham số
//! `sort` của mình (vd `sort=name`), kernel lo con trỏ.

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

/// Số dòng một trang khi client không nêu `limit`.
pub const DEFAULT_LIMIT: u32 = 50;
/// Số dòng tối đa một trang.
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
    /// order (and `sort`, where offered) while following `next_cursor`.
    #[param(value_type = Option<Order>, inline)]
    pub order: Option<String>,
}

impl PageQuery {
    /// The query parameters exactly as a client sends them (all text, each optional).
    pub const fn new(limit: Option<String>, cursor: Option<String>, order: Option<String>) -> Self {
        PageQuery { limit, cursor, order }
    }
}

/// Yêu cầu một trang, đã kiểm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRequest {
    limit: u32,
    position: Option<Position>,
    order: Order,
}

/// Chỗ trang kế bắt đầu: sau dòng có ID này, hoặc (danh sách sắp theo cột khác) sau dòng
/// có giá trị cột này và ID này.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Position {
    Id(Uuid),
    Key(KeyCursor),
}

/// Giá trị của cột sắp xếp (ngoài ID) mà con trỏ mang theo. Con trỏ ghi kèm loại giá trị
/// (`{"i": …}`, `{"s": …}`, `{"t": …}`), để một chuỗi trông như ngày không bị đọc nhầm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SortValue {
    #[serde(rename = "i")]
    Integer(i64),
    #[serde(rename = "s")]
    Text(String),
    #[serde(rename = "t")]
    Time(chrono::DateTime<chrono::Utc>),
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

/// Con trỏ của danh sách sắp theo cột `column`: giá trị cột và ID của dòng cuối trang
/// trước. Gửi cho client dạng base64url của JSON — client coi là chuỗi mờ.
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
    /// Trang đầu tiên. `limit` bị kẹp vào khoảng `1..=MAX_LIMIT`.
    pub fn first(limit: u32, order: Order) -> Self {
        PageRequest { limit: limit.clamp(1, MAX_LIMIT), position: None, order }
    }

    /// Trang đứng sau dòng có ID `cursor`.
    pub fn after(mut self, cursor: Uuid) -> Self {
        self.position = Some(Position::Id(cursor));
        self
    }

    pub fn limit(&self) -> u32 {
        self.limit
    }

    /// ID của con trỏ (danh sách sắp theo ID). `None` ở trang đầu, hoặc khi con trỏ là của
    /// danh sách sắp theo cột khác.
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

        if problems.is_empty() {
            Ok(PageRequest { limit, position, order })
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
}

impl Pagination {
    /// The applied limit and the cursor of the next page (`None` on the last page).
    pub const fn new(limit: u32, next_cursor: Option<String>) -> Self {
        Pagination { limit, next_cursor }
    }
}

impl<T> Page<T> {
    /// Đổi từng dòng sang kiểu khác (vd từ dòng của database sang kiểu trả về của
    /// API), giữ nguyên thông tin trang.
    pub fn map<U>(self, convert: impl FnMut(T) -> U) -> Page<U> {
        Page { data: self.data.into_iter().map(convert).collect(), pagination: self.pagination }
    }
}

impl<T: Serialize> IntoResponse for Page<T> {
    fn into_response(self) -> Response {
        Json(self).into_response()
    }
}

/// Lấy một trang của `select`, sắp theo cột ID.
///
/// - `id_column`: cột khoá chính (UUIDv7) của bảng.
/// - `id_of`: lấy ID từ một dòng, để tính con trỏ của trang kế.
/// - `select` có thể đã mang sẵn điều kiện lọc; KHÔNG được mang sẵn thứ tự sắp
///   xếp hay giới hạn số dòng.
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

    Ok(Page { data: rows, pagination: Pagination { limit: request.limit, next_cursor } })
}

/// Như [`fetch_page`], nhưng sắp theo cột `sort_column` trước, rồi theo ID (để hai dòng
/// cùng giá trị vẫn có thứ tự cố định).
///
/// - `sort_value`: giá trị cột đó của một dòng, để tính con trỏ của trang kế. Cột có thể
///   `NULL` thì không dùng được ở đây.
/// - Cần index `(sort_column, id)` để nhanh.
/// - Hai database phải so sánh giá trị cột như nhau: cột chữ nên lưu sẵn dạng so sánh
///   được theo byte (vd chữ thường, collation `C` trên Postgres) để thứ tự giống nhau.
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

    Ok(Page { data: rows, pagination: Pagination { limit: request.limit, next_cursor } })
}

/// Tên "cột" trong con trỏ của [`fetch_page_tiers`].
const TIER: &str = "tier";

/// Như [`fetch_page`], nhưng chia dòng thành nhiều tầng xếp hạng: mọi dòng khớp `tiers[0]` đứng
/// trước, rồi các dòng khớp `tiers[1]` (mà không khớp tầng trước)… Trong mỗi tầng sắp theo ID
/// như [`fetch_page`]; dòng không khớp tầng nào bị bỏ. Dùng cho tìm kiếm "khớp đúng dấu xếp
/// trước khớp không dấu" (D-202): hạng phụ thuộc chữ người dùng gõ nên không lưu thành cột được.
///
/// Con trỏ mang tầng và ID của dòng cuối. Một trang chạy tối đa `tiers.len()` câu truy vấn, mỗi
/// câu đi theo index của bộ lọc chung như [`fetch_page`].
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
        pagination: Pagination { limit: request.limit, next_cursor },
    })
}

/// Giá trị trong con trỏ có đúng kiểu của cột không. Con trỏ do client gửi lại nên có thể
/// bị sửa: so chữ với cột số làm Postgres báo lỗi (500) còn SQLite thì không — kiểm trước
/// để cả hai cùng trả `invalid_cursor` (D-149).
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

/// Đọc tối đa `limit` dòng; kèm biết còn dòng sau hay không.
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
        let time = chrono::DateTime::from_timestamp_micros(1_700_000_000_123_456).unwrap();
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
    fn trang_dau_tao_bang_code_bi_kep_vao_khoang_hop_le() {
        assert_eq!(PageRequest::first(0, Order::Asc).limit(), 1);
        assert_eq!(PageRequest::first(10_000, Order::Asc).limit(), MAX_LIMIT);
        let id = Uuid::now_v7();
        assert_eq!(PageRequest::first(10, Order::Asc).after(id).cursor(), Some(id));
    }

    #[test]
    fn page_thanh_json_va_doi_kieu_duoc() {
        let page = Page {
            data: vec![1, 2],
            pagination: Pagination { limit: 2, next_cursor: Some("c".to_owned()) },
        };

        let mapped = page.map(|number| number.to_string());

        assert_eq!(
            serde_json::to_value(&mapped).unwrap(),
            serde_json::json!({"data": ["1", "2"], "pagination": {"limit": 2, "next_cursor": "c"}})
        );
        let last =
            Page::<u8> { data: vec![], pagination: Pagination { limit: 2, next_cursor: None } };
        assert_eq!(
            serde_json::to_value(&last).unwrap()["pagination"]["next_cursor"],
            serde_json::Value::Null
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
                SortValue::Time(chrono::DateTime::from_timestamp_micros(micros).unwrap())
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
            let query = PageQuery { limit: None, cursor: Some(raw.clone()), order: None };
            if let Err(error) = PageRequest::try_from(query) {
                proptest::prop_assert_eq!(error.fields()[0].field.as_str(), "cursor");
            }
        }
    }
}
