//! Phần của `content` trong cấu hình cho giao diện (D-271).

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::module::{ClientSection, TenantClientSection};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_spike_macros::Schema as ToSchema;
use serde::Serialize;

use crate::config::ContentConfig;

/// Content settings an interface needs.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = ContentClientConfig)]
struct ContentClientConfig {
    /// Languages entries may be written in: the tenant's `content.locales` setting, otherwise
    /// the server's `AUTOXIDE_CONTENT_LOCALES`.
    #[schema(example = json!(["en", "vi"]))]
    locales: Vec<String>,
    /// Language of a new entry when none is given: the tenant's `content.default_locale`,
    /// otherwise the server's `AUTOXIDE_CONTENT_DEFAULT_LOCALE`.
    #[schema(example = "en")]
    default_locale: String,
    /// First segment of the address of every post (`AUTOXIDE_CONTENT_POST_PATH_PREFIX`), without
    /// slashes: `blog` gives posts at `/blog/<slug>`.
    #[schema(example = "blog")]
    post_path_prefix: String,
    /// Hosts whose videos and posts may be embedded in an entry (`AUTOXIDE_CONTENT_EMBED_HOSTS`).
    #[schema(example = json!(["www.youtube.com", "player.vimeo.com"]))]
    embed_hosts: Vec<String>,
    /// Largest entry body accepted, in bytes.
    max_body_bytes: u64,
}

pub(crate) fn section(config: &ContentConfig) -> Result<Option<ClientSection>, Error> {
    let section = ContentClientConfig {
        locales: config.locales.clone(),
        default_locale: config.default_locale.clone(),
        post_path_prefix: config.post_path_prefix.clone(),
        embed_hosts: config.embed_hosts.clone(),
        max_body_bytes: u64::try_from(config.max_body_bytes).unwrap_or(u64::MAX),
    };
    ClientSection::new(&section).map(Some)
}

/// Ngôn ngữ của khách cho phần `content` (cấu hình `content.locales`, `content.default_locale`).
pub(crate) struct TenantLanguages(pub(crate) Arc<crate::service::ContentService>);

#[async_trait::async_trait]
impl TenantClientSection for TenantLanguages {
    async fn overrides(
        &self,
        tenant: &TenantContext,
    ) -> Result<serde_json::Map<String, serde_json::Value>, Error> {
        let languages = self.0.languages(tenant).await?;
        let mut fields = serde_json::Map::new();
        fields.insert("locales".to_owned(), serde_json::json!(languages.locales));
        fields.insert("default_locale".to_owned(), serde_json::json!(languages.default));
        let embed_hosts = self.0.embed_hosts(tenant).await?;
        fields.insert("embed_hosts".to_owned(), serde_json::json!(embed_hosts));
        Ok(fields)
    }
}
