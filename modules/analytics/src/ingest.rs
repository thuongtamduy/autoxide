//! Nhận sự kiện từ website (endpoint công khai) và từ event của module (phía máy chủ).
//!
//! Chống rác (không lỗi, chỉ bỏ — trang web không hỏng vì phân tích):
//! - user agent của bot đã biết, hay không có user agent → bỏ cả lô;
//! - trang gửi (`Origin`, không có thì `Referer`) không thuộc website: không phải tên miền của
//!   request, không phải tên miền đã xác minh của khách, không có trong
//!   `AUTOXIDE_ANALYTICS_ALLOWED_HOSTS` → bỏ cả lô;
//! - quá `AUTOXIDE_ANALYTICS_EVENTS_PER_MINUTE` sự kiện mỗi phút từ một khối mạng (đã băm) → bỏ
//!   phần thừa;
//! - loại sự kiện chưa ai khai, loại phía máy chủ, thuộc tính lạ / sai kiểu, đối tượng sai dạng →
//!   bỏ sự kiện đó.
//!
//! Phiên ẩn danh không cookie: khoá phiên = HMAC(khoá của ngày, website + khối mạng + user agent
//! rút gọn). Khoá của ngày là chuỗi ngẫu nhiên lưu ở `analytics_day_keys` và bị xoá khi ngày qua
//! (`privacy.rs`), nên không nối được phiên của hai ngày và không dò ngược ra IP. Không lưu IP,
//! không lưu user agent.

use autoxide_kernel::time::Timestamp;
use std::net::IpAddr;
use std::time::Duration;

use autoxide_kernel::analytics::{AnalyticsEvent, ServerHit};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Delivered;
use autoxide_kernel::http::ClientInfo;
use autoxide_kernel::tenancy::{OrgId, Site, TenantContext, root_of};
use autoxide_kernel::throttle::Throttle;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::Sha256;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::client::{host_of, is_bot, request_host, short_agent, without_www};
use crate::entity::{day_key, event};
use crate::service::{AnalyticsService, day_of};
use crate::shared::valid_kind;

/// Độ dài tối đa của đường dẫn trang.
const PATH_MAX: usize = 512;
/// Độ dài tối đa của giá trị UTM.
const UTM_MAX: usize = 100;
/// Độ dài tối đa của ID đối tượng.
const OBJECT_ID_MAX: usize = 100;

/// UTM parameters of the page URL.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub(crate) struct Utm {
    /// `utm_source`, e.g. `facebook`.
    pub source: Option<String>,
    /// `utm_medium`, e.g. `cpc`.
    pub medium: Option<String>,
    /// `utm_campaign`, e.g. `tet_sale`.
    pub campaign: Option<String>,
    pub term: Option<String>,
    pub content: Option<String>,
}

/// One event sent by a page.
//
// Chỉ để mô tả trong OpenAPI và đọc từng sự kiện: lô nhận `Vec<Value>` để một sự kiện sai dạng
// chỉ làm bỏ sự kiện đó, không làm hỏng cả lô.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub(crate) struct IncomingEvent {
    /// A declared event type, e.g. `analytics.page_view` or `catalog.product_viewed`; see
    /// `GET /api/v1/analytics/event-types`. Unknown types are dropped.
    #[serde(rename = "type")]
    #[schema(example = "analytics.page_view")]
    pub kind: String,
    /// Path of the page, starting with `/` (query and fragment are cut off; up to 512
    /// characters).
    #[schema(example = "/products/ao-dai")]
    pub path: String,
    /// `document.referrer`: only its host is kept.
    pub referrer: Option<String>,
    pub utm: Option<Utm>,
    /// What the event is about, `<kind>:<id>`, e.g. `catalog.product:0190…`. Required when the
    /// event type names an object kind.
    pub object: Option<String>,
    /// Extra values the event type declares; anything else drops the event.
    #[schema(value_type = Option<Object>)]
    pub props: Option<Map<String, Value>>,
}

/// What was done with a batch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct Ingested {
    /// Events stored.
    pub accepted: u32,
    /// Events dropped (unknown type, wrong props, bot, foreign page, too many).
    pub dropped: u32,
}

