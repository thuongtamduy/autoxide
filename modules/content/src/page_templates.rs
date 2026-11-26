//! Trang mẫu (bước 67, D-229, D-20261122-page-templates): bố cục khối dựng sẵn theo ngành (bán lẻ,
//! ăn uống, dịch vụ, sự kiện, tuyển dụng — file `templates/pages/*.json` nhúng vào binary) và trang
//! của khách lưu thành mẫu. Bộ mẫu dựng sẵn vào bảng của khách bằng `POST /page-templates/presets`
//! (như bộ vị trí banner): danh sách là một bảng thường, phân trang như mọi danh sách. Tạo trang từ
//! mẫu = chép bố cục thành bản NHÁP, bỏ khối của module khách không dùng được.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, root_of};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::entity::page_template;
use crate::entries::{Entry, EntryFields, Kind, NewEntry, layout};
use crate::permissions::{CONTENT_READ, CONTENT_WRITE, CONTENT_WRITE_OWN};
use crate::service::{ContentService, now};

/// Ngành của mẫu.
pub(crate) const INDUSTRIES: [&str; 6] =
    ["retail", "fnb", "services", "events", "recruiting", "other"];
const NAME_MAX: usize = 120;
const DESCRIPTION_MAX: usize = 500;

pub(crate) type PageTemplate = page_template::Model;

/// Một mẫu dựng sẵn, theo ngôn ngữ.
#[derive(Debug, Deserialize)]
struct Preset {
    key: String,
    industry: String,
    name: BTreeMap<String, String>,
    description: BTreeMap<String, String>,
    layout: BTreeMap<String, Value>,
}

const FILES: [&str; 5] = [
    include_str!("../templates/pages/retail.json"),
    include_str!("../templates/pages/fnb.json"),
    include_str!("../templates/pages/services.json"),
    include_str!("../templates/pages/events.json"),
    include_str!("../templates/pages/recruiting.json"),
];

/// Bộ mẫu đã đọc. File hỏng là lỗi lúc build bản phát hành (test `bo_mau_hop_le` bắt), không làm
/// chương trình dừng: mẫu hỏng bị bỏ qua.
static PRESETS: LazyLock<Vec<Preset>> =
    LazyLock::new(|| FILES.iter().filter_map(|text| serde_json::from_str(text).ok()).collect());

pub(crate) fn page_template_not_found(id: Uuid) -> Error {
    Error::not_found("content.page_template_not_found", "There is no page template {id}.")
        .with_param("id", id)
}

/// Mẫu từ trang của khách.
pub(crate) struct SaveTemplate {
    pub entry_id: Uuid,
    pub name: String,
    pub industry: String,
    pub description: Option<String>,
}

/// Trang mới từ mẫu.
pub(crate) struct FromTemplate {
    pub template_id: Uuid,
    pub title: String,
    pub parent_id: Option<Uuid>,
    pub org_id: Option<Uuid>,
}

fn target(row: &PageTemplate) -> OrgTarget {
    OrgTarget::new(OrgId::from_uuid(row.org_id), row.created_by)
}

impl ContentService {
    /// Một trang mẫu trong phạm vi `content.read`; lọc theo ngành, ngôn ngữ.
    pub(crate) async fn list_page_templates(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        industry: Option<&str>,
        locale: Option<&str>,
        page: &PageRequest,
    ) -> Result<Page<PageTemplate>, Error> {
        principal.require(&CONTENT_READ)?;
        let scope = principal.scope_condition(&CONTENT_READ, page_template::Column::OrgId, None)?;
        let mut select = page_template::Entity::find().filter(scope).scoped(tenant);
        if let Some(industry) = industry {
            select = select.filter(page_template::Column::Industry.eq(industry));
        }
        if let Some(locale) = locale {
            select = select.filter(page_template::Column::Locale.eq(locale));
        }
        let read = tenant.database().read();
        fetch_page(select, page_template::Column::Id, |row| row.id, page, read).await
    }

    /// Cài phần còn thiếu của bộ mẫu dựng sẵn (mỗi mẫu một dòng mỗi ngôn ngữ khách dùng). Chạy lại
    /// không trùng; mẫu khách đã xoá thì cài lại. Cần `content.write` ở cả khách.
    pub(crate) async fn install_page_templates(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<Vec<PageTemplate>, Error> {
        principal.require(&CONTENT_WRITE)?;
        let languages = self.languages(tenant).await?;
        let root = root_of(tenant.id());
        let transaction = tenant.database().begin_write().await?;
        principal
            .require_write_at(&transaction, &CONTENT_WRITE, OrgTarget::new(root, None))
            .await?;
        let have: Vec<(String, String)> = page_template::Entity::find()
            .scoped(tenant)
            .filter(page_template::Column::PresetKey.is_not_null())
            .all(&transaction)
            .await?
            .into_iter()
            .filter_map(|row| Some((row.preset_key?, row.locale)))
            .collect();
        let at = now();
        let mut added = Vec::new();
        for preset in PRESETS.iter() {
            for locale in &languages.locales {
                let Some(layout) = preset.layout.get(locale) else { continue };
                if have.iter().any(|(key, known)| *key == preset.key && known == locale) {
                    continue;
                }
                let text =
                    |map: &BTreeMap<String, String>| map.get(locale).cloned().unwrap_or_default();
                let row = page_template::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    org_id: Set(root.as_uuid()),
                    industry: Set(preset.industry.clone()),
                    locale: Set(locale.clone()),
                    name: Set(text(&preset.name)),
                    description: Set(text(&preset.description)),
                    layout: Set(layout.to_string()),
                    preset_key: Set(Some(preset.key.clone())),
                    created_by: Set(None),
                    created_at: Set(at),
                }
                .insert(&transaction)
                .await?;
                added.push(row);
            }
        }
        transaction.commit().await?;
        Ok(added)
    }

