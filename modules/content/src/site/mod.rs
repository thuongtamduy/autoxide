//! Đầu trang và chân trang của website (bước 67, D-234, D-20261122-public-site): mỗi khu là một bố
//! cục khối (cùng bộ kiểm, giới hạn và khối như trang) theo (website, khu, ngôn ngữ, ô chủ đề). Sửa vào
//! bản nháp; `publish` mới thành bản công khai. Bản chủ đề kèm được khu riêng (ô = ID bản chủ đề).
//! Đọc công khai cả khung website ở `public.rs`.

pub(crate) mod public;

use std::sync::{Arc, PoisonError, RwLock};

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, Site, TenantContext};
use autoxide_kernel::time::Timestamp;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set, SqlErr,
};
use serde_json::{Value, json};
use uuid::Uuid;

pub(crate) use public::SiteCache;

use crate::api::{SiteExtension, SiteExtensions};
use crate::entity::site_region;
use crate::entries::layout;
use crate::events::{self, Change};
use crate::permissions::CONTENT_THEMES;
use crate::service::{ContentService, now};

/// Khu cố định của website.
pub(crate) const REGIONS: [&str; 2] = ["header", "footer"];
/// Ô của theme gốc (không theo bản chủ đề nào).
pub(crate) const BASE: &str = "base";

pub(crate) type SiteRegion = site_region::Model;

/// Khu nào: website, khu, ngôn ngữ, bản chủ đề (`None`: theme gốc).
#[derive(Debug, Clone)]
pub(crate) struct RegionKey {
    pub org_id: Uuid,
    pub region: String,
    pub locale: String,
    pub variant_id: Option<Uuid>,
}

pub(crate) fn region_not_found(id: Uuid) -> Error {
    Error::not_found("content.site_region_not_found", "There is no website header or footer {id}.")
        .with_param("id", id)
}

