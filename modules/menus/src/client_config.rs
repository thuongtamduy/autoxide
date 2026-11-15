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
    /// Languages menus may be written in (`AUTOXIDE_MENUS_LOCALES`), the default first.
    #[schema(example = json!(["en", "vi"]))]
    locales: Vec<String>,
    /// Language of a menu when none is given.
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
