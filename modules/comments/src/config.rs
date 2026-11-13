//! Cấu hình của module `comments`, tiền tố `AUTOXIDE_COMMENTS_`. Mọi biến có mặc định chạy được
//! ngay, không biến nào bắt buộc lúc khởi động: thiếu địa chỉ trang xác minh / huỷ theo dõi thì
//! chỉ mail báo trả lời bị tắt. Yêu cầu pháp lý còn chưa chắc (Nghị định 147/2024, luật dữ liệu cá
//! nhân) nên là cấu hình — tắt bình luận, tắt khách, bắt đăng nhập theo loại đích, thời hạn giữ —
//! không cài cứng; người cài tự đối chiếu văn bản gốc.

use std::collections::BTreeMap;

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const ENABLED: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_ENABLED",
    "Whether new comments are taken at all (`true` or `false`). With `false` existing comments \
     can still be read and moderated.",
)
.with_default("true");

pub(crate) const TARGET_TYPES: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_TARGET_TYPES",
    "Comma-separated kinds of things that take comments, written `<module>.<kind>`. A kind \
     another module resolves (e.g. `content.entry`) must exist and be public; any other kind \
     is taken as given.",
)
.with_default("content.entry");

pub(crate) const ALLOW_GUESTS: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_ALLOW_GUESTS",
    "Whether people without an account may comment (`true` or `false`). Some laws (e.g. a \
     reading of Vietnam's Decree 147/2024 for social networks) may require signed-in, verified \
     accounts: check what applies to you.",
)
.with_default("true");

pub(crate) const SIGNED_IN_ONLY: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_SIGNED_IN_ONLY_TYPES",
    "Comma-separated target kinds where only signed-in users may comment, even when guests are \
     allowed elsewhere.",
)
.with_example("media.item");

pub(crate) const VERIFIED_ONLY: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_VERIFIED_ONLY_TYPES",
    "Comma-separated target kinds where only verified accounts may comment or answer (e.g. \
     product questions, Vietnam's Decree 147/2024): guests are refused, and signed-in users are \
     checked with the module that keeps account verification. With no such module installed, \
     every signed-in account counts as verified.",
)
.with_example("catalog.product");

pub(crate) const GUEST_MODERATION: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_GUEST_MODERATION",
    "When a guest's comment needs approval: `pre` (always), `first_time` (guests have no account \
     to remember, so this works like `pre`) or `post` (shown at once).",
)
.with_default("pre");

pub(crate) const USER_MODERATION: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_USER_MODERATION",
    "When a signed-in user's comment needs approval: `pre` (always), `first_time` (until one of \
     their comments was approved) or `post` (shown at once).",
)
.with_default("first_time");

pub(crate) const MODERATION_BY_TYPE: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_MODERATION_BY_TYPE",
    "Comma-separated overrides per target kind, written `<kind>:guest=<mode>` or \
     `<kind>:user=<mode>` with the modes above.",
)
.with_example("content.entry:guest=pre,content.entry:user=post");

pub(crate) const MAX_LENGTH: Setting =
    Setting::new("AUTOXIDE_COMMENTS_MAX_LENGTH", "Longest comment, in characters (10 to 20000).")
        .with_default("4000");

pub(crate) const MAX_LINKS: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_MAX_LINKS",
    "A comment with more links than this is held for approval (0 to 100).",
)
.with_default("2");

pub(crate) const EDIT_MINUTES: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_EDIT_MINUTES",
    "Minutes during which authors may change their comment (0 to 1440; 0: never). Deleting \
     their own comment is always allowed.",
)
.with_default("15");

pub(crate) const REPORT_THRESHOLD: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_REPORT_THRESHOLD",
    "Reports that hold a shown comment for approval again (1 to 1000). This happens once per \
     comment: after a moderator approves it, more reports do not hide it.",
)
.with_default("3");

pub(crate) const AUTO_CLOSE_DAYS: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_AUTO_CLOSE_DAYS",
    "Days after its first comment a target stops taking comments (0 to 36500; 0: never).",
)
.with_default("0");

