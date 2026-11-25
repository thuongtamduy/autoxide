//! Lists in a natural order, and lists held in memory (step 70 P9, one list shape for every endpoint).
//!
//! - [`SortedPageQuery`]: the query parameters of a list whose natural order is not creation time (a
//!   position, a name, a code): the same `limit` / `cursor` / `order` / `count` as [`PageQuery`], but `order`
//!   defaults to `asc` (the natural order); `desc` reverses it.
//! - [`page_of`]: one page of a list that is already in memory — built from code (the module registry, a
//!   catalog), from platform reference data, or from a query with a hard cap. Items are ordered by a unique
//!   text key, compared byte by byte, so the order (and the cursor) is the same on SQLite and Postgres.

use serde::Deserialize;
use utoipa::IntoParams;
use uuid::Uuid;

use super::{
    KeyCursor, Order, Page, PageQuery, PageRequest, Pagination, Position, SortValue, wrong_cursor,
};
use crate::error::Error;

/// Column name the cursor of [`page_of`] carries.
const KEY: &str = "key";

// Như `PageQuery`, chỉ khác mặc định của `order` (danh sách theo thứ tự tự nhiên: vị trí, tên, mã).
/// Query parameters of a list in a natural order (position, name, code).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[non_exhaustive]
pub struct SortedPageQuery {
    /// Maximum number of items to return, 1 to 200. Defaults to 50.
    #[param(value_type = Option<u32>, minimum = 1, maximum = 200, example = 50)]
    pub limit: Option<String>,
    /// Where to continue: the `next_cursor` of the previous page. Omit for the
    /// first page. Treat the value as opaque.
    #[param(value_type = Option<String>)]
    pub cursor: Option<String>,
    /// `asc` (the list's natural order, the default; the endpoint says what it is) or `desc`
    /// (reversed). Keep the same order while following `next_cursor`.
    #[param(value_type = Option<Order>, inline)]
    pub order: Option<String>,
    /// `true` to also get `pagination.total`, the number of items matching the filters, on the
    /// first page (without `cursor`). Costs one more query; leave it out when not needed.
    #[param(value_type = Option<bool>, example = true)]
    pub count: Option<String>,
}

impl SortedPageQuery {
    /// The query parameters exactly as a client sends them (all text, each optional).
    pub const fn new(limit: Option<String>, cursor: Option<String>, order: Option<String>) -> Self {
        SortedPageQuery { limit, cursor, order, count: None }
    }
}

// `TryFrom`: như `PageQuery`, tham số sai thành lỗi 422 có tên từng tham số.
impl TryFrom<SortedPageQuery> for PageRequest {
    type Error = Error;

    fn try_from(query: SortedPageQuery) -> Result<Self, Self::Error> {
        let natural = query.order.as_deref().is_none_or(|order| order.trim().is_empty());
        let mut request = PageRequest::try_from(PageQuery {
            limit: query.limit,
            cursor: query.cursor,
            order: query.order,
            count: query.count,
        })?;
        if natural {
            request.order = Order::Asc;
        }
        Ok(request)
    }
}

/// One page of `items`, a list already in memory, ordered by `key` (unique per item; compared byte by byte)
/// in the request's order. The cursor carries the key of the last item, so the next page starts right after
/// it even if items were added or removed in between. `pagination.total` (with `count=true`) is the number of
/// items.
pub fn page_of<T>(
    mut items: Vec<T>,
    key: impl Fn(&T) -> String,
    request: &PageRequest,
) -> Result<Page<T>, Error> {
    let total = request.wants_total().then(|| u64::try_from(items.len()).unwrap_or(u64::MAX));
    // Khoá tính một lần mỗi phần tử; sắp theo byte nên hai database (và mọi máy) ra cùng thứ tự.
    let mut keyed: Vec<(String, T)> = items.drain(..).map(|item| (key(&item), item)).collect();
    keyed.sort_by(|(a, _), (b, _)| a.as_bytes().cmp(b.as_bytes()));
    if request.order == Order::Desc {
        keyed.reverse();
    }
    let start = match &request.position {
        None => 0,
        Some(Position::Key(cursor)) if cursor.column == KEY => {
            let SortValue::Text(after) = &cursor.value else { return Err(wrong_cursor()) };
            let after = after.as_bytes();
            keyed
                .iter()
                .position(|(key, _)| match request.order {
                    Order::Asc => key.as_bytes() > after,
                    Order::Desc => key.as_bytes() < after,
                })
                .unwrap_or(keyed.len())
        }
        Some(_) => return Err(wrong_cursor()),
    };
    let limit = request.limit as usize;
    let mut rest = keyed.split_off(start.min(keyed.len()));
    let has_more = rest.len() > limit;
    rest.truncate(limit);
    let next_cursor = has_more.then(|| rest.last()).flatten().map(|(key, _)| {
        KeyCursor { column: KEY.to_owned(), value: SortValue::Text(key.clone()), id: Uuid::nil() }
            .encode()
    });
    Ok(Page {
        data: rest.into_iter().map(|(_, item)| item).collect(),
        pagination: Pagination { limit: request.limit, next_cursor, total },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(limit: &str, cursor: Option<String>, order: Option<&str>) -> PageRequest {
        let query = SortedPageQuery {
            limit: Some(limit.to_owned()),
            cursor,
            order: order.map(str::to_owned),
            count: Some("true".to_owned()),
        };
        PageRequest::try_from(query).unwrap()
    }

    fn names(page: &Page<&'static str>) -> Vec<&'static str> {
        page.data.clone()
    }

    #[test]
    fn duyet_het_danh_sach_trong_bo_nho_theo_khoa() {
        let items = vec!["delta", "alpha", "charlie", "bravo", "echo"];
        let first =
            page_of(items.clone(), |item| (*item).to_owned(), &request("2", None, None)).unwrap();
        assert_eq!(names(&first), ["alpha", "bravo"]);
        assert_eq!(first.pagination.total, Some(5));
        let cursor = first.pagination.next_cursor.clone();
        let second =
            page_of(items.clone(), |item| (*item).to_owned(), &request("2", cursor, None)).unwrap();
        assert_eq!(names(&second), ["charlie", "delta"]);
        assert_eq!(second.pagination.total, None, "only the first page counts");
        // Một phần tử thêm vào giữa chừng không làm trùng hay sót phần đã qua.
        let mut grown = items.clone();
        grown.push("aardvark");
        let third = page_of(
            grown,
            |item| (*item).to_owned(),
            &request("2", second.pagination.next_cursor.clone(), None),
        )
        .unwrap();
        assert_eq!(names(&third), ["echo"]);
        assert_eq!(third.pagination.next_cursor, None);

        let reversed = page_of(items, |item| (*item).to_owned(), &request("3", None, Some("desc")));
        assert_eq!(names(&reversed.unwrap()), ["echo", "delta", "charlie"]);
    }

    #[test]
    fn con_tro_la_bi_tu_choi_va_mac_dinh_tang_dan() {
        let id_cursor = Some(Uuid::now_v7().to_string());
        let error = page_of(vec!["a"], |item| (*item).to_owned(), &request("2", id_cursor, None));
        assert!(error.is_err());
        assert_eq!(request("2", None, None).order(), Order::Asc);
        assert_eq!(PageRequest::try_from(PageQuery::default()).unwrap().order(), Order::Desc);
        let too_many = SortedPageQuery::new(Some("201".to_owned()), None, None);
        assert!(PageRequest::try_from(too_many).is_err());
    }
}
