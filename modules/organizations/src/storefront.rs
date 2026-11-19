//! Thông tin cửa hàng công khai (bước 55 mục 4, D-237, D-20261119-step55-store-info).
//!
//! - **Thương hiệu** là cài đặt kế thừa theo cây (chuỗi đặt, cửa hàng ghi đè): tên, logo, biểu
//!   tượng trang, điện thoại, email, mạng xã hội. Sửa qua endpoint cài đặt sẵn có
//!   (`PUT /units/{id}/settings/{key}`), không thêm bảng.
//! - **Địa điểm** (địa chỉ, giờ mở cửa, ngày đặc biệt) lấy từ hồ sơ công khai của nút (bước 34),
//!   không lưu hai chỗ.
//! - "Đang mở cửa" tính theo múi giờ của nút (`organizations.timezone`): ngày đặc biệt thắng giờ
//!   thường; giờ UTC và giờ địa phương khác ngày (qua nửa đêm) vẫn đúng ngày địa phương.

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{OrgSetting, Site, TenantContext};
use chrono::{DateTime, Datelike, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::EffectiveSetting;
use crate::profile::{OpeningHours, Profile, SpecialDay};
use crate::service::OrgService;

/// Mạng xã hội nhận trong `organizations.social_links`.
const NETWORKS: [&str; 9] =
    ["facebook", "instagram", "tiktok", "youtube", "zalo", "x", "linkedin", "shopee", "other"];

pub(crate) const BRAND_NAME: OrgSetting = OrgSetting::new(
    "organizations.brand_name",
    "Brand name shown on the website (header, title, footer), up to 120 characters. Unset: the \
     unit's public display name. A chain sets it once; a store may override it.",
)
.with_schema(r#"{"type":"string","minLength":1,"maxLength":120}"#)
.checked(brand_name);

pub(crate) const BRAND_LOGO: OrgSetting = OrgSetting::new(
    "organizations.brand_logo",
    "Logo of the brand: the id of a public picture in the media library.",
)
.with_schema(r#"{"type":"string","format":"uuid"}"#)
.checked(media_id);

pub(crate) const BRAND_ICON: OrgSetting = OrgSetting::new(
    "organizations.brand_icon",
    "Site icon (favicon) of the brand: the id of a public square picture in the media library.",
)
.with_schema(r#"{"type":"string","format":"uuid"}"#)
.checked(media_id);

pub(crate) const CONTACT_PHONE: OrgSetting = OrgSetting::new(
    "organizations.contact_phone",
    "Phone number visitors call, e.g. `+84 28 1234 5678` (digits, spaces, `+`, `-`, `.`, `(`, `)`).",
)
.with_schema(r#"{"type":"string","maxLength":30}"#)
.checked(phone);

pub(crate) const CONTACT_EMAIL: OrgSetting =
    OrgSetting::new("organizations.contact_email", "Email address visitors write to.")
        .with_schema(r#"{"type":"string","format":"email","maxLength":254}"#)
        .checked(email);

pub(crate) const SOCIAL_LINKS: OrgSetting = OrgSetting::new(
    "organizations.social_links",
    "Social profiles of the brand, at most 10: `[{\"network\": \"facebook\", \"url\": \
     \"https://…\"}]`; network is facebook, instagram, tiktok, youtube, zalo, x, linkedin, shopee \
     or other.",
)
.with_schema(
    r#"{"type":"array","maxItems":10,"items":{"type":"object","required":["network","url"],"properties":{"network":{"type":"string"},"url":{"type":"string","format":"uri"}}}}"#,
)
.checked(social_links);

pub(crate) fn settings() -> Vec<OrgSetting> {
    vec![BRAND_NAME, BRAND_LOGO, BRAND_ICON, CONTACT_PHONE, CONTACT_EMAIL, SOCIAL_LINKS]
}

fn brand_name(value: &Value) -> Result<Value, String> {
    let name = value.as_str().map(str::trim).unwrap_or_default();
    if name.is_empty() || name.chars().count() > 120 {
        return Err("must be 1 to 120 characters".to_owned());
    }
    Ok(Value::from(name))
}

fn media_id(value: &Value) -> Result<Value, String> {
    value
        .as_str()
        .and_then(|raw| raw.trim().parse::<Uuid>().ok())
        .map(|id| Value::from(id.to_string()))
        .ok_or_else(|| "must be the id (UUID) of a media item".to_owned())
}

fn phone(value: &Value) -> Result<Value, String> {
    let raw = value.as_str().map(str::trim).unwrap_or_default();
    let digits = raw.chars().filter(char::is_ascii_digit).count();
    let allowed = raw.chars().all(|c| c.is_ascii_digit() || " +-.()".contains(c));
    if !allowed || !(6..=15).contains(&digits) || raw.len() > 30 {
        return Err("must be a phone number of 6 to 15 digits".to_owned());
    }
    Ok(Value::from(raw))
}

fn email(value: &Value) -> Result<Value, String> {
    let raw = value.as_str().map(str::trim).unwrap_or_default();
    let valid = raw.len() <= 254
        && raw.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && domain.contains('.') && !domain.starts_with('.')
        })
        && !raw.chars().any(char::is_whitespace);
    if !valid {
        return Err("must be an email address".to_owned());
    }
    Ok(Value::from(raw.to_ascii_lowercase()))
}

fn social_links(value: &Value) -> Result<Value, String> {
    let wrong = || {
        "must be at most 10 items `{network, url}` with a known network and an `https://` url"
            .to_owned()
    };
    let items = value.as_array().filter(|items| items.len() <= 10).ok_or_else(wrong)?;
    let mut out = Vec::new();
    for item in items {
        let network = item.get("network").and_then(Value::as_str).ok_or_else(wrong)?;
        let url = item.get("url").and_then(Value::as_str).map(str::trim).ok_or_else(wrong)?;
        let https = url.to_ascii_lowercase().starts_with("https://")
            && url.len() <= 2048
            && !url.chars().any(|c| c.is_whitespace() || c.is_control());
        if !NETWORKS.contains(&network) || !https {
            return Err(wrong());
        }
        out.push(serde_json::json!({"network": network, "url": url}));
    }
    Ok(Value::Array(out))
}

/// A social profile of the brand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub(crate) struct SocialLink {
    #[schema(example = "facebook")]
    pub network: String,
    pub url: String,
}

/// Thương hiệu đã kế thừa.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Brand {
    pub name: String,
    pub logo_media_id: Option<Uuid>,
    pub icon_media_id: Option<Uuid>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub social_links: Vec<SocialLink>,
}

/// Thông tin cửa hàng của một website.
#[derive(Debug, Clone)]
pub(crate) struct Storefront {
    pub profile: Profile,
    pub brand: Brand,
    pub timezone: String,
    pub open_now: Option<bool>,
}

fn value_of(settings: &[EffectiveSetting], key: &str) -> Option<Value> {
    settings.iter().find(|setting| setting.key == key).and_then(|setting| setting.value.clone())
}

fn text_of(settings: &[EffectiveSetting], key: &str) -> Option<String> {
    value_of(settings, key).and_then(|value| value.as_str().map(str::to_owned))
}

impl OrgService {
    /// Thông tin cửa hàng của website `site` lúc `now`: thương hiệu đã kế thừa và hồ sơ địa điểm.
    pub(crate) async fn public_storefront(
        &self,
        tenant: &TenantContext,
        site: &Site,
        now: DateTime<Utc>,
    ) -> Result<Storefront, Error> {
        let profile = self.site_profile(tenant, site).await?;
        let settings = self.resolve_settings(tenant, site.org(), None).await?;
        let timezone =
            text_of(&settings, crate::settings::TIMEZONE.key()).unwrap_or_else(|| "UTC".to_owned());
        let brand = Brand {
            name: text_of(&settings, BRAND_NAME.key())
                .unwrap_or_else(|| profile.display_name.clone()),
            logo_media_id: text_of(&settings, BRAND_LOGO.key()).and_then(|id| id.parse().ok()),
            icon_media_id: text_of(&settings, BRAND_ICON.key()).and_then(|id| id.parse().ok()),
            phone: text_of(&settings, CONTACT_PHONE.key()),
            email: text_of(&settings, CONTACT_EMAIL.key()),
            social_links: value_of(&settings, SOCIAL_LINKS.key())
                .and_then(|value| serde_json::from_value(value).ok())
                .unwrap_or_default(),
        };
        let open_now = open_at(&profile.opening_hours, &profile.special_days, &timezone, now);
        Ok(Storefront { profile, brand, timezone, open_now })
    }
}

/// Cửa hàng có mở lúc `now` không, theo giờ địa phương của múi `timezone`. `None` khi chưa khai giờ
/// mở cửa (không biết). Ngày đặc biệt của ngày địa phương thắng giờ thường.
pub(crate) fn open_at(
    hours: &[OpeningHours],
    special: &[SpecialDay],
    timezone: &str,
    now: DateTime<Utc>,
) -> Option<bool> {
    let zone = timezone.parse::<chrono_tz::Tz>().unwrap_or(chrono_tz::UTC);
    let local = now.with_timezone(&zone);
    let date = local.date_naive().format("%Y-%m-%d").to_string();
    let time = local.time();
    let parse = |raw: &str| NaiveTime::parse_from_str(raw, "%H:%M").ok();
    let within = |opens: Option<NaiveTime>, closes: Option<NaiveTime>| matches!((opens, closes), (Some(opens), Some(closes)) if opens <= time && time < closes);
    if let Some(day) = special.iter().find(|day| day.date == date) {
        return Some(
            !day.closed
                && within(
                    day.opens.as_deref().and_then(parse),
                    day.closes.as_deref().and_then(parse),
                ),
        );
    }
    if hours.is_empty() {
        return None;
    }
    let weekday = u8::try_from(local.weekday().number_from_monday()).unwrap_or(0);
    Some(
        hours
            .iter()
            .filter(|range| range.day == weekday)
            .any(|range| within(parse(&range.opens), parse(&range.closes))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn range(day: u8, opens: &str, closes: &str) -> OpeningHours {
        OpeningHours { day, opens: opens.into(), closes: closes.into() }
    }

    #[test]
    fn dang_mo_cua_theo_mui_gio_cua_nut_ke_ca_qua_nua_dem() {
        // Thứ Hai 2026-11-16 18:30 UTC = thứ Ba 01:30 ở Việt Nam.
        let now = Utc.with_ymd_and_hms(2026, 11, 16, 18, 30, 0).unwrap();
        let hours = [range(1, "08:00", "18:00"), range(2, "00:00", "02:00")];
        assert_eq!(open_at(&hours, &[], "Asia/Ho_Chi_Minh", now), Some(true));
        assert_eq!(open_at(&hours, &[], "UTC", now), Some(false), "18:30 thứ Hai: đã đóng");
        let later = now + chrono::Duration::hours(1);
        assert_eq!(open_at(&hours, &[], "Asia/Ho_Chi_Minh", later), Some(false));
        assert_eq!(open_at(&[], &[], "UTC", now), None, "chưa khai giờ: không biết");
    }

    #[test]
    fn ngay_dac_biet_thang_gio_thuong() {
        let now = Utc.with_ymd_and_hms(2026, 11, 17, 3, 0, 0).unwrap(); // 10:00 thứ Ba ở Việt Nam
        let hours = [range(2, "08:00", "22:00")];
        let closed = SpecialDay {
            date: "2026-11-17".into(),
            closed: true,
            opens: None,
            closes: None,
            note: None,
        };
        assert_eq!(open_at(&hours, &[closed], "Asia/Ho_Chi_Minh", now), Some(false));
        let short = SpecialDay {
            date: "2026-11-17".into(),
            closed: false,
            opens: Some("09:00".into()),
            closes: Some("11:00".into()),
            note: None,
        };
        assert_eq!(open_at(&[], &[short], "Asia/Ho_Chi_Minh", now), Some(true));
    }

    #[test]
    fn kiem_gia_tri_thuong_hieu() {
        assert!(phone(&Value::from("+84 28 1234 5678")).is_ok());
        assert!(phone(&Value::from("call me")).is_err());
        assert_eq!(email(&Value::from(" Shop@Example.VN ")).unwrap(), "shop@example.vn");
        assert!(email(&Value::from("nope")).is_err());
        let links = serde_json::json!([{"network": "facebook", "url": "https://fb.com/shop"}]);
        assert!(social_links(&links).is_ok());
        let bad = serde_json::json!([{"network": "myspace", "url": "https://x"}]);
        assert!(social_links(&bad).is_err());
        let http = serde_json::json!([{"network": "x", "url": "http://x.com"}]);
        assert!(social_links(&http).is_err());
        assert!(brand_name(&Value::from("")).is_err());
        assert!(media_id(&Value::from("not-a-uuid")).is_err());
    }
}
