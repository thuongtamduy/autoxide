//! Khu dùng chung nhiều trang (bước 67, D-229, D-20261122-shared-sections): một bố cục khối (vd dải
//! khuyến mại cuối trang) đặt vào nhiều trang bằng khối `content.shared_section {id}`. Lúc đọc công
//! khai, khối được thay bằng nội dung hiện tại của khu (một câu truy vấn cho mọi khu của trang) — sửa
//! khu là mọi trang đổi theo, không phải đăng lại trang. Khu không chứa khu khác (không lồng, không
//! vòng). Xoá khu còn trang dùng → 409 kèm số trang.

pub(crate) mod public;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, root_of};
use autoxide_kernel::time::Timestamp;
use sea_orm::sea_query::Condition;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, Set,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{entry, published, shared_section};
use crate::entries::layout;
use crate::events::{self, Change};
use crate::permissions::{CONTENT_READ, CONTENT_WRITE};
use crate::service::{ContentService, now};

/// Loại của khu trong tập bản ghi trích từ bố cục (kiểm cùng chỗ với bản ghi nhúng).
pub(crate) const KIND: &str = "content.shared_section";
const NAME_MAX: usize = 120;

pub(crate) type SharedSection = shared_section::Model;

/// Dữ liệu tạo / thay một khu.
pub(crate) struct SectionInput {
    pub name: String,
    pub layout: Value,
    pub org_id: Option<Uuid>,
}

pub(crate) fn shared_section_not_found(id: Uuid) -> Error {
    Error::not_found("content.shared_section_not_found", "There is no shared section {id}.")
        .with_param("id", id)
}

fn target(row: &SharedSection) -> OrgTarget {
    OrgTarget::new(OrgId::from_uuid(row.org_id), row.created_by)
}