/// Sổ phần mở rộng của khung website (`api::SiteExtensions`). Ghi chỉ lúc khởi động.
#[derive(Default)]
pub(crate) struct ExtensionRegistry(RwLock<Vec<(&'static str, Arc<dyn SiteExtension>)>>);

impl ExtensionRegistry {
    pub(crate) fn all(&self) -> Vec<(&'static str, Arc<dyn SiteExtension>)> {
        self.0.read().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl SiteExtensions for ExtensionRegistry {
    fn register(
        &self,
        module: &'static str,
        extension: Arc<dyn SiteExtension>,
    ) -> Result<(), Error> {
        let mut all = self.0.write().unwrap_or_else(PoisonError::into_inner);
        if all.iter().any(|(known, _)| *known == module) {
            return Err(Error::internal(format!(
                "module `{module}` registers its website part twice"
            )));
        }
        all.push((module, extension));
        Ok(())
    }
}

impl ContentService {
    /// Kiểm khu được hỏi: tên khu, ngôn ngữ của website, bản chủ đề có thật. Trả website.
    async fn region_site(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        key: &RegionKey,
    ) -> Result<Site, Error> {
        let site = self.themes_site(tenant, principal, key.org_id).await?;
        let mut problems = Vec::new();
        if !REGIONS.contains(&key.region.as_str()) {
            problems.push(FieldError::new("region", "invalid", "must be `header` or `footer`"));
        }
        let languages = self.site_languages(tenant, &site).await?;
        self.check_locale(&languages, Some(&key.locale), "locale", &mut problems);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        if let Some(variant) = key.variant_id {
            self.variant(tenant, principal, variant).await?;
        }
        Ok(site)
    }

    /// Khu `key` (nháp và bản đăng); `None` khi chưa ai tạo.
    pub(crate) async fn site_region(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        key: &RegionKey,
    ) -> Result<Option<SiteRegion>, Error> {
        self.region_site(tenant, principal, key).await?;
        find_key(tenant.database().read(), tenant, key).await
    }

    /// Thay bản nháp của khu `key` bằng `layout` (kiểm như bố cục trang). `expected`: phiên bản đã đọc
    /// (khu chưa có: `*` hay mốc 0). Bản công khai không đổi tới khi `publish`.
    pub(crate) async fn put_site_region(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        key: &RegionKey,
        layout: &Value,
        expected: Option<Timestamp>,
    ) -> Result<SiteRegion, Error> {
        let site = self.region_site(tenant, principal, key).await?;
        let mut problems = Vec::new();
        if !layout::is_layout(layout) {
            problems.push(FieldError::new("layout.type", "invalid", "must be `layout`"));
            return Err(Error::validation(problems));
        }
        let info = self.check_body(tenant, "page", layout, &mut problems).await?;
        let media = info.media_ids.iter().map(|id| ("layout".to_owned(), *id)).collect();
        self.check_media_ids(tenant, media, false, &mut problems).await?;
        self.check_records(tenant, &info.records, &mut problems).await?;
        for problem in &mut problems {
            if let Some(rest) = problem.field.strip_prefix("body") {
                problem.field = format!("layout{rest}");
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let text = serde_json::to_string(layout).map_err(Error::internal)?;
        let at = now();
        let transaction = tenant.database().begin_write().await?;
        principal
            .require_write_at(&transaction, &CONTENT_THEMES, OrgTarget::new(site.org(), None))
            .await?;
        let before = find_key(&transaction, tenant, key).await?;
        let version = before.as_ref().map_or(Timestamp::UNIX_EPOCH, |row| row.updated_at);
        if expected.is_some_and(|expected| expected != version) {
            return Err(autoxide_kernel::version::changed());
        }
        let after = match &before {
            Some(row) => {
                let mut active: site_region::ActiveModel = row.clone().into();
                active.draft_layout = Set(Some(text));
                active.updated_by = Set(Some(principal.user_id()));
                active.updated_at = Set(at);
                active.update(&transaction).await?
            }
            None => site_region::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                org_id: Set(key.org_id),
                region: Set(key.region.clone()),
                locale: Set(key.locale.clone()),
                variant_slot: Set(slot(key.variant_id)),
                layout: Set(None),
                draft_layout: Set(Some(text)),
                published_at: Set(None),
                updated_by: Set(Some(principal.user_id())),
                created_at: Set(at),
                updated_at: Set(at),
            }
            .insert(&transaction)
            .await
            // Hai người cùng tạo khu đầu tiên: bên sau nhận 412 như sửa trùng.
            .map_err(|error| match error.sql_err() {
                Some(SqlErr::UniqueConstraintViolation(_)) => autoxide_kernel::version::changed(),
                _ => error.into(),
            })?,
        };
        self.record_region(&transaction, tenant, principal, &after, "draft").await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Đăng bản nháp của khu `id`: thành bản công khai, khung website đổi ngay trên máy này.
    pub(crate) async fn publish_site_region(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        expected: Option<Timestamp>,
    ) -> Result<SiteRegion, Error> {
        principal.require(&CONTENT_THEMES)?;
        let transaction = tenant.database().begin_write().await?;
        let before = site_region::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| region_not_found(id))?;
        let target = OrgTarget::new(OrgId::from_uuid(before.org_id), None);
        principal.require_at(&transaction, &CONTENT_THEMES, target).await.map_err(|error| {
            if error.code() == "org_out_of_scope" { region_not_found(id) } else { error }
        })?;
        principal.require_write_at(&transaction, &CONTENT_THEMES, target).await?;
        if expected.is_some_and(|expected| expected != before.updated_at) {
            return Err(autoxide_kernel::version::changed());
        }
        let Some(draft) = before.draft_layout.clone() else {
            return Err(Error::conflict(
                "content.site_region_no_draft",
                "There is no draft to publish.",
            ));
        };
        let at = now();
        let mut active: site_region::ActiveModel = before.into();
        active.layout = Set(Some(draft));
        active.published_at = Set(Some(at));
        active.updated_by = Set(Some(principal.user_id()));
        active.updated_at = Set(at);
        let after = active.update(&transaction).await?;
        self.record_region(&transaction, tenant, principal, &after, "published").await?;
        transaction.commit().await?;
        Ok(after)
    }

    async fn record_region<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        principal: &Principal,
        row: &SiteRegion,
        state: &str,
    ) -> Result<(), Error> {
        // Khung website đã tính trên máy này hết hiệu lực ngay; máy khác thấy trong 30 giây.
        self.site_cache.invalidate();
        let after = json!({"kind": "region", "id": row.id, "org_id": row.org_id, "region": row.region,
            "locale": row.locale, "variant_slot": row.variant_slot, "state": state});
        let event =
            events::ThemeChanged(Change { before: None, after: Some(after), published: None });
        self.outbox
            .record(
                connection,
                tenant,
                Actor::from(principal),
                events::subject("site_region", row.id),
                &event,
            )
            .await?;
        Ok(())
    }
}

/// Ô của khu theo bản chủ đề.
pub(crate) fn slot(variant: Option<Uuid>) -> String {
    variant.map_or_else(|| BASE.to_owned(), |id| id.to_string())
}

async fn find_key<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    key: &RegionKey,
) -> Result<Option<SiteRegion>, Error> {
    Ok(site_region::Entity::find()
        .scoped(tenant)
        .filter(site_region::Column::OrgId.eq(key.org_id))
        .filter(site_region::Column::Region.eq(key.region.clone()))
        .filter(site_region::Column::Locale.eq(key.locale.clone()))
        .filter(site_region::Column::VariantSlot.eq(slot(key.variant_id)))
        .one(connection)
        .await?)
}