/// Bên gửi lô.
pub(crate) struct Visitor<'a> {
    pub client: &'a ClientInfo,
    /// Header `Host` của request.
    pub host: Option<&'a str>,
    /// Người dùng đã đăng nhập (nếu có).
    pub user_id: Option<Uuid>,
}

/// Một sự kiện đã kiểm, chưa có phiên.
struct Checked {
    kind: &'static str,
    path: String,
    referrer_host: Option<String>,
    source: String,
    landing: bool,
    utm: Utm,
    object: Option<String>,
    props: Option<String>,
}

impl AnalyticsService {
    /// Nhận một lô sự kiện của website `site`, coi `now` là bây giờ.
    pub(crate) async fn ingest(
        &self,
        tenant: &TenantContext,
        site: &Site,
        visitor: Visitor<'_>,
        events: Vec<Value>,
        now: Timestamp,
    ) -> Result<Ingested, Error> {
        let total = u32::try_from(events.len()).unwrap_or(u32::MAX);
        let dropped_all = Ingested { accepted: 0, dropped: total };
        if !self.config.enabled || events.is_empty() || is_bot(visitor.client.user_agent.as_deref())
        {
            return Ok(dropped_all);
        }
        let Some(page_host) = page_host(visitor.client) else {
            return Ok(dropped_all);
        };
        if !self.host_allowed(tenant, &page_host, visitor.host).await? {
            return Ok(dropped_all);
        }
        let mut checked: Vec<Checked> =
            events.into_iter().filter_map(|value| self.check(value, &page_host)).collect();
        let ip = visitor.client.ip;
        let block = match ip {
            Some(ip) => {
                autoxide_kernel::ip_hash::bucket(&self.secret, "analytics", ip, now).await?
            }
            None => "unknown".to_owned(),
        };
        if !checked.is_empty() {
            let wanted = u32::try_from(checked.len()).unwrap_or(u32::MAX);
            let granted =
                throttle(self.config.events_per_minute)?.take(tenant, &block, wanted).await?;
            checked.truncate(usize::try_from(granted.granted).unwrap_or(0));
        }
        // Trần mỗi ngày của khối mạng (PT-12): client ngoài trình duyệt giả được `Origin` và user agent, nên chỉ giới
        // hạn được lượng — một mạng làm lệch báo cáo tối đa chừng này sự kiện mỗi ngày.
        if !checked.is_empty() {
            let wanted = u32::try_from(checked.len()).unwrap_or(u32::MAX);
            let daily = Throttle::new("analytics", self.config.events_per_day, DAY)?;
            let granted = daily.take(tenant, &format!("day:{block}"), wanted).await?;
            checked.truncate(usize::try_from(granted.granted).unwrap_or(0));
        }
        if checked.is_empty() {
            return Ok(dropped_all);
        }
        let day = day_of(now);
        let key = self.day_key(tenant, &day, now).await?;
        let session =
            session_key(&key, site.org(), ip, &short_agent(visitor.client.user_agent.as_deref()))?;
        let user_id = match visitor.user_id {
            Some(user) if self.personal_allowed(tenant, user).await? => Some(user),
            _ => None,
        };
        let accepted = u32::try_from(checked.len()).unwrap_or(u32::MAX);
        let rows = checked.into_iter().map(|event| event::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            site_org_id: Set(site.org().as_uuid()),
            occurred_at: Set(now),
            day: Set(day.clone()),
            kind: Set(event.kind.to_owned()),
            session_key: Set(session.clone()),
            user_id: Set(user_id),
            path: Set(event.path),
            referrer_host: Set(event.referrer_host),
            source: Set(event.source),
            landing: Set(event.landing),
            utm_source: Set(event.utm.source),
            utm_medium: Set(event.utm.medium),
            utm_campaign: Set(event.utm.campaign),
            utm_term: Set(event.utm.term),
            utm_content: Set(event.utm.content),
            object: Set(event.object),
            props: Set(event.props),
        });
        event::Entity::insert_many(rows).exec_without_returning(tenant.database().write()).await?;
        Ok(Ingested { accepted, dropped: total - accepted })
    }

    /// Trang gửi thuộc website: cùng tên miền với request, tên miền khai trong cấu hình, hay tên
    /// miền đã xác minh của chính khách.
    async fn host_allowed(
        &self,
        tenant: &TenantContext,
        page_host: &str,
        request: Option<&str>,
    ) -> Result<bool, Error> {
        if request.is_some_and(|host| request_host(host) == page_host) {
            return Ok(true);
        }
        // Danh sách riêng của khách thay danh sách chung (đọc qua cache của kernel).
        let own = match &self.settings {
            Some(settings) => {
                autoxide_kernel::tenancy::parsed_setting(
                    settings.as_ref(),
                    tenant,
                    &crate::config::TENANT_ALLOWED_HOSTS,
                    crate::config::tenant_allowed_hosts,
                )
                .await?
            }
            None => None,
        };
        let listed = own.as_ref().unwrap_or(&self.config.allowed_hosts);
        if listed.iter().any(|host| host == page_host) {
            return Ok(true);
        }
        Ok(matches!(
            self.registry.domain_site(page_host).await?,
            Some((owner, _)) if owner == tenant.id()
        ))
    }

    /// Kiểm một sự kiện; `None`: bỏ.
    fn check(&self, value: Value, page_host: &str) -> Option<Checked> {
        let incoming: IncomingEvent = serde_json::from_value(value).ok()?;
        let declared = self.catalog.event(incoming.kind.trim())?;
        if !declared.from_browser() {
            return None;
        }
        let path = clean_path(&incoming.path)?;
        let object = check_object(declared, incoming.object.as_deref())?;
        let props = check_props(declared, incoming.props)?;
        let utm = clean_utm(incoming.utm.unwrap_or_default());
        let referrer = incoming.referrer.as_deref().and_then(host_of);
        let internal =
            referrer.as_deref().is_some_and(|host| without_www(host) == without_www(page_host));
        let referrer_host =
            if internal { None } else { referrer.map(|h| without_www(&h).to_owned()) };
        let source = match (&utm.source, &referrer_host) {
            (Some(source), _) => source.clone(),
            (None, Some(host)) => host.clone(),
            (None, None) => "direct".to_owned(),
        };
        Some(Checked {
            kind: declared.key,
            path,
            referrer_host,
            source,
            landing: !internal,
            utm,
            object,
            props,
        })
    }

    /// Khoá ngẫu nhiên của ngày `day` cho khách; chưa có thì tạo. Hai request cùng lúc: một dòng
    /// thắng (khoá chính), cả hai đọc lại dòng đó.
    pub(crate) async fn day_key(
        &self,
        tenant: &TenantContext,
        day: &str,
        now: Timestamp,
    ) -> Result<String, Error> {
        let find = || {
            day_key::Entity::find()
                .filter(day_key::Column::TenantId.eq(tenant.id().as_uuid()))
                .filter(day_key::Column::Day.eq(day))
        };
        if let Some(found) = find().one(tenant.database().read()).await? {
            return Ok(found.key);
        }
        day_key::Entity::insert(day_key::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            day: Set(day.to_owned()),
            key: Set(autoxide_kernel::secret::random_token(32)?),
            created_at: Set(now),
        })
        .on_conflict(
            OnConflict::columns([day_key::Column::TenantId, day_key::Column::Day])
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(tenant.database().write())
        .await?;
        // Đọc qua kết nối ghi: bản đọc (read replica) có thể chưa thấy dòng vừa ghi.
        find()
            .one(tenant.database().write())
            .await?
            .map(|found| found.key)
            .ok_or_else(|| Error::internal("the analytics key of the day vanished"))
    }

    /// Ghi sự kiện phía máy chủ rút từ `event` (bên nhận event: chịu được nhận lại).
    pub(crate) async fn record_server_hit(
        &self,
        declared: &AnalyticsEvent,
        delivered: &Delivered,
        hit: ServerHit,
    ) -> Result<(), Error> {
        let tenant = &delivered.tenant;
        // Mỗi event một "phiên" riêng, bắt đầu bằng `~` (không có trong bảng chữ base64url của khoá
        // phiên trình duyệt): đếm phễu theo số event, không lẫn với khách xem trang.
        let session = format!("~{}", &delivered.id.simple().to_string()[..31]);
        let exists = event::Entity::find()
            .filter(event::Column::TenantId.eq(tenant.id().as_uuid()))
            .filter(event::Column::SessionKey.eq(session.as_str()))
            .filter(event::Column::Kind.eq(declared.key))
            .one(tenant.database().write())
            .await?
            .is_some();
        if exists {
            return Ok(());
        }
        let object = match (declared.object_kind, hit.object) {
            (Some(kind), Some(object)) => valid_object(kind, &object).then_some(object),
            _ => None,
        };
        let user_id = match hit.user_id {
            Some(user) if self.personal_allowed(tenant, user).await? => Some(user),
            _ => None,
        };
        let site = hit.org_id.unwrap_or_else(|| root_of(tenant.id()));
        event::Entity::insert(event::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            site_org_id: Set(site.as_uuid()),
            occurred_at: Set(hit.occurred_at),
            day: Set(day_of(hit.occurred_at)),
            kind: Set(declared.key.to_owned()),
            session_key: Set(session),
            user_id: Set(user_id),
            path: Set(String::new()),
            referrer_host: Set(None),
            source: Set(String::new()),
            landing: Set(false),
            utm_source: Set(None),
            utm_medium: Set(None),
            utm_campaign: Set(None),
            utm_term: Set(None),
            utm_content: Set(None),
            object: Set(object),
            props: Set(None),
        })
        .exec_without_returning(tenant.database().write())
        .await?;
        Ok(())
    }
}

