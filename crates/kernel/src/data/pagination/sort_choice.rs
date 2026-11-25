//! Query parameters of a list with a `sort` choice (step 70 P9, D-20261125-list-pages), e.g. by creation time
//! or by name. The same `limit` / `cursor` / `order` / `count` as [`PageQuery`], but the default of `order`
//! follows the chosen sort: newest first for a time, A→Z (smallest first) for a name, position, code or size.
//! The handler applies it with [`PageRequest::default_order`].

use serde::Deserialize;
use utoipa::IntoParams;

use super::{PageQuery, PageRequest};
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
    /// `asc` or `desc`. Default: follows `sort` — `desc` (newest first) for a time, `asc` (A→Z,
    /// smallest first) for a name, position, code or size. Text is ordered by the database's
    /// collation. Keep the same order (and `sort`) while following `next_cursor`: changing it
    /// midway only changes where the next page starts.
    #[param(schema_with = super::order_schema_by_sort)]
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
}

// `TryFrom`: như `PageQuery`; mặc định của `order` do handler áp sau bằng `default_order` (nó mới biết `sort`).
// Quên gọi thì ra `desc` như `PageQuery`.
impl TryFrom<SortChoicePageQuery> for PageRequest {
    type Error = Error;

    fn try_from(query: SortChoicePageQuery) -> Result<Self, Self::Error> {
        PageRequest::try_from(PageQuery {
            limit: query.limit,
            cursor: query.cursor,
            order: query.order,
            count: query.count,
        })
    }
}