impl ContentService {
    /// Một trang khu dùng chung trong phạm vi `content.read` của người gọi.
    pub(crate) async fn list_shared_sections(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<SharedSection>, Error> {
        principal.require(&CONTENT_READ)?;
        let scope =
            principal.scope_condition(&CONTENT_READ, shared_section::Column::OrgId, None)?;
        let select = shared_section::Entity::find().filter(scope).scoped(tenant);
        let read = tenant.database().read();
        fetch_page(select, shared_section::Column::Id, |row| row.id, page, read).await
    }

    /// Khu `id`. Cần `content.read` ở nút của khu.
    pub(crate) async fn shared_section(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<SharedSection, Error> {
        principal.require(&CONTENT_READ)?;
        let read = tenant.database().read();
        let row = find(read, tenant, id).await?;
        hidden(principal.require_at(read, &CONTENT_READ, target(&row)).await, id)?;
        Ok(row)
    }

    /// Tạo khu. Cần `content.write` ở nút của khu (mặc định: gốc).
    pub(crate) async fn create_shared_section(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: SectionInput,
    ) -> Result<SharedSection, Error> {
        principal.require(&CONTENT_WRITE)?;
        let org = input.org_id.map_or(root_of(tenant.id()), OrgId::from_uuid);
        let (name, text) = self.check_section(tenant, &input).await?;
        let at = now();
        let transaction = tenant.database().begin_write().await?;
        principal.require_write_at(&transaction, &CONTENT_WRITE, OrgTarget::new(org, None)).await?;
        let row = shared_section::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(org.as_uuid()),
            name: Set(name),
            layout: Set(text),
            created_by: Set(Some(principal.user_id())),
            created_at: Set(at),
            updated_at: Set(at),
        }
        .insert(&transaction)
        .await?;
        self.record_section(&transaction, tenant, principal, None, Some(&row), 0).await?;
        transaction.commit().await?;
        Ok(row)
    }

    /// Thay tên và bố cục của khu `id`. `expected`: phiên bản đã đọc. Mọi trang dùng khu đổi theo.
    pub(crate) async fn put_shared_section(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        input: SectionInput,
        expected: Option<Timestamp>,
    ) -> Result<SharedSection, Error> {
        principal.require(&CONTENT_WRITE)?;
        let (name, text) = self.check_section(tenant, &input).await?;
        let at = now();
        let transaction = tenant.database().begin_write().await?;
        let before = find(&transaction, tenant, id).await?;
        hidden(principal.require_at(&transaction, &CONTENT_READ, target(&before)).await, id)?;
        principal.require_write_at(&transaction, &CONTENT_WRITE, target(&before)).await?;
        if expected.is_some_and(|expected| expected != before.updated_at) {
            return Err(autoxide_kernel::version::changed());
        }
        let mut active: shared_section::ActiveModel = before.clone().into();
        active.name = Set(name);
        active.layout = Set(text);
        active.updated_at = Set(at);
        let after = active.update(&transaction).await?;
        let pages = self.pages_using(&transaction, tenant, id).await?;
        self.record_section(&transaction, tenant, principal, Some(&before), Some(&after), pages)
            .await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Xoá khu `id`; còn trang (bản nháp hay bản đăng) dùng → 409 `content.shared_section_in_use`.
    pub(crate) async fn delete_shared_section(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        expected: Option<Timestamp>,
    ) -> Result<(), Error> {
        principal.require(&CONTENT_WRITE)?;
        let transaction = tenant.database().begin_write().await?;
        let before = find(&transaction, tenant, id).await?;
        hidden(principal.require_at(&transaction, &CONTENT_READ, target(&before)).await, id)?;
        principal.require_write_at(&transaction, &CONTENT_WRITE, target(&before)).await?;
        if expected.is_some_and(|expected| expected != before.updated_at) {
            return Err(autoxide_kernel::version::changed());
        }
        let pages = self.pages_using(&transaction, tenant, id).await?;
        if pages > 0 {
            return Err(Error::conflict(
                "content.shared_section_in_use",
                "{count} pages use this shared section; remove it from them first.",
            )
            .with_param("count", pages));
        }
        shared_section::Entity::delete_many()
            .scoped(tenant)
            .filter(shared_section::Column::Id.eq(id))
            .exec(&transaction)
            .await?;
        self.record_section(&transaction, tenant, principal, Some(&before), None, 0).await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Kiểm tên và bố cục của khu (như thân `layout` của trang; khu không chứa khu). Mọi lỗi một
    /// lần. Trả tên đã cắt khoảng trắng và bố cục dạng chữ.
    async fn check_section(
        &self,
        tenant: &TenantContext,
        input: &SectionInput,
    ) -> Result<(String, String), Error> {
        let mut problems = Vec::new();
        let name = input.name.trim().to_owned();
        if name.is_empty() || name.chars().count() > NAME_MAX {
            problems.push(
                FieldError::new("name", "length", "must be 1 to {max} characters")
                    .with_param("max", NAME_MAX),
            );
        }
        if !layout::is_layout(&input.layout) {
            problems.push(FieldError::new("layout.type", "invalid", "must be `layout`"));
            return Err(Error::validation(problems));
        }
        let info = self.check_body(tenant, "page", &input.layout, &mut problems).await?;
        // Quét thẳng bố cục: báo cả khi bố cục còn lỗi khác (mọi lỗi một lần).
        if layout::blocks_of(&input.layout).iter().any(|(block, _, _)| *block == KIND) {
            problems.push(FieldError::new(
                "layout",
                "nested",
                "a shared section cannot hold another shared section",
            ));
        }
        let media = info.media_ids.iter().map(|id| ("layout".to_owned(), *id)).collect();
        self.check_media_ids(tenant, media, false, &mut problems).await?;
        self.check_records(tenant, &info.records, &mut problems).await?;
        // Lỗi của bộ kiểm bố cục mang tiền tố `body`: khu gọi trường là `layout`.
        for problem in &mut problems {
            if let Some(rest) = problem.field.strip_prefix("body") {
                problem.field = format!("layout{rest}");
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let text = serde_json::to_string(&input.layout).map_err(Error::internal)?;
        Ok((name, text))
    }

    /// Khu `ids` (chuỗi) có thật trong khách không; khu thiếu vào `problems`.
    pub(crate) async fn check_shared_ids(
        &self,
        tenant: &TenantContext,
        ids: &[String],
        problems: &mut Vec<FieldError>,
    ) -> Result<(), Error> {
        let parsed: Vec<Uuid> = ids.iter().filter_map(|id| Uuid::parse_str(id).ok()).collect();
        let found: Vec<Uuid> = if parsed.is_empty() {
            Vec::new()
        } else {
            shared_section::Entity::find()
                .scoped(tenant)
                .filter(shared_section::Column::Id.is_in(parsed))
                .all(tenant.database().read())
                .await?
                .into_iter()
                .map(|row| row.id)
                .collect()
        };
        for id in ids {
            if !Uuid::parse_str(id).is_ok_and(|id| found.contains(&id)) {
                problems.push(
                    FieldError::new(
                        "body",
                        "shared_section_not_found",
                        "there is no shared section {id}",
                    )
                    .with_param("id", id),
                );
            }
        }
        Ok(())
    }

    /// Số trang (bản nháp hay bản đăng) có khối dùng khu `id`. Quét chữ trong thân trang của khách:
    /// chỉ chạy khi sửa hay xoá khu (hiếm), không ở đường đọc công khai (D-20261122-shared-sections).
    async fn pages_using<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<u64, Error> {
        let needle = id.to_string();
        let drafts = entry::Entity::find()
            .scoped(tenant)
            .filter(entry::Column::Kind.eq("page"))
            .filter(entry::Column::Body.contains(&needle))
            .count(connection)
            .await?;
        // Trang đã đăng mà bản nháp đã bỏ khu vẫn đang hiện khu: đếm cả bản đăng, không đếm trùng.
        let published = published::Entity::find()
            .scoped(tenant)
            .filter(published::Column::Kind.eq("page"))
            .filter(published::Column::Body.contains(&needle))
            .filter(
                Condition::all().add(
                    published::Column::EntryId.not_in_subquery(
                        sea_orm::sea_query::Query::select()
                            .column(entry::Column::Id)
                            .from(entry::Entity)
                            .and_where(entry::Column::TenantId.eq(tenant.id().as_uuid()))
                            .and_where(entry::Column::Kind.eq("page"))
                            .and_where(entry::Column::Body.contains(&needle))
                            .to_owned(),
                    ),
                ),
            )
            .count(connection)
            .await?;
        Ok(drafts + published)
    }

    async fn record_section<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        principal: &Principal,
        before: Option<&SharedSection>,
        after: Option<&SharedSection>,
        pages: u64,
    ) -> Result<(), Error> {
        let view =
            |row: &SharedSection| json!({"id": row.id, "name": row.name, "org_id": row.org_id});
        let id = after.or(before).map_or(Uuid::nil(), |row| row.id);
        let mut after_view = after.map(view);
        if let Some(Value::Object(fields)) = after_view.as_mut() {
            fields.insert("pages".to_owned(), json!(pages));
        }
        let event = events::SharedSectionChanged(Change {
            before: before.map(view),
            after: after_view,
            published: None,
        });
        self.outbox
            .record(
                connection,
                tenant,
                Actor::from(principal),
                events::subject("shared_section", id),
                &event,
            )
            .await?;
        Ok(())
    }
}

async fn find<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<SharedSection, Error> {
    shared_section::Entity::find_by_id(id)
        .scoped(tenant)
        .one(connection)
        .await?
        .ok_or_else(|| shared_section_not_found(id))
}

/// Ngoài phạm vi đọc: như không có (không lộ khu tồn tại).
fn hidden(result: Result<(), Error>, id: Uuid) -> Result<(), Error> {
    result.map_err(|error| {
        if error.code() == "org_out_of_scope" { shared_section_not_found(id) } else { error }
    })
}
