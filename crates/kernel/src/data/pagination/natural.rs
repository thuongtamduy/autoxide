//! Query parameters of a list in a natural order (step 70 P9, D-20261125-list-pages): a position, a name, a
//! code — not creation time. The same `limit` / `cursor` / `order` / `count` as [`PageQuery`], but `order`
//! defaults to `asc` (the natural order); `desc` reverses it. Rule: a list whose only order is creation time
//! takes [`PageQuery`] (newest first); a list with a natural order takes this.

use serde::Deserialize;

use super::{Order, PageQuery, PageRequest};
use crate::error::Error;

// Bọc `PageQuery` (`transparent`: đọc từ query đúng như `PageQuery`) để bốn tham số chỉ khai một chỗ; trường
// riêng tư nên chỉ có một cách ra `PageRequest` — `TryFrom` với mặc định `asc`.
/// Query parameters of a list in a natural order (position, name, code).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct NaturalOrderPageQuery(PageQuery);

impl NaturalOrderPageQuery {
    /// The query parameters exactly as a client sends them (all text, each optional).
    pub const fn new(limit: Option<String>, cursor: Option<String>, order: Option<String>) -> Self {
        NaturalOrderPageQuery(PageQuery::new(limit, cursor, order))
    }

    /// Like [`NaturalOrderPageQuery::new`], also asking for the total (`count=true`).
    #[must_use]
    pub fn with_count(self, count: bool) -> Self {
        NaturalOrderPageQuery(self.0.with_count(count))
    }
}

// `TryFrom`: như `PageQuery`, tham số sai thành lỗi 422 có tên từng tham số.
impl TryFrom<NaturalOrderPageQuery> for PageRequest {
    type Error = Error;

    fn try_from(query: NaturalOrderPageQuery) -> Result<Self, Self::Error> {
        PageRequest::parse(query.0, Order::Asc)
    }
}