pub(crate) const BANNED_WORDS: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_BANNED_WORDS",
    "Comma-separated words or phrases that make a comment refused (whole words, ignoring case \
     and accents).",
)
.with_example("casino,viagra");

pub(crate) const SUSPECT_WORDS: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_SUSPECT_WORDS",
    "Comma-separated words or phrases that hold a comment for approval (whole words, ignoring \
     case and accents).",
)
.with_example("giảm giá,free money");

pub(crate) const RESERVED_NAMES: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_RESERVED_NAMES",
    "Comma-separated names guests and users may not show as theirs (ignoring case and \
     accents); moderators may.",
)
.with_default("admin,administrator,moderator,root,system");

pub(crate) const PER_IP_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_PER_IP_PER_HOUR",
    "Guest comments accepted per target kind from one network block (IPv4 /24, IPv6 /64) per \
     hour (1 to 100000); more are dropped silently.",
)
.with_default("20");

pub(crate) const PER_EMAIL_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_PER_EMAIL_PER_HOUR",
    "Guest comments accepted per target kind from one email address per hour (1 to 100000); \
     more are dropped silently.",
)
.with_default("10");

pub(crate) const PER_USER_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_PER_USER_PER_HOUR",
    "Comments per hour from one signed-in user (1 to 1000000); more are refused with 429.",
)
.with_default("60");

pub(crate) const REPORTS_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_REPORTS_PER_HOUR",
    "Reports counted per hour from one network block or user (1 to 100000); more are \
     answered as usual but not stored.",
)
.with_default("30");

pub(crate) const RETENTION_DAYS: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_RETENTION_DAYS",
    "Days a comment that was never shown (waiting for approval or marked as spam) is kept \
     before a daily job deletes it (1 to 3650). Comments that were shown once are never deleted \
     on their own: a moderator decides.",
)
.with_default("30");

pub(crate) const IP_RETENTION_DAYS: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_IP_RETENTION_DAYS",
    "Days the hashed network address of a comment (and of a guest's report) is kept before the \
     daily job clears it (1 to 3650). Raw addresses are never stored.",
)
.with_default("90");

pub(crate) const VERIFY_URL: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_VERIFY_URL",
    "Page of your website that confirms an email address for reply mails, with `{token}` where \
     the token goes; the page sends it to `POST /api/v1/comments/public/verify`. Without it \
     (or the unsubscribe page) reply mails are off.",
)
.with_example("https://www.example.com/comments/verify?token={token}");

pub(crate) const UNSUBSCRIBE_URL: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_UNSUBSCRIBE_URL",
    "Page of your website that stops reply mails, with `{token}` where the token goes; the page \
     sends it to `POST /api/v1/comments/public/unsubscribe`. Every reply mail links there.",
)
.with_example("https://www.example.com/comments/unsubscribe?token={token}");

pub(crate) const SITE_URL: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_SITE_URL",
    "Address of your website, put before the path of the commented page in reply mails.",
)
.with_example("https://www.example.com");

pub(crate) const VERIFY_MAILS_PER_DAY: Setting = Setting::new(
    "AUTOXIDE_COMMENTS_VERIFY_MAILS_PER_DAY",
    "Address confirmation mails sent to one address in 24 hours (1 to 100).",
)
.with_default("3");

/// Khi nào bình luận cần duyệt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Duyệt mọi bình luận.
    Pre,
    /// Duyệt tới khi có một bình luận của tài khoản được duyệt.
    FirstTime,
    /// Hiện ngay.
    Post,
}