/// Cửa sổ của trần mỗi ngày.
const DAY: Duration = Duration::from_secs(24 * 60 * 60);

fn throttle(per_minute: u32) -> Result<Throttle, Error> {
    Throttle::new("analytics", per_minute, Duration::from_secs(60))
}

/// Tên miền của trang gửi: `Origin`, không có thì `Referer`.
fn page_host(client: &ClientInfo) -> Option<String> {
    client
        .origin
        .as_deref()
        .and_then(host_of)
        .or_else(|| client.referer.as_deref().and_then(host_of))
}

/// Khoá phiên: HMAC-SHA256(khoá của ngày, website | khối mạng | user agent rút gọn), 16 byte đầu,
/// base64url (22 ký tự).
pub(crate) fn session_key(
    day_key: &str,
    site: OrgId,
    ip: Option<IpAddr>,
    agent: &str,
) -> Result<String, Error> {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(day_key.as_bytes())
        .map_err(|error| Error::internal(format!("hmac key: {error}")))?;
    let block = ip.map(|ip| autoxide_kernel::ip_hash::prefix(ip).to_string()).unwrap_or_default();
    mac.update(format!("{}|{block}|{agent}", site.as_uuid()).as_bytes());
    Ok(URL_SAFE_NO_PAD.encode(&mac.finalize().into_bytes()[..16]))
}