    /// Lưu bố cục của trang `entry_id` thành mẫu (ở nút của trang). Cần đọc được trang và
    /// `content.write` ở nút đó.
    pub(crate) async fn save_page_template(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: SaveTemplate,
    ) -> Result<PageTemplate, Error> {
        principal.require(&CONTENT_WRITE)?;
        let entry = self.get(tenant, principal, input.entry_id).await?;
        let mut problems = Vec::new();
        let name = input.name.trim().to_owned();
        if name.is_empty() || name.chars().count() > NAME_MAX {
            problems.push(
                FieldError::new("name", "length", "must be 1 to {max} characters")
                    .with_param("max", NAME_MAX),
            );
        }
        let description = input.description.unwrap_or_default().trim().to_owned();
        if description.chars().count() > DESCRIPTION_MAX {
            problems.push(
                FieldError::new("description", "length", "must be at most {max} characters")
                    .with_param("max", DESCRIPTION_MAX),
            );
        }
        if !INDUSTRIES.contains(&input.industry.as_str()) {
            problems.push(
                FieldError::new("industry", "invalid", "must be one of {allowed}")
                    .with_param("allowed", INDUSTRIES.join(", ")),
            );
        }
        let body: Value = serde_json::from_str(&entry.body).unwrap_or(Value::Null);
        if !layout::is_layout(&body) {
            problems.push(FieldError::new(
                "entry_id",
                "not_layout",
                "the page has a rich text body; only block layouts can become templates",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let org = entry.org_id.map_or(root_of(tenant.id()), OrgId::from_uuid);
        let transaction = tenant.database().begin_write().await?;
        principal.require_write_at(&transaction, &CONTENT_WRITE, OrgTarget::new(org, None)).await?;
        let row = page_template::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(org.as_uuid()),
            industry: Set(input.industry),
            locale: Set(entry.locale.clone()),
            name: Set(name),
            description: Set(description),
            layout: Set(entry.body.clone()),
            preset_key: Set(None),
            created_by: Set(Some(principal.user_id())),
            created_at: Set(now()),
        }
        .insert(&transaction)
        .await?;
        transaction.commit().await?;
        Ok(row)
    }

    /// Xoá mẫu `id`. Cần `content.write` ở nút của mẫu.
    pub(crate) async fn delete_page_template(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        principal.require(&CONTENT_WRITE)?;
        let transaction = tenant.database().begin_write().await?;
        let row = self.find_page_template(&transaction, tenant, principal, id).await?;
        principal.require_write_at(&transaction, &CONTENT_WRITE, target(&row)).await?;
        page_template::Entity::delete_many()
            .scoped(tenant)
            .filter(page_template::Column::Id.eq(id))
            .exec(&transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Tạo trang NHÁP từ mẫu: chép bố cục ở ngôn ngữ của mẫu, bỏ khối khách không dùng được (module
    /// tắt, gói thiếu tính năng). Trả trang và tên các khối đã bỏ.
    pub(crate) async fn page_from_template(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: FromTemplate,
    ) -> Result<(Entry, Vec<String>), Error> {
        principal.require(&CONTENT_READ)?;
        // Tạo trang cần quyền viết (`content.write` hay `content.write_own`, `create` kiểm theo nút): kiểm trước khi
        // tra mẫu để người chỉ đọc không biết mẫu nào tồn tại.
        principal.require_any(&[&CONTENT_WRITE, &CONTENT_WRITE_OWN])?;
        let read = tenant.database().read();
        let template = self.find_page_template(read, tenant, principal, input.template_id).await?;
        let stored: Value =
            serde_json::from_str(&template.layout).unwrap_or_else(|_| layout::empty());
        let usable = self.usable_blocks(tenant).await?;
        let kept = layout::public(&stored, &usable);
        let mut dropped: Vec<String> = layout::blocks_of(&stored)
            .into_iter()
            .map(|(block, _, _)| block)
            .filter(|block| !usable.contains_key(block))
            .map(str::to_owned)
            .collect();
        dropped.sort();
        dropped.dedup();
        let entry = self
            .create(
                tenant,
                principal,
                NewEntry {
                    kind: Kind::Page,
                    type_id: None,
                    locale: Some(template.locale.clone()),
                    parent_id: input.parent_id,
                    org_id: input.org_id,
                    fields: EntryFields {
                        title: Some(input.title),
                        body: Some(kept),
                        ..EntryFields::default()
                    },
                },
            )
            .await?;
        Ok((entry, dropped))
    }

    async fn find_page_template<C: sea_orm::ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<PageTemplate, Error> {
        let row = page_template::Entity::find_by_id(id)
            .scoped(tenant)
            .one(connection)
            .await?
            .ok_or_else(|| page_template_not_found(id))?;
        principal.require_at(connection, &CONTENT_READ, target(&row)).await.map_err(|error| {
            if error.code() == "org_out_of_scope" { page_template_not_found(id) } else { error }
        })?;
        Ok(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bo_mau_hop_le() {
        assert_eq!(PRESETS.len(), FILES.len(), "every preset file parses");
        for preset in PRESETS.iter() {
            assert!(INDUSTRIES.contains(&preset.industry.as_str()), "{}", preset.key);
            for locale in ["en", "vi"] {
                assert!(
                    preset.name.contains_key(locale) && preset.description.contains_key(locale)
                );
                assert!(layout::is_layout(&preset.layout[locale]), "{} {locale}", preset.key);
            }
        }
    }
}