impl Mode {
    pub(crate) fn parse(raw: &str) -> Option<Mode> {
        match raw.trim() {
            "pre" => Some(Mode::Pre),
            "first_time" => Some(Mode::FirstTime),
            "post" => Some(Mode::Post),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommentsConfig {
    pub enabled: bool,
    pub target_types: Vec<String>,
    pub allow_guests: bool,
    pub signed_in_only: Vec<String>,
    /// Loại đích chỉ tài khoản đã xác thực được viết (D-202).
    pub verified_only: Vec<String>,
    pub guest_mode: Mode,
    pub user_mode: Mode,
    /// (loại đích, `true` = khách) → chế độ.
    pub mode_overrides: BTreeMap<(String, bool), Mode>,
    pub max_length: usize,
    pub max_links: usize,
    pub edit_minutes: u32,
    pub report_threshold: u32,
    /// 0: không tự đóng.
    pub auto_close_days: u32,
    /// Đã `search::fold`.
    pub banned_words: Vec<String>,
    pub suspect_words: Vec<String>,
    pub reserved_names: Vec<String>,
    pub per_ip_per_hour: u32,
    pub per_email_per_hour: u32,
    pub per_user_per_hour: u32,
    pub reports_per_hour: u32,
    pub retention_days: u32,
    pub ip_retention_days: u32,
    pub verify_url: Option<String>,
    pub unsubscribe_url: Option<String>,
    pub site_url: Option<String>,
    pub verify_mails_per_day: u32,
}

impl Default for CommentsConfig {
    fn default() -> Self {
        CommentsConfig {
            enabled: true,
            target_types: vec!["content.entry".to_owned()],
            allow_guests: true,
            signed_in_only: Vec::new(),
            verified_only: Vec::new(),
            guest_mode: Mode::Pre,
            user_mode: Mode::FirstTime,
            mode_overrides: BTreeMap::new(),
            max_length: 4000,
            max_links: 2,
            edit_minutes: 15,
            report_threshold: 3,
            auto_close_days: 0,
            banned_words: Vec::new(),
            suspect_words: Vec::new(),
            reserved_names: ["admin", "administrator", "moderator", "root", "system"]
                .map(str::to_owned)
                .to_vec(),
            per_ip_per_hour: 20,
            per_email_per_hour: 10,
            per_user_per_hour: 60,
            reports_per_hour: 30,
            retention_days: 30,
            ip_retention_days: 90,
            verify_url: None,
            unsubscribe_url: None,
            site_url: None,
            verify_mails_per_day: 3,
        }
    }
}

impl CommentsConfig {
    pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<Self, Invalid> {
        let enabled = reader.get_with(&ENABLED, flag);
        let types = reader.get_with(&TARGET_TYPES, |raw| {
            let types = kinds(raw)?;
            if types.is_empty() { Err("must name at least one kind".to_owned()) } else { Ok(types) }
        });
        let guests = reader.get_with(&ALLOW_GUESTS, flag);
        let signed_in_only =
            reader.optional_with(&SIGNED_IN_ONLY, kinds).map(Option::unwrap_or_default);
        let verified_only =
            reader.optional_with(&VERIFIED_ONLY, kinds).map(Option::unwrap_or_default);
        let guest_mode = reader.get_with(&GUEST_MODERATION, mode);
        let user_mode = reader.get_with(&USER_MODERATION, mode);
        let overrides =
            reader.optional_with(&MODERATION_BY_TYPE, overrides).map(Option::unwrap_or_default);
        let max_length = reader.get_with(&MAX_LENGTH, |raw| parse_in_range(raw, 10usize, 20_000));
        let max_links = reader.get_with(&MAX_LINKS, |raw| parse_in_range(raw, 0usize, 100));
        let edit = reader.get_with(&EDIT_MINUTES, |raw| parse_in_range(raw, 0u32, 1440));
        let threshold = reader.get_with(&REPORT_THRESHOLD, |raw| parse_in_range(raw, 1u32, 1000));
        let close = reader.get_with(&AUTO_CLOSE_DAYS, |raw| parse_in_range(raw, 0u32, 36_500));
        let banned = reader.optional_with(&BANNED_WORDS, words).map(Option::unwrap_or_default);
        let suspect = reader.optional_with(&SUSPECT_WORDS, words).map(Option::unwrap_or_default);
        let reserved = reader.get_with(&RESERVED_NAMES, words);
        let per_ip = reader.get_with(&PER_IP_PER_HOUR, |raw| parse_in_range(raw, 1u32, 100_000));
        let per_email =
            reader.get_with(&PER_EMAIL_PER_HOUR, |raw| parse_in_range(raw, 1u32, 100_000));
        let per_user =
            reader.get_with(&PER_USER_PER_HOUR, |raw| parse_in_range(raw, 1u32, 1_000_000));
        let reports = reader.get_with(&REPORTS_PER_HOUR, |raw| parse_in_range(raw, 1u32, 100_000));
        let retention = reader.get_with(&RETENTION_DAYS, |raw| parse_in_range(raw, 1u32, 3650));
        let ip = reader.get_with(&IP_RETENTION_DAYS, |raw| parse_in_range(raw, 1u32, 3650));
        let verify_url = reader.optional_with(&VERIFY_URL, token_url);
        let unsubscribe_url = reader.optional_with(&UNSUBSCRIBE_URL, token_url);
        let site_url = reader.optional_with(&SITE_URL, site_url);
        let verify_mails =
            reader.get_with(&VERIFY_MAILS_PER_DAY, |raw| parse_in_range(raw, 1u32, 100));
        let target_types = types?;
        let mode_overrides = overrides?;
        if let Some(((kind, _), _)) =
            mode_overrides.iter().find(|((kind, _), _)| !target_types.contains(kind))
        {
            return reader.reject(
                &MODERATION_BY_TYPE,
                format!("names `{kind}`, which is not in AUTOXIDE_COMMENTS_TARGET_TYPES"),
            );
        }
        Ok(CommentsConfig {
            enabled: enabled?,
            target_types,
            allow_guests: guests?,
            signed_in_only: signed_in_only?,
            verified_only: verified_only?,
            guest_mode: guest_mode?,
            user_mode: user_mode?,
            mode_overrides,
            max_length: max_length?,
            max_links: max_links?,
            edit_minutes: edit?,
            report_threshold: threshold?,
            auto_close_days: close?,
            banned_words: banned?,
            suspect_words: suspect?,
            reserved_names: reserved?,
            per_ip_per_hour: per_ip?,
            per_email_per_hour: per_email?,
            per_user_per_hour: per_user?,
            reports_per_hour: reports?,
            retention_days: retention?,
            ip_retention_days: ip?,
            verify_url: verify_url?,
            unsubscribe_url: unsubscribe_url?,
            site_url: site_url?,
            verify_mails_per_day: verify_mails?,
        })
    }

    /// Chế độ duyệt cho loại đích `kind`, khách hay người dùng đăng nhập.
    pub(crate) fn mode(&self, kind: &str, guest: bool) -> Mode {
        self.mode_overrides.get(&(kind.to_owned(), guest)).copied().unwrap_or(if guest {
            self.guest_mode
        } else {
            self.user_mode
        })
    }

    /// Mail báo trả lời bật khi có đủ trang xác minh và trang huỷ theo dõi.
    pub(crate) fn reply_mails(&self) -> bool {
        self.verify_url.is_some() && self.unsubscribe_url.is_some()
    }
}

fn flag(raw: &str) -> Result<bool, String> {
    match raw.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err("must be `true` or `false`".to_owned()),
    }
}

fn mode(raw: &str) -> Result<Mode, String> {
    Mode::parse(raw).ok_or_else(|| "must be `pre`, `first_time` or `post`".to_owned())
}

/// Loại đích `<module>.<loại>`: chữ thường, số, `_`.
pub(crate) fn is_kind(kind: &str) -> bool {
    let part = |part: &str| {
        !part.is_empty()
            && part.starts_with(|c: char| c.is_ascii_lowercase())
            && part.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    };
    kind.len() <= 100
        && kind.split_once('.').is_some_and(|(module, rest)| part(module) && part(rest))
}

fn kinds(raw: &str) -> Result<Vec<String>, String> {
    let mut kinds = Vec::new();
    for kind in raw.split(',').map(str::trim).filter(|kind| !kind.is_empty()) {
        if !is_kind(kind) {
            return Err(format!("`{kind}` must be written `<module>.<kind>` in lower case"));
        }
        if !kinds.iter().any(|known| known == kind) {
            kinds.push(kind.to_owned());
        }
    }
    Ok(kinds)
}

fn overrides(raw: &str) -> Result<BTreeMap<(String, bool), Mode>, String> {
    let mut found = BTreeMap::new();
    for item in raw.split(',').map(str::trim).filter(|item| !item.is_empty()) {
        let parsed = item.split_once(':').and_then(|(kind, rest)| {
            let (who, mode) = rest.split_once('=')?;
            let guest = match who.trim() {
                "guest" => true,
                "user" => false,
                _ => return None,
            };
            Some((kind.trim().to_owned(), guest, Mode::parse(mode)?))
        });
        match parsed {
            Some((kind, guest, mode)) if is_kind(&kind) => {
                found.insert((kind, guest), mode);
            }
            _ => {
                return Err(format!(
                    "`{item}` must be written `<kind>:guest=<mode>` or `<kind>:user=<mode>` with \
                     a mode of `pre`, `first_time` or `post`"
                ));
            }
        }
    }
    Ok(found)
}

/// Danh sách từ / tên, đã `fold` (không phân biệt hoa thường, dấu).
fn words(raw: &str) -> Result<Vec<String>, String> {
    Ok(raw.split(',').map(crate::text::words_key).filter(|word| !word.trim().is_empty()).collect())
}

/// Địa chỉ http(s) có chỗ điền `{token}`.
fn token_url(raw: &str) -> Result<String, String> {
    let url = raw.trim();
    let lower = url.to_ascii_lowercase();
    if (lower.starts_with("https://") || lower.starts_with("http://")) && url.contains("{token}") {
        Ok(url.to_owned())
    } else {
        Err("must be an http(s) URL containing `{token}`".to_owned())
    }
}

/// Địa chỉ http(s) của website, bỏ `/` cuối.
fn site_url(raw: &str) -> Result<String, String> {
    let url = raw.trim().trim_end_matches('/');
    let lower = url.to_ascii_lowercase();
    let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"));
    match rest {
        Some(rest) if !rest.is_empty() && !rest.contains(['?', '#', ' ']) => Ok(url.to_owned()),
        _ => Err("must be an absolute `https://` (or `http://`) address like \
                  `https://www.example.com`"
            .to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::config::ConfigSource;

    fn read(pairs: &[(&str, &str)]) -> Result<CommentsConfig, String> {
        let source = ConfigSource::from_pairs(pairs.iter().copied());
        let mut reader = ConfigReader::new(&source);
        let config = CommentsConfig::read(&mut reader);
        reader.finish(config).map(|(config, _)| config).map_err(|errors| errors.to_string())
    }

    #[test]
    fn mac_dinh_chay_duoc_va_giong_default() {
        assert_eq!(read(&[]).unwrap(), CommentsConfig::default());
        assert!(!CommentsConfig::default().reply_mails());
    }

    #[test]
    fn gia_tri_sai_bi_bao_cung_luc() {
        let error = read(&[
            ("AUTOXIDE_COMMENTS_GUEST_MODERATION", "never"),
            ("AUTOXIDE_COMMENTS_TARGET_TYPES", "Entry"),
            ("AUTOXIDE_COMMENTS_VERIFY_URL", "https://example.com/verify"),
        ])
        .unwrap_err();
        for name in ["GUEST_MODERATION", "TARGET_TYPES", "VERIFY_URL"] {
            assert!(error.contains(name), "{error}");
        }
        let error =
            read(&[("AUTOXIDE_COMMENTS_MODERATION_BY_TYPE", "forms.form:guest=post")]).unwrap_err();
        assert!(error.contains("not in AUTOXIDE_COMMENTS_TARGET_TYPES"), "{error}");
    }

    #[test]
    fn che_do_theo_loai_dich() {
        let config = read(&[
            ("AUTOXIDE_COMMENTS_TARGET_TYPES", "content.entry, media.item"),
            ("AUTOXIDE_COMMENTS_MODERATION_BY_TYPE", "media.item:guest=post, media.item:user=pre"),
            ("AUTOXIDE_COMMENTS_BANNED_WORDS", "Mua Bán, , CASINO"),
        ])
        .unwrap();
        assert_eq!(config.mode("content.entry", true), Mode::Pre);
        assert_eq!(config.mode("content.entry", false), Mode::FirstTime);
        assert_eq!(config.mode("media.item", true), Mode::Post);
        assert_eq!(config.mode("media.item", false), Mode::Pre);
        assert_eq!(config.banned_words, ["mua ban", "casino"]);
    }
}
