//! Phần của `content` trong cấu hình cho giao diện (D-271).

use autoxide_kernel::error::Error;
use autoxide_kernel::module::ClientSection;
use serde::Serialize;
use utoipa::ToSchema;

use crate::config::ContentConfig;

/// Content settings an interface needs.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = ContentClientConfig)]
struct ContentClientConfig {
    /// Languages entries may be written in (`AUTOXIDE_CONTENT_LOCALES`), the default first.
    #[schema(example = json!(["en", "vi"]))]
    locales: Vec<String>,
    /// Language of a new entry when none is given.
    #[schema(example = "en")]
    default_locale: String,
    /// Path prefix of posts on the website (`AUTOXIDE_CONTENT_POST_PATH_PREFIX`), e.g. `/blog/`.
    #[schema(example = "/blog/")]
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
