//! Trang của cây danh mục (bước 70 P9 mục 3, D-20261125-list-pages): quản trị và công khai cùng thứ tự độ sâu,
//! vị trí, id — cha luôn đứng trước con. Keyset trên ba cột ngay trong database (index
//! `catalog_categories_tree_page_idx` (tenant_id, depth, position, id)); con trỏ mang `<độ sâu>\u{1f}<vị trí>\u{1f}<id>`
//! (`text_key_cursor`, vì `fetch_page_by` chỉ sắp theo một cột).

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::pagination::{
    Order, Page, PageRequest, Pagination, text_key_cursor, total_of,
};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::{Expr, ExprTrait as _, Query};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    Select,
};
use uuid::Uuid;

use crate::categories::CategoryDetail;
use crate::entity::{category, category_path, category_translation};
use crate::public::PublicCategory;
use crate::service::CatalogService;

/// Tên thứ tự trong con trỏ của cây danh mục.
const TREE_CURSOR: &str = "category:tree";

impl CatalogService {
    /// Một trang danh mục của khách (mọi danh mục, ẩn cũng có), kèm tên theo mọi ngôn ngữ.
    pub(crate) async fn list_categories_page(
        &self,
        tenant: &TenantContext,
        page: &PageRequest,
    ) -> Result<Page<CategoryDetail>, Error> {
        let read = tenant.database().read();
        let rows = tree_page(category::Entity::find().scoped(tenant), page, read).await?;
        let ids: Vec<Uuid> = rows.data.iter().map(|row| row.id).collect();
        let texts = category_translation::Entity::find()
            .filter(category_translation::Column::CategoryId.is_in(ids))
            .scoped(tenant)
            .order_by_asc(category_translation::Column::Locale)
            .all(read)
            .await?;
        Ok(rows.map(|category| {
            let translations =
                texts.iter().filter(|text| text.category_id == category.id).cloned().collect();
            CategoryDetail { category, translations }
        }))
    }

    /// Một trang cây danh mục công khai: danh mục hiện, có tên bằng `locale`, và mọi tổ tiên cũng hiện và có
    /// tên bằng `locale` (danh mục dưới một danh mục bị ẩn thì vắng) — cùng kết quả với
    /// [`CatalogService::public_categories`], nhưng điều kiện viết trong SQL (bảng đường đi `catalog_category_paths`)
    /// để phân trang trong database.
    pub(crate) async fn public_categories_page(
        &self,
        tenant: &TenantContext,
        locale: &str,
        page: &PageRequest,
    ) -> Result<Page<PublicCategory>, Error> {
        let read = tenant.database().read();
        let tenant_id = tenant.id().as_uuid();
        let named = Query::select()
            .column(category_translation::Column::CategoryId)
            .from(category_translation::Entity)
            .and_where(Expr::col(category_translation::Column::TenantId).eq(tenant_id))
            .and_where(Expr::col(category_translation::Column::Locale).eq(locale))
            .to_owned();
        let hidden = Query::select()
            .column(category::Column::Id)
            .from(category::Entity)
            .and_where(Expr::col(category::Column::TenantId).eq(tenant_id))
            .and_where(Expr::col(category::Column::Visible).eq(false))
            .to_owned();
        // Danh mục có một tổ tiên (không kể chính nó) bị ẩn hay không có tên bằng `locale`.
        let under_hidden = Query::select()
            .column(category_path::Column::DescendantId)
            .from(category_path::Entity)
            .and_where(Expr::col(category_path::Column::TenantId).eq(tenant_id))
            .and_where(Expr::col(category_path::Column::Depth).gt(0))
            .cond_where(
                Condition::any()
                    .add(Expr::col(category_path::Column::AncestorId).in_subquery(hidden))
                    .add(
                        Expr::col(category_path::Column::AncestorId).not_in_subquery(named.clone()),
                    ),
            )
            .to_owned();
        let select = category::Entity::find()
            .filter(category::Column::Visible.eq(true))
            .filter(category::Column::Id.in_subquery(named))
            .filter(category::Column::Id.not_in_subquery(under_hidden))
            .scoped(tenant);
        let rows = tree_page(select, page, read).await?;
        let ids: Vec<Uuid> = rows.data.iter().map(|row| row.id).collect();
        let texts = category_translation::Entity::find()
            .filter(category_translation::Column::CategoryId.is_in(ids))
            .filter(category_translation::Column::Locale.eq(locale))
            .scoped(tenant)
            .all(read)
            .await?;
        let pagination = rows.pagination;
        let data = rows
            .data
            .into_iter()
            .filter_map(|category| {
                let text = texts.iter().find(|text| text.category_id == category.id)?.clone();
                Some(PublicCategory { category, text })
            })
            .collect();
        Ok(Page { data, pagination })
    }
}

