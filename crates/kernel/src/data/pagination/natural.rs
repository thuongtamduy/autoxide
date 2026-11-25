//! Query parameters of a list in a natural order (step 70 P9, D-20261125-list-pages): a position, a name, a
//! code — not creation time. The same `limit` / `cursor` / `order` / `count` as [`PageQuery`], but `order`
//! defaults to `asc` (the natural order); `desc` reverses it. Rule: a list whose only order is creation time
//! takes [`PageQuery`] (newest first); a list with a natural order takes this.

use serde::Deserialize;
use utoipa::IntoParams;

use super::{Order, PageQuery, PageRequest};
use crate::error::Error;

// Như `PageQuery`, chỉ khác mặc định của `order` (danh sách theo thứ tự tự nhiên: vị trí, tên, mã).
/// Query parameters of a list in a natural order (position, name, code).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[non_exhaustive]
pub struct NaturalOrderPageQuery {
    /// Maximum number of items to return, 1 to 200. Defaults to 50.
    #[param(value_type = Option<u32>, minimum = 1, maximum = 200, example = 50)]
    pub limit: Option<String>,
    /// Where to continue: the `next_cursor` of the previous page. Omit for the
    /// first page. Treat the value as opaque.
    #[param(value_type = Option<String>)]
    pub cursor: Option<String>,
    // Mô tả cho OpenAPI ở `order_param.rs` (`schema_with` bỏ qua doc comment).
    /// `asc` (the list's natural order, the default) or `desc` (reversed).
    #[param(schema_with = super::order_param::natural)]
    pub order: Option<String>,
    /// `true` to also get `pagination.total`, the number of items matching the filters, on the
    /// first page (without `cursor`). Costs one more query; leave it out when not needed.
    #[param(value_type = Option<bool>, example = true)]
    pub count: Option<String>,
}

impl NaturalOrderPageQuery {
    /// The query parameters exactly as a client sends them (all text, each optional).
    pub const fn new(limit: Option<String>, cursor: Option<String>, order: Option<String>) -> Self {
        NaturalOrderPageQuery { limit, cursor, order, count: None }
    }

    /// Like [`NaturalOrderPageQuery::new`], also asking for the total (`count=true`).
    #[must_use]
    pub fn with_count(mut self, count: bool) -> Self {
        self.count = Some(count.to_string());
        self
    }
}

// `TryFrom`: như `PageQuery`, tham số sai thành lỗi 422 có tên từng tham số.
impl TryFrom<NaturalOrderPageQuery> for PageRequest {
    type Error = Error;

    fn try_from(query: NaturalOrderPageQuery) -> Result<Self, Self::Error> {
        PageRequest::parse(
            PageQuery {
                limit: query.limit,
                cursor: query.cursor,
                order: query.order,
                count: query.count,
            },
            Order::Asc,
        )
    }
}
