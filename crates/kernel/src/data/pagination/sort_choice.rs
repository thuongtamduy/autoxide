//! Query parameters of a list with a `sort` choice (step 70 P9, D-20261125-list-pages), e.g. by creation time
//! or by name. The same `limit` / `cursor` / `order` / `count` as [`PageQuery`], but the default of `order`
//! follows the chosen sort: newest first for a time, A→Z (smallest first) for a name, position, code or size.
//! The handler, which knows the chosen sort, gets the request with [`SortChoicePageQuery::into_request`].

use serde::Deserialize;
use utoipa::IntoParams;

use super::{Order, PageQuery, PageRequest};
use crate::error::Error;

// Như `PageQuery`, chỉ khác: `order` không có mặc định cố định — theo cột `sort` đang chọn.
/// Query parameters of a list with a `sort` choice.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[non_exhaustive]
pub struct SortChoicePageQuery {
    /// Maximum number of items to return, 1 to 200. Defaults to 50.
    #[param(value_type = Option<u32>, minimum = 1, maximum = 200, example = 50)]
    pub limit: Option<String>,
    /// Where to continue: the `next_cursor` of the previous page. Omit for the
    /// first page. Treat the value as opaque.
    #[param(value_type = Option<String>)]
    pub cursor: Option<String>,
    // Mô tả cho OpenAPI ở `order_param.rs` (`schema_with` bỏ qua doc comment).
    /// `asc` or `desc`; the default follows `sort`.
    #[param(schema_with = super::order_param::by_sort)]
    pub order: Option<String>,
    /// `true` to also get `pagination.total`, the number of items matching the filters, on the
    /// first page (without `cursor`). Costs one more query; leave it out when not needed.
    #[param(value_type = Option<bool>, example = true)]
    pub count: Option<String>,
}

impl SortChoicePageQuery {
    /// The query parameters exactly as a client sends them (all text, each optional).
    pub const fn new(limit: Option<String>, cursor: Option<String>, order: Option<String>) -> Self {
        SortChoicePageQuery { limit, cursor, order, count: None }
    }

    /// Like [`SortChoicePageQuery::new`], also asking for the total (`count=true`).
    #[must_use]
    pub fn with_count(mut self, count: bool) -> Self {
        self.count = Some(count.to_string());
        self
    }

    // Không cài `TryFrom` cho kiểu này: mặc định của `order` phụ thuộc `sort` mà chỉ handler biết, nên chuyển
    // đổi bắt buộc nêu mặc định — quên thì không có `PageRequest`, thay vì lặng lẽ ra `desc` trái tài liệu.
    /// The validated page request; `default` is the order when the client sent none, from the chosen `sort`:
    /// [`Order::Desc`] for a time (newest first), [`Order::Asc`] for a name, position, code or size.
    pub fn into_request(self, default: Order) -> Result<PageRequest, Error> {
        PageRequest::parse(
            PageQuery {
                limit: self.limit,
                cursor: self.cursor,
                order: self.order,
                count: self.count,
            },
            default,
        )
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
}