/// Đường dẫn trang: bắt đầu bằng `/`, bỏ query và fragment (có thể chứa dữ liệu cá nhân).
fn clean_path(raw: &str) -> Option<String> {
    let path = raw.trim().split(['?', '#']).next().unwrap_or_default();
    (path.starts_with('/')
        && path.chars().count() <= PATH_MAX
        && !path.chars().any(char::is_control))
    .then(|| path.to_owned())
}

fn clean_utm(utm: Utm) -> Utm {
    let clean = |value: Option<String>| {
        value
            .map(|text| text.trim().to_lowercase().chars().take(UTM_MAX).collect::<String>())
            .filter(|text| !text.is_empty() && !text.chars().any(char::is_control))
    };
    Utm {
        source: clean(utm.source),
        medium: clean(utm.medium),
        campaign: clean(utm.campaign),
        term: clean(utm.term),
        content: clean(utm.content),
    }
}

/// Đối tượng của sự kiện. `None`: bỏ sự kiện; `Some(None)`: sự kiện không có đối tượng.
fn check_object(declared: &AnalyticsEvent, object: Option<&str>) -> Option<Option<String>> {
    let object = object.map(str::trim).filter(|object| !object.is_empty());
    match (declared.object_kind, object) {
        (Some(kind), Some(object)) => valid_object(kind, object).then(|| Some(object.to_owned())),
        (Some(_), None) => None,
        (None, Some(object)) => {
            let kind = object.split_once(':').map(|(kind, _)| kind).unwrap_or_default();
            valid_kind(kind).then_some(())?;
            valid_object(kind, object).then(|| Some(object.to_owned()))
        }
        (None, None) => Some(None),
    }
}

