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
}

impl AuditService {
    pub(crate) fn new(database: Database) -> Self {
        AuditService { database }
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
