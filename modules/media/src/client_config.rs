//! Phần của `media` trong cấu hình cho giao diện (D-271).

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::module::{ClientSection, TenantClientSection};
use autoxide_kernel::tenancy::TenantContext;
use serde::Serialize;
use utoipa::ToSchema;

use crate::config::MediaConfig;

/// Media library settings an interface needs.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = MediaClientConfig)]
struct MediaClientConfig {
    /// Largest upload, in bytes (`AUTOXIDE_MEDIA_MAX_BYTES`).
    #[schema(example = 52428800)]
    max_bytes: u64,
    /// Media types accepted at upload (`AUTOXIDE_MEDIA_ALLOWED_TYPES`): `type/subtype` or
    /// `type/*`.
    #[schema(example = json!(["image/*", "application/pdf"]))]
    allowed_types: Vec<String>,
    /// Languages of `alt` translations: the tenant's `media.alt_locales` setting, otherwise the
    /// server's `AUTOXIDE_MEDIA_ALT_LOCALES`. Show one alt field per language.
    #[schema(example = json!(["en", "vi"]))]
    alt_locales: Vec<String>,
    /// Widths of the smaller copies made of images, in pixels.
    #[schema(example = json!([320, 640, 1280]))]
    image_widths: Vec<u32>,
    /// Days an item stays in the trash before it is deleted for good.
    #[schema(example = 30)]
    trash_days: u32,
}

pub(crate) fn section(config: &MediaConfig) -> Result<Option<ClientSection>, Error> {
    let section = MediaClientConfig {
        max_bytes: config.limits.max_bytes,
        allowed_types: config.limits.allowed_types.clone(),
        alt_locales: config.alt_locales.clone(),
        image_widths: config.image_widths.clone(),
        trash_days: config.trash_days,
    };
    ClientSection::new(&section).map(Some)
}

/// Ngôn ngữ dịch `alt` của khách cho phần `media` (cấu hình `media.alt_locales`).
pub(crate) struct TenantAltLocales(pub(crate) Arc<crate::service::MediaService>);

#[async_trait::async_trait]
impl TenantClientSection for TenantAltLocales {
    async fn overrides(
        &self,
        tenant: &TenantContext,
    ) -> Result<serde_json::Map<String, serde_json::Value>, Error> {
        let locales = self.0.alt_locales(tenant).await?;
        let mut fields = serde_json::Map::new();
        fields.insert("alt_locales".to_owned(), serde_json::json!(locales));
        Ok(fields)
    }
}
