//! Phần của `menus` trong cấu hình cho giao diện (D-271).

use autoxide_kernel::error::Error;
use autoxide_kernel::module::ClientSection;
use serde::Serialize;
use utoipa::ToSchema;

use crate::config::MenusConfig;

/// Menu settings an interface needs.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = MenusClientConfig)]
struct MenusClientConfig {
    /// Languages menus may be written in (tenant setting `menus.locales`, otherwise
    /// `AUTOXIDE_MENUS_LOCALES`), in the order they are set.
    #[schema(example = json!(["en", "vi"]))]
    locales: Vec<String>,
    /// Language of a menu when none is given (tenant setting `menus.default_locale`, otherwise
    /// `AUTOXIDE_MENUS_DEFAULT_LOCALE`); always one of `locales`.
    #[schema(example = "en")]
    default_locale: String,
}

pub(crate) fn section(config: &MenusConfig) -> Result<Option<ClientSection>, Error> {
    let section = MenusClientConfig {
        locales: config.locales.clone(),
        default_locale: config.default_locale.clone(),
    };
    ClientSection::new(&section).map(Some)
}

/// Ngôn ngữ menu của khách (cấu hình `menus.locales`, `menus.default_locale`).
pub(crate) struct TenantLanguages(pub(crate) std::sync::Arc<crate::service::MenuService>);

#[async_trait::async_trait]
impl autoxide_kernel::module::TenantClientSection for TenantLanguages {
    async fn overrides(
        &self,
        tenant: &autoxide_kernel::tenancy::TenantContext,
    ) -> Result<serde_json::Map<String, serde_json::Value>, Error> {
        let (locales, default) = self.0.tenant_locales(tenant).await?;
        let mut fields = serde_json::Map::new();
        fields.insert("locales".to_owned(), serde_json::json!(locales));
        fields.insert("default_locale".to_owned(), serde_json::json!(default));
        Ok(fields)
    }
}
