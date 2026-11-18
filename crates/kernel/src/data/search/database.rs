//! [`DatabaseSearch`]: bộ máy tìm mặc định, chỉ mục nằm trong chính database của khách (bước 40 P7).
//!
//! - `kernel_search_docs`: một dòng mỗi tài liệu, khoá `(tenant_id, kind, id)`. Bốn cột khoá tìm như
//!   ô tìm của module (D-202): `primary_*` là trường trọng số cao nhất ("tiêu đề"), `*_key` là mọi
//!   trường có trọng số; `*_exact` / `exact_key` dạng [`exact_key`](crate::search::exact_key) (đúng
//!   dấu), `*_fold` / `fold_key` dạng [`fold`](crate::search::fold) (không dấu).
//! - `kernel_search_filters`: một dòng mỗi `(tài liệu, bộ lọc)`, index `(tenant_id, kind, name, value)`
//!   — bộ lọc bất kỳ module khai đều là cột có index, không cần đổi schema theo module.
//!
//! Xếp hạng bốn tầng (`fetch_page_tiers`): tiêu đề đúng dấu, nội dung đúng dấu, tiêu đề không dấu,
//! nội dung không dấu; trong mỗi tầng theo ID. "Chứa" là `LIKE` như mọi ô tìm: chạy như nhau trên
//! SQLite và Postgres, quét các dòng còn lại sau bộ lọc khách / loại / phạm vi.

use std::collections::BTreeMap;

use chrono::{SubsecRound, Utc};
use sea_orm::sea_query::{Expr, ExprTrait as _, Query};
use sea_orm::{ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

use super::backend::{SearchBackend, SearchDoc, SearchHit, SearchHits, SearchKind, SearchRequest};
use super::index::Kinds;
use crate::error::Error;
use crate::migration::prelude::*;
use crate::pagination::{Page, fetch_page_tiers};
use crate::tenancy::{OrgId, OrgTree, ScopeExt, TenantContext};

pub(crate) mod doc {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "kernel_search_docs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub kind: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub org_id: Uuid,
        pub locale: String,
        pub primary_exact: String,
        pub primary_fold: String,
        pub exact_key: String,
        pub fold_key: String,
        /// JSON: trường gốc (tên → giá trị), trả lại trong kết quả.
        pub fields: String,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl crate::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}

pub(crate) mod filter {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "kernel_search_filters")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub kind: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub doc_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub name: String,
        pub value: String,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl crate::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}

/// Giá trị lọc dài nhất (cột có index).
const MAX_FILTER_VALUE: usize = 200;

/// Bộ máy tìm trong database (mặc định, `AUTOXIDE_SEARCH_BACKEND=database`).
#[derive(Clone)]
pub struct DatabaseSearch {
    kinds: Kinds,
}

impl DatabaseSearch {
    pub(crate) fn new(kinds: Kinds) -> Self {
        DatabaseSearch { kinds }
    }
}

/// Giá trị lọc dạng chuỗi: chuỗi giữ nguyên, số / bool theo JSON.
pub(crate) fn filter_value(value: &serde_json::Value) -> Option<String> {
    let text = match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Number(number) => number.to_string(),
        serde_json::Value::Bool(flag) => flag.to_string(),
        _ => return None,
    };
    Some(text.chars().take(MAX_FILTER_VALUE).collect())
}

async fn delete_docs<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    kind: &str,
    ids: Option<&[Uuid]>,
) -> Result<(), Error> {
    let mut docs = doc::Entity::delete_many().filter(doc::Column::Kind.eq(kind)).scoped(tenant);
    let mut filters =
        filter::Entity::delete_many().filter(filter::Column::Kind.eq(kind)).scoped(tenant);
    if let Some(ids) = ids {
        docs = docs.filter(doc::Column::Id.is_in(ids.iter().copied()));
        filters = filters.filter(filter::Column::DocId.is_in(ids.iter().copied()));
    }
    filters.exec(connection).await?;
    docs.exec(connection).await?;
    Ok(())
}

