//! Query parameters of a list with a `sort` choice (step 70 P9, D-20261125-list-pages), e.g. by creation time
//! or by name. The same `limit` / `cursor` / `order` / `count` as [`PageQuery`], but the default of `order`
//! follows the chosen sort: newest first for a time, A→Z (smallest first) for a name, position, code or size.
//! The handler, which knows the chosen sort, gets the request with [`SortChoicePageQuery::into_request`].

use serde::Deserialize;

use super::{Order, PageQuery, PageRequest};
use crate::error::Error;

// Bọc `PageQuery` như `NaturalOrderPageQuery`; trường riêng tư nên không vòng qua `into_request` được.
/// Query parameters of a list with a `sort` choice.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct SortChoicePageQuery(PageQuery);

impl SortChoicePageQuery {
    /// The query parameters exactly as a client sends them (all text, each optional).
    pub const fn new(limit: Option<String>, cursor: Option<String>, order: Option<String>) -> Self {
        SortChoicePageQuery(PageQuery::new(limit, cursor, order))
    }

    /// Like [`SortChoicePageQuery::new`], also asking for the total (`count=true`).
    #[must_use]
    pub fn with_count(self, count: bool) -> Self {
        SortChoicePageQuery(self.0.with_count(count))
    }

    // Không cài `TryFrom` cho kiểu này: mặc định của `order` phụ thuộc `sort` mà chỉ handler biết, nên chuyển
    // đổi bắt buộc nêu mặc định — quên thì không có `PageRequest`, thay vì lặng lẽ ra `desc` trái tài liệu.
    /// The validated page request; `default` is the order when the client sent none, from the chosen `sort`:
    /// [`Order::Desc`] for a time (newest first), [`Order::Asc`] for a name, position, code or size.
    pub fn into_request(self, default: Order) -> Result<PageRequest, Error> {
        PageRequest::parse(self.0, default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mặc định của `order` theo cột sắp; `order` client gửi thì giữ nguyên.
    #[test]
    fn mac_dinh_cua_order_theo_cot_sap_khong_de_lua_chon_cua_client() {
        let request = |order: Option<&str>, default: Order| {
            SortChoicePageQuery::new(None, None, order.map(str::to_owned))
                .into_request(default)
                .unwrap()
                .order()
        };
        assert_eq!(request(None, Order::Asc), Order::Asc);
        assert_eq!(request(None, Order::Desc), Order::Desc);
        assert_eq!(request(Some(" "), Order::Asc), Order::Asc);
        assert_eq!(request(Some("desc"), Order::Asc), Order::Desc);
        assert_eq!(request(Some("asc"), Order::Desc), Order::Asc);
        let wrong =
            SortChoicePageQuery::new(None, None, Some("up".to_owned())).into_request(Order::Asc);
        assert_eq!(wrong.unwrap_err().fields()[0].field, "order");
    }

    // Đọc từ query string đúng như `PageQuery` (bọc `transparent`).
    #[test]
    fn doc_tu_query_nhu_page_query() {
        let uri: axum::http::Uri = "/x?limit=3&order=asc&count=true&other=x".parse().unwrap();
        let axum::extract::Query(query) =
            axum::extract::Query::<SortChoicePageQuery>::try_from_uri(&uri).unwrap();
        let request = query.into_request(Order::Desc).unwrap();
        assert_eq!(
            (request.limit(), request.order(), request.wants_total()),
            (3, Order::Asc, true)
        );
    }
}
