//! Bộ giao diện của website (bước 67, D-229, D-20261122-theme-tokens): token màu, chữ, khoảng cách
//! theo nút của cây tổ chức, kế thừa (chuỗi đặt, cửa hàng ghi đè), kiểm tương phản WCAG AA (cảnh
//! báo, không chặn). Đọc / ghi cần `content.themes.manage` ở nút đó.

pub(crate) mod tokens;

use std::collections::BTreeMap;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, Site, TenantContext};
use autoxide_kernel::time::Timestamp;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, SqlErr};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::entity::theme;
use crate::events::{self, Change};
use crate::permissions::CONTENT_THEMES;
use crate::service::{ContentService, now};
use tokens::ContrastWarning;

/// Bộ giao diện của một nút: phần ghi đè của chính nó và token hiệu lực sau khi kế thừa.
#[derive(Debug, Clone)]
pub(crate) struct ThemeView {
    pub org_id: Uuid,
    /// Phần ghi đè của chính nút (rỗng khi chưa đặt gì).
    pub tokens: BTreeMap<String, Value>,
    /// Token hiệu lực của website của nút: mặc định ⊕ gốc ⊕ … ⊕ nút.
    pub effective: BTreeMap<String, Value>,
    /// Cặp màu hiệu lực dưới mức tương phản AA.
    pub warnings: Vec<ContrastWarning>,
    /// Phiên bản của phần ghi đè (`ETag`); nút chưa có dòng nào: mốc 0.
    pub version: Timestamp,
}

fn stored(row: &theme::Model) -> BTreeMap<String, Value> {
    serde_json::from_str(&row.tokens).unwrap_or_default()
}

impl ContentService {
    /// Token hiệu lực của website `site` (theo chuỗi nút của nó) và các dòng đã dùng. Một câu truy
    /// vấn.
    pub(crate) async fn effective_theme(
        &self,
        tenant: &TenantContext,
        site: &Site,
    ) -> Result<(BTreeMap<String, Value>, Vec<theme::Model>), Error> {
        let rows = theme::Entity::find()
            .scoped(tenant)
            .filter(theme::Column::OrgId.is_in(site.lineage_ids()))
            .all(tenant.database().read())
            .await?;
        // `lineage` gần nhất trước: ghi đè theo thứ tự ngược lại, gốc trước.
        let mut ordered: Vec<&theme::Model> = Vec::new();
        for org in site.lineage().iter().rev() {
            ordered.extend(rows.iter().filter(|row| row.org_id == org.as_uuid()));
        }
        let layers: Vec<BTreeMap<String, Value>> = ordered.iter().map(|row| stored(row)).collect();
        Ok((tokens::merge(&layers), rows))
    }

    /// Bộ giao diện của nút `org`. Cần `content.themes.manage` ở nút đó.
    pub(crate) async fn theme(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Uuid,
    ) -> Result<ThemeView, Error> {
        principal.require(&CONTENT_THEMES)?;
        let site = Site::of(tenant, OrgId::from_uuid(org)).await?;
        let target = OrgTarget::new(site.org(), None);
        principal.require_at(tenant.database().read(), &CONTENT_THEMES, target).await?;
        self.theme_view(tenant, &site).await
    }

    /// Logo và ảnh nền phải là ảnh công khai có thật của `media` (hiện trên website).
    async fn check_theme_media(
        &self,
        tenant: &TenantContext,
        checked: &BTreeMap<String, Value>,
    ) -> Result<(), Error> {
        let wanted: Vec<(&str, Uuid)> = ["logo_media_id", "background_media_id"]
            .into_iter()
            .filter_map(|key| {
                let id = checked.get(key).and_then(Value::as_str)?;
                Some((key, Uuid::parse_str(id).ok()?))
            })
            .collect();
        if wanted.is_empty() {
            return Ok(());
        }
        let ids: Vec<Uuid> = wanted.iter().map(|(_, id)| *id).collect();
        let found = self.media.find_many(tenant, &ids).await?;
        let mut problems = Vec::new();
        for (key, id) in wanted {
            let public = found.iter().any(|item| {
                item.id == id && item.visibility == autoxide_media::api::MediaVisibility::Public
            });
            if !public {
                problems.push(
                    FieldError::new(
                        format!("tokens.{key}"),
                        "media_not_found",
                        "media item {id} does not exist or is not public",
                    )
                    .with_param("id", id),
                );
            }
        }
        if problems.is_empty() { Ok(()) } else { Err(Error::validation(problems)) }
    }

    async fn theme_view(&self, tenant: &TenantContext, site: &Site) -> Result<ThemeView, Error> {
        let (effective, rows) = self.effective_theme(tenant, site).await?;
        let own = rows.iter().find(|row| row.org_id == site.org().as_uuid());
        Ok(ThemeView {
            org_id: site.org().as_uuid(),
            tokens: own.map(stored).unwrap_or_default(),
            warnings: tokens::contrast_warnings(&effective),
            effective,
            version: own.map_or(Timestamp::UNIX_EPOCH, |row| row.updated_at),
        })
    }

    /// Thay phần ghi đè của nút `org` bằng `input` (`null` bỏ một token). `expected`: phiên bản đã
    /// đọc (`None` = `If-Match: *`). Cần `content.themes.manage` ở nút đó.
    pub(crate) async fn put_theme(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Uuid,
        input: &Map<String, Value>,
        expected: Option<Timestamp>,
    ) -> Result<ThemeView, Error> {
        principal.require(&CONTENT_THEMES)?;
        let site = Site::of(tenant, OrgId::from_uuid(org)).await?;
        let target = OrgTarget::new(site.org(), None);
        principal.require_at(tenant.database().read(), &CONTENT_THEMES, target).await?;
        let checked = tokens::check(input).map_err(Error::validation)?;
        self.check_theme_media(tenant, &checked).await?;
        let at = now();
        let transaction = tenant.database().begin_write().await?;
        principal.require_write_at(&transaction, &CONTENT_THEMES, target).await?;
        let before = theme::Entity::find()
            .scoped(tenant)
            .filter(theme::Column::OrgId.eq(org))
            .one(&transaction)
            .await?;
        let version = before.as_ref().map_or(Timestamp::UNIX_EPOCH, |row| row.updated_at);
        if expected.is_some_and(|expected| expected != version) {
            return Err(autoxide_kernel::version::changed());
        }
        let text = serde_json::to_string(&checked).unwrap_or_else(|_| "{}".to_owned());
        let written = match &before {
            Some(row) => {
                let mut active: theme::ActiveModel = row.clone().into();
                active.tokens = Set(text);
                active.updated_by = Set(Some(principal.user_id()));
                active.updated_at = Set(at);
                active.update(&transaction).await?
            }
            None => theme::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                org_id: Set(org),
                tokens: Set(text),
                updated_by: Set(Some(principal.user_id())),
                created_at: Set(at),
                updated_at: Set(at),
            }
            .insert(&transaction)
            .await
            // Hai người cùng tạo bộ giao diện đầu tiên của nút: bên sau nhận 412 như sửa trùng.
            .map_err(|error| match error.sql_err() {
                Some(SqlErr::UniqueConstraintViolation(_)) => autoxide_kernel::version::changed(),
                _ => error.into(),
            })?,
        };
        let event = events::ThemeChanged(Change {
            before: before.as_ref().map(|row| json!({"org_id": row.org_id, "tokens": stored(row)})),
            after: Some(json!({"org_id": org, "tokens": &checked})),
            published: None,
        });
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::subject("theme", written.id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(org_id = %org, "theme changed");
        self.theme_view(tenant, &site).await
    }
}