#[async_trait::async_trait]
impl SearchBackend for DatabaseSearch {
    async fn index(&self, tenant: &TenantContext, docs: &[SearchDoc]) -> Result<(), Error> {
        if docs.is_empty() {
            return Ok(());
        }
        let now = Utc::now().trunc_subsecs(6);
        let mut rows = Vec::new();
        let mut filters = Vec::new();
        for item in docs {
            let kind = self.kinds.kind(&item.kind)?;
            let ranked = kind.ranked_fields();
            let values: Vec<&str> = ranked
                .iter()
                .filter_map(|name| item.fields.get(*name).map(String::as_str))
                .collect();
            let primary = ranked.first().and_then(|name| item.fields.get(*name)).cloned();
            let primary = primary.unwrap_or_default();
            let all = values.join("\n");
            rows.push(doc::ActiveModel {
                tenant_id: Set(tenant.id().as_uuid()),
                kind: Set(item.kind.clone()),
                id: Set(item.id),
                org_id: Set(item.org_id.as_uuid()),
                locale: Set(item.locale.clone()),
                primary_exact: Set(crate::search::exact_key(&primary)),
                primary_fold: Set(crate::search::fold(&primary)),
                exact_key: Set(crate::search::exact_key(&all)),
                fold_key: Set(crate::search::fold(&all)),
                fields: Set(serde_json::to_string(&item.fields).map_err(Error::internal)?),
                updated_at: Set(now),
            });
            for name in &kind.filters {
                let Some(value) = item.filters.get(*name).and_then(filter_value) else {
                    continue;
                };
                filters.push(filter::ActiveModel {
                    tenant_id: Set(tenant.id().as_uuid()),
                    kind: Set(item.kind.clone()),
                    doc_id: Set(item.id),
                    name: Set((*name).to_owned()),
                    value: Set(value),
                });
            }
        }
        // Thay hẳn tài liệu cũ: gọi lại (job chạy lại) cho cùng kết quả.
        let transaction = tenant.database().begin_write().await?;
        let mut by_kind: BTreeMap<&str, Vec<Uuid>> = BTreeMap::new();
        for item in docs {
            by_kind.entry(item.kind.as_str()).or_default().push(item.id);
        }
        for (kind, ids) in &by_kind {
            delete_docs(&transaction, tenant, kind, Some(ids)).await?;
        }
        doc::Entity::insert_many(rows).exec(&transaction).await?;
        if !filters.is_empty() {
            filter::Entity::insert_many(filters).exec(&transaction).await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    async fn remove(&self, tenant: &TenantContext, kind: &str, ids: &[Uuid]) -> Result<(), Error> {
        if ids.is_empty() {
            return Ok(());
        }
        let transaction = tenant.database().begin_write().await?;
        delete_docs(&transaction, tenant, kind, Some(ids)).await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn clear(&self, tenant: &TenantContext, kind: &str) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        delete_docs(&transaction, tenant, kind, None).await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn query(
        &self,
        tenant: &TenantContext,
        kind: &SearchKind,
        request: &SearchRequest,
    ) -> Result<SearchHits, Error> {
        let mut select = doc::Entity::find().filter(doc::Column::Kind.eq(kind.kind)).scoped(tenant);
        if let Some(locale) = &request.locale {
            select = select.filter(doc::Column::Locale.eq(locale.as_str()));
        }
        for (name, value) in &request.filters {
            let matching = Query::select()
                .expr(Expr::val(1))
                .from(filter::Entity)
                .and_where(
                    Expr::col((filter::Entity, filter::Column::TenantId))
                        .equals((doc::Entity, doc::Column::TenantId)),
                )
                .and_where(
                    Expr::col((filter::Entity, filter::Column::Kind))
                        .equals((doc::Entity, doc::Column::Kind)),
                )
                .and_where(
                    Expr::col((filter::Entity, filter::Column::DocId))
                        .equals((doc::Entity, doc::Column::Id)),
                )
                .and_where(Expr::col((filter::Entity, filter::Column::Name)).eq(name.as_str()))
                .and_where(Expr::col((filter::Entity, filter::Column::Value)).eq(value.as_str()))
                .to_owned();
            select = select.filter(Expr::exists(matching));
        }
        if let Some(scope) = &request.scope {
            if scope.subtrees.is_empty() && scope.units.is_empty() {
                let pagination = crate::pagination::Pagination::new(request.page.limit(), None)
                    .with_total(request.page.wants_total().then_some(0));
                return Ok(Page { data: Vec::new(), pagination });
            }
            let mut any = Condition::any();
            if !scope.subtrees.is_empty() {
                any = any.add(OrgTree::subtree_condition(
                    tenant.id(),
                    &scope.subtrees,
                    doc::Column::OrgId,
                ));
            }
            if !scope.units.is_empty() {
                any = any.add(doc::Column::OrgId.is_in(scope.units.iter().map(OrgId::as_uuid)));
            }
            select = select.filter(any);
            if let Some(fence) = scope.fence {
                select = select.filter(OrgTree::subtree_condition(
                    tenant.id(),
                    &[fence],
                    doc::Column::OrgId,
                ));
            }
        }
        let text = request.text.trim();
        let tiers: Vec<Condition> = if text.is_empty() {
            vec![Condition::all()]
        } else {
            vec![
                Condition::all()
                    .add(crate::search::contains_exact(doc::Column::PrimaryExact, text)),
                Condition::all().add(crate::search::contains_exact(doc::Column::ExactKey, text)),
                Condition::all().add(crate::search::contains(doc::Column::PrimaryFold, text)),
                Condition::all().add(crate::search::contains(doc::Column::FoldKey, text)),
            ]
        };
        let read = tenant.database().read();
        let page =
            fetch_page_tiers(select, &tiers, doc::Column::Id, |row| row.id, &request.page, read)
                .await?;
        let hits = page.data.into_iter().map(|row| SearchHit {
            fields: serde_json::from_str(&row.fields).unwrap_or_default(),
            kind: row.kind,
            id: row.id,
            org_id: OrgId::from_uuid(row.org_id),
            locale: row.locale,
        });
        Ok(Page { data: hits.collect(), pagination: page.pagination })
    }
}

/// Migration tạo `kernel_search_docs` và `kernel_search_filters` (cấp khách hàng, mọi database dữ liệu).
pub(crate) struct CreateSearchDocs;

impl MigrationName for CreateSearchDocs {
    fn name(&self) -> &str {
        "m0034_create_search_docs"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateSearchDocs {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("kernel_search_docs")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("kind", 64))
                    .col(schema::uuid("id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("locale", 16))
                    .col(schema::text("primary_exact"))
                    .col(schema::text("primary_fold"))
                    .col(schema::text("exact_key"))
                    .col(schema::text("fold_key"))
                    .col(schema::text("fields"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("tenant_id").col("kind").col("id"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("kernel_search_docs_org_idx")
                    .table("kernel_search_docs")
                    .col("tenant_id")
                    .col("kind")
                    .col("org_id")
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("kernel_search_filters")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("kind", 64))
                    .col(schema::uuid("doc_id"))
                    .col(schema::string_len("name", 64))
                    .col(schema::string_len("value", 200))
                    .primary_key(
                        Index::create().col("tenant_id").col("kind").col("doc_id").col("name"),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("kernel_search_filters_value_idx")
                    .table("kernel_search_filters")
                    .col("tenant_id")
                    .col("kind")
                    .col("name")
                    .col("value")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("kernel_search_filters").to_owned()).await?;
        manager.drop_table(Table::drop().table("kernel_search_docs").to_owned()).await
    }
}