/// `object` là `<kind>:<id>`, ID 1–100 ký tự chữ, số, `-`, `_`, `.`.
fn valid_object(kind: &str, object: &str) -> bool {
    object.strip_prefix(kind).and_then(|rest| rest.strip_prefix(':')).is_some_and(|id| {
        !id.is_empty()
            && id.len() <= OBJECT_ID_MAX
            && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    })
}

/// Thuộc tính đã khai, đúng kiểu → JSON để lưu (`Some(None)`: không có). Lạ hay sai → bỏ sự kiện.
fn check_props(
    declared: &AnalyticsEvent,
    props: Option<Map<String, Value>>,
) -> Option<Option<String>> {
    let Some(props) = props.filter(|props| !props.is_empty()) else {
        return Some(None);
    };
    for (name, value) in &props {
        let prop = declared.props.iter().find(|prop| prop.name == name)?;
        if !prop.kind.accepts(value) {
            return None;
        }
    }
    Some(Some(Value::Object(props).to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::analytics::{Prop, PropKind};

    const VIEWED: AnalyticsEvent = AnalyticsEvent::browser("catalog.product_viewed", "Viewed.")
        .object_kind("catalog.product")
        .props(&[Prop::new("variant", PropKind::Text)]);
    const PAGE: AnalyticsEvent = AnalyticsEvent::browser("analytics.page_view", "Page.");

    #[test]
    fn duong_dan_bo_query() {
        assert_eq!(clean_path("/a/b?email=x@y#top").as_deref(), Some("/a/b"));
        assert_eq!(clean_path("a/b"), None);
        assert_eq!(clean_path(&format!("/{}", "a".repeat(600))), None);
    }

    #[test]
    fn doi_tuong_va_thuoc_tinh() {
        assert_eq!(
            check_object(&VIEWED, Some("catalog.product:abc-1")),
            Some(Some("catalog.product:abc-1".to_owned()))
        );
        assert_eq!(check_object(&VIEWED, Some("content.entry:abc")), None);
        assert_eq!(check_object(&VIEWED, None), None);
        assert_eq!(check_object(&PAGE, None), Some(None));
        assert_eq!(
            check_object(&PAGE, Some("content.entry:9")),
            Some(Some("content.entry:9".to_owned()))
        );
        assert_eq!(check_object(&PAGE, Some("bad kind:9")), None);
        let props = |value: Value| value.as_object().cloned();
        assert!(
            check_props(&VIEWED, props(serde_json::json!({"variant": "red"}))).unwrap().is_some()
        );
        assert_eq!(check_props(&VIEWED, props(serde_json::json!({"variant": 1}))), None);
        assert_eq!(check_props(&VIEWED, props(serde_json::json!({"other": "x"}))), None);
        assert_eq!(check_props(&PAGE, None), Some(None));
    }

    #[test]
    fn khoa_phien_doi_theo_khoa_ngay_va_website() {
        let site = OrgId::from_uuid(Uuid::nil());
        let ip: IpAddr = "203.0.113.7".parse().unwrap();
        let same_block: IpAddr = "203.0.113.99".parse().unwrap();
        let a = session_key("key-1", site, Some(ip), "chrome|android|mobile").unwrap();
        assert_eq!(a.len(), 22);
        assert_eq!(
            a,
            session_key("key-1", site, Some(same_block), "chrome|android|mobile").unwrap()
        );
        assert_ne!(a, session_key("key-2", site, Some(ip), "chrome|android|mobile").unwrap());
        let other = OrgId::from_uuid(Uuid::now_v7());
        assert_ne!(a, session_key("key-1", other, Some(ip), "chrome|android|mobile").unwrap());
    }
}
