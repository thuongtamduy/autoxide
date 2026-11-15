//! Đọc audit log: danh sách có lọc và phân trang, và từng dòng.

use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::entity::{self, Column, Entity};

/// Điều kiện lọc; trường `None` không lọc.
#[derive(Debug, Clone, Default)]
pub(crate) struct Filter {
    pub actor_user_id: Option<Uuid>,
    pub subject: Option<String>,
    pub action: Option<String>,
    pub module: Option<String>,
}

pub(crate) struct AuditService {
    database: Database,
    /// Sổ đối tượng của kernel: tên người làm (`auth.user`) không cần biết module `auth`.
    resources: autoxide_kernel::resource::ResourceDirectory,
}

/// Loại đối tượng "tài khoản người dùng" trong sổ đối tượng (module `auth` đăng ký; không có
/// thì không có tên).
const USER_KIND: &str = "auth.user";

impl AuditService {
    pub(crate) fn new(
        database: Database,
        resources: autoxide_kernel::resource::ResourceDirectory,
    ) -> Self {
        AuditService { database, resources }
    }

    /// Tên hiển thị của những người `ids`, theo lô (một lần tra cho cả trang). Chương trình không
    /// có module tài khoản, hay người đã bị xoá: vắng trong kết quả.
    pub(crate) async fn actor_names(
        &self,
        tenant: &autoxide_kernel::tenancy::TenantContext,
        ids: &[uuid::Uuid],
    ) -> Result<std::collections::HashMap<uuid::Uuid, String>, Error> {
        if ids.is_empty() || !self.resources.knows(USER_KIND) {
            return Ok(std::collections::HashMap::new());
        }
        let keys: Vec<String> = ids.iter().map(ToString::to_string).collect();
        let found = self.resources.resolve_many(tenant, USER_KIND, &keys, None).await?;
        Ok(found
            .into_iter()
            .filter_map(|(id, resource)| Some((id.parse().ok()?, resource.title)))
            .collect())
    }

    /// Một trang audit log, mới nhất trước (mặc định).
    pub(crate) async fn list(
        &self,
        filter: Filter,
        page: &PageRequest,
    ) -> Result<Page<entity::Model>, Error> {
        let mut select = Entity::find();
        if let Some(actor) = filter.actor_user_id {
            select = select.filter(Column::ActorUserId.eq(actor));
        }
        if let Some(subject) = filter.subject {
            select = select.filter(Column::Subject.eq(subject));
        }
        if let Some(action) = filter.action {
            select = select.filter(Column::Action.eq(action));
        }
        if let Some(module) = filter.module {
            select = select.filter(Column::Module.eq(module));
        }
        fetch_page(select, Column::Id, |row| row.id, page, self.database.read()).await
    }

    pub(crate) async fn get(&self, id: Uuid) -> Result<entity::Model, Error> {
        Entity::find_by_id(id).one(self.database.read()).await?.ok_or_else(|| {
            Error::not_found("audit.entry_not_found", "Audit entry `{id}` does not exist.")
                .with_param("id", id)
        })
    }
}
