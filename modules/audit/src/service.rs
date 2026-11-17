//! Đọc audit log: danh sách có lọc và phân trang, và từng dòng.

use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::entity::{self, Column, Entity};

/// Điều kiện lọc; trường `None` không lọc.
#[derive(Debug, Clone, Default)]
pub(crate) struct Filter {
    pub actor_user_id: Option<Uuid>,
    pub subject: Option<String>,
    pub action: Option<String>,
    pub module: Option<String>,
    /// `user`, `system` hay `public` (đã kiểm ở handler).
    pub actor_kind: Option<String>,
}

// Service không giữ database: database là của khách, đi theo `TenantContext` của từng lời gọi.
pub(crate) struct AuditService {
    /// Sổ đối tượng của kernel: tên người làm (`auth.user`) không cần biết module `auth`.
    resources: autoxide_kernel::resource::ResourceDirectory,
    /// Event của mọi module: câu hoạt động và permission để thấy nó (bước 51 P5).
    events: autoxide_kernel::event::EventCatalog,
    translations: autoxide_kernel::i18n::Translations,
}

/// Một dòng hoạt động đã dựng câu.
pub(crate) struct ActivityLine {
    pub row: entity::Model,
    pub sentence: String,
}

/// Loại đối tượng "tài khoản người dùng" trong sổ đối tượng (module `auth` đăng ký; không có
/// thì không có tên).
const USER_KIND: &str = "auth.user";

impl AuditService {
    pub(crate) fn new(resources: autoxide_kernel::resource::ResourceDirectory) -> Self {
        AuditService {
            resources,
            events: autoxide_kernel::event::EventCatalog::default(),
            translations: autoxide_kernel::i18n::Translations::default(),
        }
    }

    pub(crate) fn with_events(
        mut self,
        events: autoxide_kernel::event::EventCatalog,
        translations: autoxide_kernel::i18n::Translations,
    ) -> Self {
        self.events = events;
        self.translations = translations;
        self
    }

    /// Hoạt động gần đây (bước 51 P5): chỉ event có câu hoạt động mà người gọi có permission để
    /// thấy, mới nhất trước, câu đã dịch theo `language` và điền tên người làm, trường của dữ liệu.
    pub(crate) async fn recent_activity(
        &self,
        tenant: &TenantContext,
        principal: &autoxide_kernel::access::Principal,
        limit: u32,
        language: autoxide_kernel::i18n::Language,
    ) -> Result<Vec<ActivityLine>, Error> {
        let mut sentences = std::collections::HashMap::new();
        for event in self.events.iter() {
            if let Some((sentence, reader)) = event.activity_line()
                && principal.require(&reader).is_ok()
            {
                sentences.insert(event.name(), sentence);
            }
        }
        if sentences.is_empty() {
            return Ok(Vec::new());
        }
        let actions: Vec<&str> = sentences.keys().copied().collect();
        let page = PageRequest::first(limit, autoxide_kernel::pagination::Order::Desc);
        let select = Entity::find().filter(Column::Action.is_in(actions)).scoped(tenant);
        let rows = fetch_page(select, Column::Id, |row| row.id, &page, tenant.database().read())
            .await?
            .data;
        let ids: Vec<uuid::Uuid> = rows.iter().filter_map(|row| row.actor_user_id).collect();
        let names = self.actor_names(tenant, &ids).await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let template = sentences.get(row.action.as_str()).copied().unwrap_or_default();
                let mut sentence = self.translations.describe(template, language);
                let actor = row
                    .actor_user_id
                    .and_then(|id| names.get(&id).cloned())
                    .unwrap_or_else(|| "—".to_owned());
                sentence = sentence.replace("{actor}", &actor);
                for side in [&row.after, &row.before] {
                    let Some(Ok(serde_json::Value::Object(fields))) =
                        side.as_deref().map(serde_json::from_str::<serde_json::Value>)
                    else {
                        continue;
                    };
                    for (key, value) in fields {
                        if let serde_json::Value::String(text) = value {
                            sentence = sentence.replace(&format!("{{{key}}}"), &text);
                        }
                    }
                }
                ActivityLine { row, sentence }
            })
            .collect())
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
        tenant: &TenantContext,
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
        if let Some(kind) = filter.actor_kind {
            // Dòng ghi trước khi có cột `actor_kind` (NULL) hiểu theo `actor_user_id`, như khi trả về.
            let legacy = match kind.as_str() {
                "user" => Some(Column::ActorUserId.is_not_null()),
                "system" => Some(Column::ActorUserId.is_null()),
                _ => None,
            };
            let mut condition = Condition::any().add(Column::ActorKind.eq(kind));
            if let Some(legacy) = legacy {
                condition =
                    condition.add(Condition::all().add(Column::ActorKind.is_null()).add(legacy));
            }
            select = select.filter(condition);
        }
        fetch_page(select.scoped(tenant), Column::Id, |row| row.id, page, tenant.database().read())
            .await
    }

    pub(crate) async fn get(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<entity::Model, Error> {
        let found = Entity::find_by_id(id).scoped(tenant).one(tenant.database().read()).await?;
        found.ok_or_else(|| {
            Error::not_found("audit.entry_not_found", "Audit entry `{id}` does not exist.")
                .with_param("id", id)
        })
    }
}