/// Một trang của `select` theo (độ sâu, vị trí, id); `order=desc` đảo cả ba.
async fn tree_page<C: ConnectionTrait>(
    select: Select<category::Entity>,
    page: &PageRequest,
    connection: &C,
) -> Result<Page<category::Model>, Error> {
    let after = page.text_key_after(TREE_CURSOR)?.map(|key| parse_key(&key)).transpose()?;
    let total = total_of(&select, page, connection).await?;
    let ascending = page.order() == Order::Asc;
    let mut select = select;
    if let Some((depth, position, id)) = after {
        // Cận `depth >= d` (giảm dần: `<=`) cho database nhảy thẳng tới điểm trong index, rồi điều kiện
        // "sau (d, p, id)" đầy đủ.
        let (bound, deeper, later, tie) = if ascending {
            (
                category::Column::Depth.gte(depth),
                category::Column::Depth.gt(depth),
                category::Column::Position.gt(position),
                category::Column::Id.gt(id),
            )
        } else {
            (
                category::Column::Depth.lte(depth),
                category::Column::Depth.lt(depth),
                category::Column::Position.lt(position),
                category::Column::Id.lt(id),
            )
        };
        select = select.filter(bound).filter(
            Condition::any()
                .add(deeper)
                .add(Condition::all().add(category::Column::Depth.eq(depth)).add(later))
                .add(
                    Condition::all()
                        .add(category::Column::Depth.eq(depth))
                        .add(category::Column::Position.eq(position))
                        .add(tie),
                ),
        );
    }
    let select = if ascending {
        select
            .order_by_asc(category::Column::Depth)
            .order_by_asc(category::Column::Position)
            .order_by_asc(category::Column::Id)
    } else {
        select
            .order_by_desc(category::Column::Depth)
            .order_by_desc(category::Column::Position)
            .order_by_desc(category::Column::Id)
    };
    let mut rows = select.limit(u64::from(page.limit()) + 1).all(connection).await?;
    let has_more = rows.len() > page.limit() as usize;
    rows.truncate(page.limit() as usize);
    let next = has_more.then(|| rows.last()).flatten().map(|row| {
        text_key_cursor(
            TREE_CURSOR,
            &format!("{}\u{1f}{}\u{1f}{}", row.depth, row.position, row.id),
        )
    });
    Ok(Page { data: rows, pagination: Pagination::new(page.limit(), next).with_total(total) })
}

/// Khoá con trỏ `<độ sâu>\u{1f}<vị trí>\u{1f}<id>`; hỏng thì 422 ở `cursor`.
fn parse_key(key: &str) -> Result<(i32, i32, Uuid), Error> {
    let mut parts = key.split('\u{1f}');
    let parsed = (|| {
        let depth = parts.next()?.parse::<i32>().ok()?;
        let position = parts.next()?.parse::<i32>().ok()?;
        let id = Uuid::parse_str(parts.next()?).ok()?;
        parts.next().is_none().then_some((depth, position, id))
    })();
    parsed.ok_or_else(|| {
        Error::validation(vec![FieldError::new(
            "cursor",
            "invalid",
            "must be the `next_cursor` value of a previous page of the same list and sort, unchanged",
        )])
    })
}
