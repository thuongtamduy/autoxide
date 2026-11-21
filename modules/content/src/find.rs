//! Nguồn tìm xuyên phân hệ (bước 51 P3): bài viết và trang có tiêu đề, tóm tắt hay thân chứa từ
//! tìm (cột `search_key`, không dấu), trong phạm vi của `content.read` của người gọi. Hiểu bộ lọc
//! (P9): `status:`, `kind:`, `locale:` (bằng, có thể phủ định), `updated` / `created` (ngày
//! `YYYY-MM-DD` hay RFC 3339, so `>` `<` `>=` `<=` hay khoảng `a..b`).

use autoxide_kernel::access::{OrgTarget, Permission, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::find::{FinderSource, Found, SearchFilter, SearchOp, SearchQuery};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::time::{Date, Timestamp};
use jiff::SignedDuration;
use sea_orm::sea_query::{Condition, SimpleExpr};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use crate::entity::entry::{Column, Entity};
use crate::permissions::CONTENT_READ;

pub(crate) struct Entries;

#[async_trait::async_trait]
impl FinderSource for Entries {
    fn kind(&self) -> &'static str {
        "content.entry"
    }

    fn permission(&self) -> Permission {
        CONTENT_READ
    }

    async fn find(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        target: OrgTarget,
        query: &str,
        limit: u16,
    ) -> Result<Vec<Found>, Error> {
        let query = SearchQuery { text: query.to_owned(), ..SearchQuery::default() };
        self.find_query(tenant, principal, target, &query, limit).await
    }

    fn scoped(&self) -> bool {
        true
    }

    fn fields(&self) -> &'static [&'static str] {
        &["status", "kind", "locale", "updated", "created"]
    }

    async fn find_query(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        _: OrgTarget,
        query: &SearchQuery,
        limit: u16,
    ) -> Result<Vec<Found>, Error> {
        let scope =
            principal.scope_condition(&CONTENT_READ, Column::OrgId, Some(Column::CreatedBy))?;
        let mut select = Entity::find().filter(scope).scoped(tenant);
        if !query.text.is_empty() {
            select =
                select.filter(autoxide_kernel::search::contains(Column::SearchKey, &query.text));
        }
        if let Some(scope) = query.scope_condition(tenant, Column::OrgId) {
            select = select.filter(scope);
        }
        for filter in &query.filters {
            if let Some(condition) = condition(filter) {
                select = select.filter(condition);
            }
        }
        let rows = select
            .order_by_desc(Column::UpdatedAt)
            .limit(u64::from(limit))
            .all(tenant.database().read())
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                Found::new(row.id.to_string(), row.title, "content.entries")
                    .subtitle(format!("{} · {} · {}", row.kind, row.status, row.locale))
            })
            .collect())
    }
}

/// Điều kiện SQL của một bộ lọc; trường lạ hay giá trị sai thì `None` (bỏ qua).
fn condition(filter: &SearchFilter) -> Option<Condition> {
    let text = |column: Column| -> Option<SimpleExpr> {
        match &filter.op {
            SearchOp::Eq(value) => Some(column.eq(value.to_lowercase())),
            _ => None,
        }
    };
    let time = |column: Column| -> Option<Condition> {
        let at = |value: &str| instant(value);
        Some(Condition::all().add(match &filter.op {
            SearchOp::Eq(value) => {
                let from = at(value)?;
                return Some(
                    Condition::all()
                        .add(column.gte(from))
                        .add(column.lt(from + SignedDuration::from_hours(24))),
                );
            }
            SearchOp::Gt(value) => column.gt(at(value)?),
            SearchOp::Ge(value) => column.gte(at(value)?),
            SearchOp::Lt(value) => column.lt(at(value)?),
            SearchOp::Le(value) => column.lte(at(value)?),
            SearchOp::Range(from, to) => {
                let mut both = Condition::all();
                if !from.is_empty() {
                    both = both.add(column.gte(at(from)?));
                }
                if !to.is_empty() {
                    both = both.add(column.lte(at(to)?));
                }
                return Some(both);
            }
        }))
    };
    let condition = match filter.field.as_str() {
        "status" => Condition::all().add(text(Column::Status)?),
        "kind" => Condition::all().add(text(Column::Kind)?),
        "locale" => Condition::all().add(text(Column::Locale)?),
        "updated" => time(Column::UpdatedAt)?,
        "created" => time(Column::CreatedAt)?,
        _ => return None,
    };
    Some(if filter.negated { condition.not() } else { condition })
}

/// `YYYY-MM-DD` (đầu ngày, UTC) hay RFC 3339.
fn instant(value: &str) -> Option<Timestamp> {
    Timestamp::parse_rfc3339(value)
        .ok()
        .or_else(|| Date::parse_from_str(value, "%Y-%m-%d").ok().map(Date::start_of_day_utc))
}

/// Mở bài viết / trang theo đường dẫn (bước 51 P9): mã là slug hay đường dẫn công khai
/// (`/blog/duong-den-da-lat`, phần cuối là slug), trong phạm vi `content.read` của người gọi.
pub(crate) struct BySlug;

#[async_trait::async_trait]
impl autoxide_kernel::resolve::CodeResolver for BySlug {
    fn kind(&self) -> &'static str {
        "content.entry"
    }

    fn permission(&self) -> Permission {
        CONTENT_READ
    }

    async fn resolve(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        code: &str,
    ) -> Result<Option<Found>, Error> {
        let slug = code.trim_end_matches('/').rsplit('/').next().unwrap_or(code).to_lowercase();
        if slug.is_empty() {
            return Ok(None);
        }
        let scope =
            principal.scope_condition(&CONTENT_READ, Column::OrgId, Some(Column::CreatedBy))?;
        let row = Entity::find()
            .filter(scope)
            .scoped(tenant)
            .filter(Column::Slug.eq(slug))
            .order_by_desc(Column::UpdatedAt)
            .one(tenant.database().read())
            .await?;
        Ok(row.map(|row| {
            Found::new(row.id.to_string(), row.title, "content.entries")
                .subtitle(format!("{} · {} · {}", row.kind, row.status, row.locale))
        }))
    }
}
