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

use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Select,
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

/// Sort direction of a list. Items are always ordered by creation time.
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
pub struct PageQuery {
    /// Maximum number of items to return, 1 to 200. Defaults to 50.
    #[param(value_type = Option<u32>, minimum = 1, maximum = 200, example = 50)]
    pub limit: Option<String>,
    /// Where to continue: the `next_cursor` of the previous page. Omit for the
    /// first page. Treat the value as opaque.
    #[param(value_type = Option<String>)]
    pub cursor: Option<String>,
    /// `desc` (newest first, the default) or `asc` (oldest first). Keep the same
    /// order while following `next_cursor`.
    #[param(value_type = Option<Order>, inline)]
    pub order: Option<String>,
}

/// Yêu cầu một trang, đã kiểm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageRequest {
    limit: u32,
    cursor: Option<Uuid>,
    order: Order,
}

impl PageRequest {
    /// Trang đầu tiên. `limit` bị kẹp vào khoảng `1..=MAX_LIMIT`.
    pub fn first(limit: u32, order: Order) -> Self {
        PageRequest { limit: limit.clamp(1, MAX_LIMIT), cursor: None, order }
    }

    /// Trang đứng sau dòng có ID `cursor`.
    pub fn after(mut self, cursor: Uuid) -> Self {
        self.cursor = Some(cursor);
        self
    }

    pub fn limit(&self) -> u32 {
        self.limit
    }

    pub fn cursor(&self) -> Option<Uuid> {
        self.cursor
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
                    problems.push(FieldError::new(
                        "limit",
                        "range",
                        format!("must be a whole number between 1 and {MAX_LIMIT}"),
                    ));
                    DEFAULT_LIMIT
                }
            },
        };
        let cursor = match query.cursor.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(raw) => match Uuid::parse_str(raw) {
                Ok(cursor) => Some(cursor),
                Err(_) => {
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
            Ok(PageRequest { limit, cursor, order })
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
pub struct Pagination {
    /// The limit that was applied to this page.
    #[schema(example = 50)]
    pub limit: u32,
    /// Pass this as `cursor` to get the next page. `null` means this is the last
    /// page.
    pub next_cursor: Option<String>,
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
    let select = match (request.cursor, request.order) {
        (None, _) => select,
        (Some(cursor), Order::Asc) => select.filter(id_column.gt(cursor)),
        (Some(cursor), Order::Desc) => select.filter(id_column.lt(cursor)),
    };
    let select = match request.order {
        Order::Asc => select.order_by_asc(id_column),
        Order::Desc => select.order_by_desc(id_column),
    };
    // Lấy dư MỘT dòng để biết còn trang sau hay không mà không cần câu lệnh đếm.
    let mut rows = select.limit(u64::from(request.limit) + 1).all(connection).await?;

    let limit = request.limit as usize;
    let has_more = rows.len() > limit;
    rows.truncate(limit);
    let next_cursor = if has_more { rows.last().map(|row| id_of(row).to_string()) } else { None };

    Ok(Page { data: rows, pagination: Pagination { limit: request.limit, next_cursor } })
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
        // Tham số rỗng (`?limit=&cursor=`) coi như không nêu.
        assert_eq!(PageRequest::try_from(query(Some(""), Some(" "), Some(""))).unwrap(), request);
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
}
