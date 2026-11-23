//! Đọc công khai mục của loại tự định nghĩa (D-235): chỉ loại đang bật và `listable`, chỉ bản đã đăng
//! của website đang xem, mới nhất trước. Lọc `cf.<key>` chỉ trên trường `public` + `filterable`, theo
//! giá trị của BẢN ĐĂNG (bảng index `content_published_custom_index`, ghi lúc đăng) — không bao giờ
//! theo giá trị nháp (D-20261123-content-types-later).

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::pagination::{Page, PageRequest, SortValue, fetch_page_by};
use autoxide_kernel::tenancy::{ScopeExt, Site, TenantContext};
use sea_orm::sea_query::Alias;
use sea_orm::sea_query::{Expr, ExprTrait, Query};
use sea_orm::{ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter};
use serde_json::{Map, Value};
use uuid::Uuid;

use super::{ContentType, field_target, type_not_found};
use crate::entity::{content_type, published};
use crate::publishing::Published;
use crate::publishing::reading::on_site;
use crate::service::ContentService;

/// Mục `kind = custom` chỉ công khai khi loại của nó đang bật (`subquery`, như nhau trên hai database).
pub(crate) fn type_is_active(tenant: &TenantContext) -> Condition {
    let active = Query::select()
        .column(content_type::Column::Id)
        .from(content_type::Entity)
        .and_where(Expr::col(content_type::Column::TenantId).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(content_type::Column::Active).eq(true))
        .to_owned();
    Condition::any()
        .add(published::Column::TypeId.is_null())
        .add(published::Column::TypeId.in_subquery(active))
}

impl ContentService {
    /// Mục đã đăng của loại `key` ở ngôn ngữ `locale` trên website `site`. Loại không có, đang tắt
    /// hay không `listable` → 404 `content.type_not_found` (không lộ loại tồn tại).
    pub(crate) async fn public_type_entries(
        &self,
        tenant: &TenantContext,
        site: &Site,
        key: &str,
        locale: &str,
        filters: &[(String, String)],
        page: &PageRequest,
    ) -> Result<(ContentType, Page<Published>), Error> {
        let found = self.find_type(tenant, key).await.ok().filter(|row| row.active && row.listable);
        let Some(row) = found else { return Err(type_not_found(key)) };
        let mut select = published::Entity::find()
            .scoped(tenant)
            .filter(published::Column::RestrictedAt.is_null())
            .filter(published::Column::TypeId.eq(row.id))
            .filter(published::Column::Locale.eq(locale))
            .filter(on_site(&site.lineage_ids()));
        if !filters.is_empty() {
            select = select.filter(self.public_filters(tenant, &row, filters).await?);
        }
        let sort = |entry: &Published| SortValue::Time(entry.published_at);
        let read = tenant.database().read();
        let page = fetch_page_by(
            select,
            published::Column::PublishedAt,
            sort,
            published::Column::EntryId,
            |entry| entry.entry_id,
            page,
            read,
        )
        .await?;
        Ok((row, page))
    }
}

/// Bảng index lọc theo giá trị của bản đăng (cùng dạng bảng index của kernel).
pub(crate) const PUBLISHED_INDEX: &str = "content_published";

impl ContentService {
    /// Điều kiện của các bộ lọc `cf.<key>` (`filters`: khoá, `<toán tử>:<giá trị>`) trên danh sách
    /// công khai của loại `row`: chỉ trường `public` + `filterable`; mọi lỗi một lần.
    pub(crate) async fn public_filters(
        &self,
        tenant: &TenantContext,
        row: &ContentType,
        filters: &[(String, String)],
    ) -> Result<Condition, Error> {
        let target = field_target(&row.key);
        let public = self.custom_fields.public_keys(tenant, &target).await?;
        let mut condition = Condition::all();
        let mut problems = Vec::new();
        for (key, value) in filters {
            if !public.contains(key) {
                // Trường nội bộ trả như trường không có: không lộ tên trường nội bộ.
                problems.push(FieldError::new(
                    format!("cf.{key}"),
                    "unknown",
                    "is not a public field of this content type",
                ));
                continue;
            }
            match self
                .custom_fields
                .condition(tenant, PUBLISHED_INDEX, &target, published::Column::EntryId, key, value)
                .await
            {
                Ok(found) => condition = condition.add(found),
                Err(error) if !error.fields().is_empty() => {
                    problems.extend(error.fields().iter().cloned())
                }
                Err(error) => return Err(error),
            }
        }
        if problems.is_empty() { Ok(condition) } else { Err(Error::validation(problems)) }
    }

    /// Ghi index lọc của bản đăng của mục `entry_id` (loại `type_id`) theo giá trị trường đã đăng
    /// `stored` — cùng transaction với bản đăng.
    pub(crate) async fn write_published_index<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        type_id: Uuid,
        entry_id: Uuid,
        stored: Option<&str>,
    ) -> Result<(), Error> {
        let Some(row) =
            content_type::Entity::find_by_id(type_id).scoped(tenant).one(connection).await?
        else {
            return remove_published_index(connection, tenant, entry_id).await;
        };
        let values: Map<String, Value> =
            stored.and_then(|text| serde_json::from_str(text).ok()).unwrap_or_default();
        self.custom_fields
            .write_index(
                connection,
                tenant,
                PUBLISHED_INDEX,
                &field_target(&row.key),
                entry_id,
                &values,
            )
            .await
    }
}

/// Bỏ index lọc của bản đăng của mục `entry_id` (gỡ, xoá mục).
pub(crate) async fn remove_published_index<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    entry_id: Uuid,
) -> Result<(), Error> {
    let delete = Query::delete()
        .from_table(Alias::new(format!("{PUBLISHED_INDEX}_custom_index")))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(Alias::new("record_id")).eq(entry_id))
        .to_owned();
    connection.execute(&delete).await?;
    Ok(())
}
