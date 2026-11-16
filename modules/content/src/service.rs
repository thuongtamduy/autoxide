//! Service của `content`: MỌI logic nằm ở đây và trong bốn nhóm theo tính năng — `entries`
//! (soạn bài: nháp, revision, thân bài, slug, xin duyệt), `publishing` (đăng, hẹn giờ, xem
//! trước, đọc công khai, đường dẫn, chuyển hướng), `taxonomy` (danh mục, thẻ, tác giả, menu),
//! `ai` (dịch, tìm theo ý nghĩa, SEO). Handler REST và tool MCP chỉ lấy dữ liệu vào rồi gọi service — cùng
//! kết quả, cùng permission (luật kiến trúc 8). Mỗi lệnh ghi đi cùng event trong CÙNG
//! transaction (luật kiến trúc 3).

use std::sync::Arc;

use autoxide_kernel::ai::{Embedder, TextModel};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Outbox;
use autoxide_kernel::job::JobQueue;
use autoxide_kernel::tenancy::{DatabaseRegistry, TenantContext, TenantSettings, parsed_setting};
use autoxide_media::api::Media;
use autoxide_organizations::api::OrgDirectory;
use chrono::{DateTime, SubsecRound, Utc};

use crate::config::{self, ContentConfig, Languages};

// Không giữ `Database`: mọi hàm chạm dữ liệu nhận `&TenantContext` và dùng database của khách
// (bước 33).
pub(crate) struct ContentService {
    /// Sổ database: job định kỳ tra ngữ cảnh từng khách; link xem trước tìm khách ghi trong token.
    pub(crate) databases: DatabaseRegistry,
    pub(crate) outbox: Outbox,
    pub(crate) jobs: JobQueue,
    pub(crate) media: Arc<dyn Media>,
    /// Mô hình AI của module `ai` (bản "đã tắt" khi chưa cấu hình): gợi ý SEO, dịch (D-109).
    pub(crate) ai: Arc<dyn TextModel>,
    /// Embedding của module `ai`: tìm theo ý nghĩa.
    pub(crate) embedder: Arc<dyn Embedder>,
    pub(crate) config: ContentConfig,
    /// Cấu hình theo khách của kernel (`content.locales`, `content.default_locale`).
    pub(crate) settings: Arc<dyn TenantSettings>,
    /// Vector và câu tìm giữ trong bộ nhớ của tiến trình (tìm theo ý nghĩa).
    pub(crate) search_cache: crate::ai::semantic::SearchCache,
    /// Bản dịch của chương trình: câu kết quả kiểm SEO theo `Accept-Language` (D-264).
    pub(crate) translations: autoxide_kernel::i18n::Translations,
    /// Cây tổ chức (module `organizations`): ngôn ngữ của từng website (D-307). `None` khi app
    /// không đăng ký `organizations` (test của riêng module): mọi website dùng ngôn ngữ của khách.
    pub(crate) orgs: Option<Arc<dyn OrgDirectory>>,
}

/// Những gì service dùng từ module khác.
pub(crate) struct Dependencies {
    pub media: Arc<dyn Media>,
    pub ai: Arc<dyn TextModel>,
    pub embedder: Arc<dyn Embedder>,
    pub settings: Arc<dyn TenantSettings>,
}

/// Cài đặt ngôn ngữ của website, do module `organizations` khai.
const SITE_LANGUAGES: &str = "organizations.languages";

impl ContentService {
    pub(crate) fn new(
        databases: DatabaseRegistry,
        outbox: Outbox,
        jobs: JobQueue,
        dependencies: Dependencies,
        config: ContentConfig,
    ) -> Self {
        let Dependencies { media, ai, embedder, settings } = dependencies;
        let search_cache = crate::ai::semantic::SearchCache::default();
        ContentService {
            databases,
            outbox,
            jobs,
            media,
            ai,
            embedder,
            config,
            settings,
            search_cache,
            translations: Default::default(),
            orgs: None,
        }
    }

    /// Dịch câu kết quả kiểm SEO bằng `translations`.
    pub(crate) fn with_orgs(mut self, orgs: Option<Arc<dyn OrgDirectory>>) -> Self {
        self.orgs = orgs;
        self
    }

    /// Ngôn ngữ website `site` đưa ra (D-307): cài đặt `organizations.languages` của nút website
    /// (kế thừa theo cây), chỉ giữ ngôn ngữ khách viết được, ngôn ngữ đầu là mặc định. Không đặt,
    /// hay website của cả khách: ngôn ngữ của khách.
    pub(crate) async fn site_languages(
        &self,
        tenant: &TenantContext,
        site: &autoxide_kernel::tenancy::Site,
    ) -> Result<Languages, Error> {
        let languages = self.languages(tenant).await?;
        let Some(orgs) = self.orgs.as_ref() else { return Ok(languages) };
        let setting = orgs.setting(tenant, site.org(), SITE_LANGUAGES).await?;
        let wanted: Vec<String> = setting
            .and_then(|setting| setting.value)
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        let locales: Vec<String> =
            wanted.into_iter().filter(|locale| languages.locales.contains(locale)).collect();
        Ok(match locales.first().cloned() {
            Some(default) => Languages { locales, default },
            None => languages,
        })
    }

    pub(crate) fn with_translations(
        mut self,
        translations: autoxide_kernel::i18n::Translations,
    ) -> Self {
        self.translations = translations;
        self
    }

    /// Ngôn ngữ của khách `tenant`: cấu hình `content.locales` / `content.default_locale` của
    /// khách, thiếu thì `AUTOXIDE_CONTENT_LOCALES` / `AUTOXIDE_CONTENT_DEFAULT_LOCALE`. Đọc qua cache
    /// của kernel (không thêm truy vấn mỗi request).
    pub(crate) async fn languages(&self, tenant: &TenantContext) -> Result<Languages, Error> {
        let settings = self.settings.as_ref();
        let locales =
            parsed_setting(settings, tenant, &config::TENANT_LOCALES, config::tenant_locales)
                .await?;
        let default = parsed_setting(
            settings,
            tenant,
            &config::TENANT_DEFAULT_LOCALE,
            config::tenant_default_locale,
        )
        .await?;
        Ok(Languages::resolve(&self.config, locales, default))
    }

    /// `locale` đã chuẩn hoá nếu nằm trong `languages`; không thì thêm lỗi `field` (và trả ngôn
    /// ngữ mặc định để kiểm tiếp các ô khác). Không gửi: ngôn ngữ mặc định.
    pub(crate) fn check_locale(
        &self,
        languages: &Languages,
        raw: Option<&str>,
        field: &str,
        problems: &mut Vec<FieldError>,
    ) -> String {
        let Some(raw) = raw else {
            return languages.default.clone();
        };
        match crate::config::canonical_locale(raw.trim())
            .filter(|locale| languages.locales.contains(locale))
        {
            Some(locale) => locale,
            None => {
                problems.push(
                    FieldError::new(field, "locale_not_supported", "must be one of: {allowed}")
                        .with_param("allowed", languages.locales.join(", ")),
                );
                languages.default.clone()
            }
        }
    }
}

/// Lỗi kiểm dữ liệu với mã phù hợp: chỉ lỗi thân bài → `content.invalid_body`; chỉ ảnh
/// không có → `content.media_not_found`; chỉ ảnh không công khai → `content.media_not_public`;
/// lẫn nhiều loại → `validation_failed`. Mọi lỗi luôn nằm trong `errors` (D-083).
pub(crate) fn invalid(problems: Vec<FieldError>) -> Error {
    let all = |test: &dyn Fn(&FieldError) -> bool| problems.iter().all(test);
    // `ErrorKind` đứng ngay trước mã để `cargo xtask check` nhận ra đây là chỗ tạo lỗi.
    let (kind, code, message) = if all(&|p| p.code == "media_not_found") {
        (
            ErrorKind::Validation,
            "content.media_not_found",
            "A media item used by the entry does not exist (or is in the trash); see `errors`.",
        )
    } else if all(&|p| p.code == "media_not_public") {
        (
            ErrorKind::Validation,
            "content.media_not_public",
            "A media item used by the entry is not public, so readers could not see it. Make it public first; see `errors`.",
        )
    } else if all(&|p| p.field == "body" || p.field.starts_with("body.")) {
        (
            ErrorKind::Validation,
            "content.invalid_body",
            "The body is not valid; every problem is listed in `errors` with its place in the JSON.",
        )
    } else {
        return Error::validation(problems);
    };
    problems.into_iter().fold(Error::new(kind, code, message), Error::with_field)
}

/// Bây giờ, cắt tới micro giây để hai database trả cùng giá trị.
pub(crate) fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

pub(crate) fn entry_not_found(id: uuid::Uuid) -> Error {
    Error::not_found("content.not_found", "No post or page has the id `{id}`.").with_param("id", id)
}

/// Kiểm độ dài chữ (sau khi bỏ khoảng trắng hai đầu); thêm lỗi `field` nếu dài quá.
pub(crate) fn check_length(
    field: &str,
    value: Option<&str>,
    max: usize,
    problems: &mut Vec<FieldError>,
) {
    if value.is_some_and(|value| value.trim().chars().count() > max) {
        problems.push(
            FieldError::new(field, "length", "must be at most {max} characters long")
                .with_param("max", max),
        );
    }
}

/// Kiểm một địa chỉ `http(s)` (rỗng thì được); thêm lỗi `field` nếu sai.
pub(crate) fn check_url(field: &str, value: Option<&str>, problems: &mut Vec<FieldError>) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else { return };
    let lower = value.to_ascii_lowercase();
    let valid = (lower.starts_with("https://") || lower.starts_with("http://"))
        && value.len() <= 2048
        && !value.chars().any(|c| c.is_whitespace() || c.is_control());
    if !valid {
        problems.push(FieldError::new(
            field,
            "invalid",
            "must be an `https://` or `http://` address",
        ));
    }
}
