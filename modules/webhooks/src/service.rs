//! Service của webhook: quản lý nơi nhận (endpoint) và xem các lần gửi. Handler REST
//! chỉ gọi vào đây.

use std::collections::HashMap;

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, EventCatalog, Outbox};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QuerySelect, Set,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::delivery::{MAX_ATTEMPTS, STATUS_FAILED, STATUS_PENDING, STATUS_SUCCEEDED, Send};
use crate::entity::{delivery, endpoint};
use crate::events::{self, Change};
use crate::signature;
use autoxide_kernel::job::{EnqueueOptions, JobQueue};

/// Tên event đặc biệt: nhận mọi event.
pub(crate) const ALL_EVENTS: &str = "*";

/// Số event tối đa một endpoint đăng ký (danh sách tên rõ ràng).
const MAX_EVENTS: usize = 100;

/// Độ dài tối đa của địa chỉ và mô tả, theo ký tự.
const URL_MAX_LENGTH: usize = 2048;
const DESCRIPTION_MAX_LENGTH: usize = 200;

/// Một nơi nhận, không kèm secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Endpoint {
    pub id: Uuid,
    pub url: String,
    pub description: String,
    pub events: Vec<String>,
    pub disabled_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Endpoint {
    pub(crate) fn from_row(row: endpoint::Model) -> Result<Self, Error> {
        Ok(Endpoint {
            events: parse_events(&row.events)?,
            id: row.id,
            url: row.url,
            description: row.description,
            disabled_at: row.disabled_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    /// Endpoint có nhận event tên `name` không.
    pub(crate) fn wants(&self, name: &str) -> bool {
        self.disabled_at.is_none()
            && self.events.iter().any(|event| event == ALL_EVENTS || event == name)
    }
}

pub(crate) fn parse_events(raw: &str) -> Result<Vec<String>, Error> {
    serde_json::from_str(raw)
        .map_err(|error| Error::internal(format!("webhook events are not a JSON list: {error}")))
}

/// Endpoint mới, kèm secret chỉ trả MỘT lần.
pub(crate) struct CreatedEndpoint {
    pub endpoint: Endpoint,
    pub secret: String,
}

// Tự viết `Debug`: không in secret ra log.
impl std::fmt::Debug for CreatedEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreatedEndpoint").field("endpoint", &self.endpoint).finish_non_exhaustive()
    }
}

pub(crate) struct NewEndpoint {
    pub url: String,
    pub description: String,
    pub events: Vec<String>,
}

/// Thay đổi cho một endpoint; trường `None` giữ nguyên.
#[derive(Default)]
pub(crate) struct EndpointChanges {
    pub url: Option<String>,
    pub description: Option<String>,
    pub events: Option<Vec<String>>,
    pub disabled: Option<bool>,
}

/// Một lần gửi một event tới một endpoint.
pub(crate) type Delivery = delivery::Model;

/// Mục đích của secret ký khi mã hoá (D-257): giá trị chép sang dòng hay khách khác không mở được.
pub(crate) const SECRET_PURPOSE: &str = "webhooks.endpoint_secret";

// Service không giữ database: database là của khách, đi theo `TenantContext` của từng lời gọi.
pub(crate) struct WebhookService {
    secrets: autoxide_kernel::secret::SecretKey,
    outbox: Outbox,
    catalog: EventCatalog,
    allow_http: bool,
    allow_private_networks: bool,
    /// Bản dịch mô tả event (D-264); mặc định rỗng (tiếng Anh).
    translations: autoxide_kernel::i18n::Translations,
}

impl WebhookService {
    pub(crate) fn new(
        secrets: autoxide_kernel::secret::SecretKey,
        outbox: Outbox,
        catalog: EventCatalog,
        allow_http: bool,
        allow_private_networks: bool,
    ) -> Self {
        WebhookService {
            secrets,
            outbox,
            catalog,
            allow_http,
            allow_private_networks,
            translations: Default::default(),
        }
    }

    /// Dịch mô tả event bằng `translations`.
    pub(crate) fn with_translations(
        mut self,
        translations: autoxide_kernel::i18n::Translations,
    ) -> Self {
        self.translations = translations;
        self
    }

    /// Mô tả event `english` bằng `language`.
    pub(crate) fn describe(
        &self,
        english: &str,
        language: autoxide_kernel::i18n::Language,
    ) -> String {
        self.translations.describe(english, language)
    }

    /// Mọi loại event có thể đăng ký, theo tên.
    pub(crate) fn event_catalog(&self) -> &EventCatalog {
        &self.catalog
    }

    pub(crate) async fn create(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        input: NewEndpoint,
    ) -> Result<CreatedEndpoint, Error> {
        let (url, events) =
            self.check(Some(&input.url), Some(&input.description), Some(input.events))?;
        let (url, events) = (url.unwrap_or_default(), events.unwrap_or_default());
        let secret = signature::new_secret()?;
        let now = now();
        let id = Uuid::now_v7();
        // Lưu secret đã mã hoá (D-257): bản sao database không đủ để giả chữ ký.
        let sealed = self.seal(tenant, id, &secret).await?;
        let transaction = tenant.database().begin_write().await?;
        let row = endpoint::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            url: Set(url),
            description: Set(input.description.trim().to_owned()),
            secret: Set(sealed),
            events: Set(Value::from(events).to_string()),
            disabled_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&transaction)
        .await?;
        let endpoint = Endpoint::from_row(row)?;
        let event =
            events::EndpointCreated(Change { before: None, after: Some(snapshot(&endpoint)) });
        self.outbox
            .record(&transaction, tenant, actor, events::endpoint_subject(endpoint.id), &event)
            .await?;
        transaction.commit().await?;
        tracing::info!(endpoint_id = %endpoint.id, "webhook endpoint created");
        Ok(CreatedEndpoint { endpoint, secret })
    }

    /// Lần gửi mới nhất của từng endpoint trong `ids` — hai câu truy vấn cho cả trang (đề bài
    /// giao diện B3): thời điểm tạo mới nhất theo endpoint, rồi các dòng đúng thời điểm đó.
    pub(crate) async fn last_deliveries(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, delivery::Model>, Error> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let read = tenant.database().read();
        let latest: Vec<(Uuid, chrono::DateTime<chrono::Utc>)> = delivery::Entity::find()
            .select_only()
            .column(delivery::Column::EndpointId)
            .column_as(delivery::Column::CreatedAt.max(), "latest")
            .filter(delivery::Column::EndpointId.is_in(ids.iter().copied()))
            .scoped(tenant)
            .group_by(delivery::Column::EndpointId)
            .into_tuple()
            .all(read)
            .await?;
        if latest.is_empty() {
            return Ok(HashMap::new());
        }
        let wanted: HashMap<Uuid, chrono::DateTime<chrono::Utc>> = latest.into_iter().collect();
        let rows = delivery::Entity::find()
            .filter(delivery::Column::EndpointId.is_in(wanted.keys().copied()))
            .filter(delivery::Column::CreatedAt.is_in(wanted.values().copied()))
            .scoped(tenant)
            .all(read)
            .await?;
        let mut found = HashMap::new();
        for row in rows {
            if wanted.get(&row.endpoint_id) == Some(&row.created_at) {
                found.entry(row.endpoint_id).or_insert(row);
            }
        }
        Ok(found)
    }

    /// Một trang endpoint, mới tạo trước (mặc định); `status`: chỉ endpoint đang nhận
    /// (`Some(true)`) hay đã tắt (`Some(false)`).
    pub(crate) async fn list(
        &self,
        tenant: &TenantContext,
        active: Option<bool>,
        page: &PageRequest,
    ) -> Result<Page<Endpoint>, Error> {
        let select = match active {
            Some(true) => endpoint::Entity::find().filter(endpoint::Column::DisabledAt.is_null()),
            Some(false) => {
                endpoint::Entity::find().filter(endpoint::Column::DisabledAt.is_not_null())
            }
            None => endpoint::Entity::find(),
        };
        let read = tenant.database().read();
        let rows =
            fetch_page(select.scoped(tenant), endpoint::Column::Id, |row| row.id, page, read)
                .await?;
        let mut endpoints = Vec::with_capacity(rows.data.len());
        for row in rows.data {
            endpoints.push(Endpoint::from_row(row)?);
        }
        Ok(Page { data: endpoints, pagination: rows.pagination })
    }

    pub(crate) async fn get(&self, tenant: &TenantContext, id: Uuid) -> Result<Endpoint, Error> {
        let read = tenant.database().read();
        let row = endpoint::Entity::find_by_id(id).scoped(tenant).one(read).await?;
        Endpoint::from_row(row.ok_or_else(|| endpoint_not_found(id))?)
    }

    /// Đổi địa chỉ, mô tả, danh sách event hoặc bật / tắt endpoint `id`.
    pub(crate) async fn update(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
        changes: EndpointChanges,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Endpoint, Error> {
        let (url, events) =
            self.check(changes.url.as_deref(), changes.description.as_deref(), changes.events)?;
        let transaction = tenant.database().begin_write().await?;
        let row = endpoint::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| endpoint_not_found(id))?;
        let before = Endpoint::from_row(row.clone())?;
        let now = now();
        let mut active = row.into_active_model();
        if let Some(url) = url {
            active.url = Set(url);
        }
        if let Some(description) = changes.description {
            active.description = Set(description.trim().to_owned());
        }
        if let Some(events) = events {
            active.events = Set(Value::from(events).to_string());
        }
        match changes.disabled {
            Some(true) if before.disabled_at.is_none() => active.disabled_at = Set(Some(now)),
            Some(false) => active.disabled_at = Set(None),
            _ => {}
        }
        active.updated_at = Set(now);
        let mut update = endpoint::Entity::update(active)
            .validate()?
            .filter(endpoint::Column::TenantId.eq(tenant.id().as_uuid()));
        if let Some(expected) = expected {
            // Kiểm phiên bản ngay trong câu `UPDATE` (D-084).
            update = update.filter(endpoint::Column::UpdatedAt.eq(expected));
        }
        let updated = update.exec(&transaction).await.map_err(|error| match error {
            sea_orm::DbErr::RecordNotUpdated => autoxide_kernel::version::changed(),
            error => Error::from(error),
        })?;
        let after = Endpoint::from_row(updated)?;
        let change = Change { before: Some(snapshot(&before)), after: Some(snapshot(&after)) };
        let event = events::EndpointUpdated(change);
        self.outbox
            .record(&transaction, tenant, actor, events::endpoint_subject(id), &event)
            .await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Thay secret của endpoint `id`; secret cũ hết hiệu lực ngay. Trả secret mới (chỉ
    /// lần này).
    pub(crate) async fn rotate_secret(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
    ) -> Result<String, Error> {
        let secret = signature::new_secret()?;
        let sealed = self.seal(tenant, id, &secret).await?;
        let transaction = tenant.database().begin_write().await?;
        let updated = endpoint::Entity::update_many()
            .col_expr(endpoint::Column::Secret, sea_orm::sea_query::Expr::value(sealed))
            .col_expr(endpoint::Column::UpdatedAt, sea_orm::sea_query::Expr::value(now()))
            .filter(endpoint::Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if updated.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(endpoint_not_found(id));
        }
        let event = events::EndpointUpdated(Change {
            before: None,
            after: Some(json!({"secret_replaced": true})),
        });
        self.outbox
            .record(&transaction, tenant, actor, events::endpoint_subject(id), &event)
            .await?;
        transaction.commit().await?;
        tracing::info!(endpoint_id = %id, "webhook secret replaced");
        Ok(secret)
    }

    /// Xoá endpoint `id` cùng lịch sử gửi của nó; lần gửi còn chờ bị bỏ.
    pub(crate) async fn delete(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let row = endpoint::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| endpoint_not_found(id))?;
        let deleted = endpoint::Entity::delete_by_id(id).scoped(tenant).exec(&transaction).await?;
        if deleted.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(endpoint_not_found(id));
        }
        let before = Endpoint::from_row(row)?;
        let event =
            events::EndpointDeleted(Change { before: Some(snapshot(&before)), after: None });
        self.outbox
            .record(&transaction, tenant, actor, events::endpoint_subject(id), &event)
            .await?;
        transaction.commit().await?;
        tracing::info!(endpoint_id = %id, "webhook endpoint deleted");
        Ok(())
    }

    /// Một trang lần gửi của endpoint `id`, mới trước (mặc định).
    pub(crate) async fn deliveries(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<Delivery>, Error> {
        self.get(tenant, id).await?;
        let select =
            delivery::Entity::find().filter(delivery::Column::EndpointId.eq(id)).scoped(tenant);
        fetch_page(select, delivery::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Một trang lần gửi của cả khách (bàn kỹ thuật, bước 52), lọc theo trạng thái và endpoint.
    pub(crate) async fn all_deliveries(
        &self,
        tenant: &TenantContext,
        status: Option<&str>,
        endpoint_id: Option<Uuid>,
        page: &PageRequest,
    ) -> Result<Page<Delivery>, Error> {
        let mut select = delivery::Entity::find().scoped(tenant);
        if let Some(status) = status {
            if ![STATUS_PENDING, STATUS_SUCCEEDED, STATUS_FAILED].contains(&status) {
                return Err(Error::validation(vec![FieldError::new(
                    "status",
                    "invalid",
                    "must be `pending`, `succeeded` or `failed`",
                )]));
            }
            select = select.filter(delivery::Column::Status.eq(status));
        }
        if let Some(endpoint_id) = endpoint_id {
            select = select.filter(delivery::Column::EndpointId.eq(endpoint_id));
        }
        fetch_page(select, delivery::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Gửi lại lần gửi `id` (đã xong hay đã hỏng hẳn): mở lại thành `pending` và xếp job gửi,
    /// cùng transaction với event. Thân và `webhook-id` giữ nguyên, nên bên nhận lọc trùng được.
    /// Lần gửi đang chờ thì từ chối: job của nó vẫn đang thử.
    pub(crate) async fn redeliver(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
    ) -> Result<Delivery, Error> {
        let row = delivery::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| delivery_not_found(id))?;
        let transaction = tenant.database().begin_write().await?;
        // Cập nhật có điều kiện: hai lần bấm cùng lúc chỉ một lần mở lại được.
        let reopened = delivery::Entity::update_many()
            .col_expr(delivery::Column::Status, sea_orm::sea_query::Expr::value(STATUS_PENDING))
            .filter(delivery::Column::Id.eq(id))
            .filter(delivery::Column::Status.ne(STATUS_PENDING))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if reopened.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(Error::conflict(
                "webhooks.delivery_in_progress",
                "Delivery `{id}` is still being sent or retried; wait until it succeeds or fails.",
            )
            .with_param("id", id));
        }
        let options = EnqueueOptions::default().max_attempts(MAX_ATTEMPTS);
        JobQueue::new("webhooks")
            .enqueue(&transaction, tenant, &Send { delivery_id: id }, options)
            .await?;
        let event = events::DeliveryRedelivered(Change {
            before: Some(json!({"delivery_id": id, "status": row.status})),
            after: Some(
                json!({"delivery_id": id, "status": STATUS_PENDING, "event_name": row.event_name}),
            ),
        });
        self.outbox
            .record(&transaction, tenant, actor, events::endpoint_subject(row.endpoint_id), &event)
            .await?;
        transaction.commit().await?;
        tracing::info!(delivery_id = %id, "webhook delivery sent again by hand");
        Ok(Delivery { status: STATUS_PENDING.to_owned(), ..row })
    }

    /// Mã hoá secret ký của endpoint `id` thuộc khách `tenant`.
    async fn seal(&self, tenant: &TenantContext, id: Uuid, secret: &str) -> Result<String, Error> {
        let sealer = self.secrets.sealer().await?;
        sealer.seal(tenant.id(), SECRET_PURPOSE, &id.to_string(), secret.as_bytes())
    }

    /// Kiểm đầu vào của một endpoint (trường `None` bỏ qua) và báo mọi lỗi một lần. Trả
    /// địa chỉ đã chuẩn hoá và danh sách event đã sắp, bỏ trùng.
    fn check(
        &self,
        url: Option<&str>,
        description: Option<&str>,
        events: Option<Vec<String>>,
    ) -> Result<(Option<String>, Option<Vec<String>>), Error> {
        let mut problems = Vec::new();
        let url = url.and_then(|url| match self.check_url(url) {
            Ok(url) => Some(url),
            Err(problem) => {
                problems.push(problem);
                None
            }
        });
        if description.is_some_and(|text| text.trim().chars().count() > DESCRIPTION_MAX_LENGTH) {
            problems.push(
                FieldError::new(
                    "description",
                    "length",
                    "must be at most {description_max_length} characters long",
                )
                .with_param("description_max_length", DESCRIPTION_MAX_LENGTH),
            );
        }
        let events = events.map(|events| self.check_events(events, &mut problems));
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        Ok((url, events))
    }

    /// Địa chỉ đã kiểm: URL tuyệt đối tới 2048 ký tự, `https` (hoặc `http` khi cấu hình
    /// cho phép), không chứa tên đăng nhập / mật khẩu. Sai thì câu nói rõ phải sửa gì.
    fn check_url(&self, raw: &str) -> Result<String, FieldError> {
        let raw = raw.trim();
        if raw.chars().count() > URL_MAX_LENGTH {
            return Err(FieldError::new("url", "invalid", "must be at most {max} characters long")
                .with_param("max", URL_MAX_LENGTH));
        }
        let Ok(url) = reqwest::Url::parse(raw) else {
            return Err(FieldError::new("url", "invalid", "must be an absolute URL"));
        };
        if url.host_str().is_none_or(str::is_empty) {
            return Err(FieldError::new("url", "invalid", "must name a host"));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(FieldError::new(
                "url",
                "invalid",
                "must not contain a user name or password",
            ));
        }
        if !self.allow_private_networks && !crate::network::is_public_host(&url) {
            return Err(FieldError::new(
                "url",
                "invalid",
                // Cùng câu với `network::PRIVATE_ADDRESS` (lỗi lúc gửi).
                "points to a private or local network address; webhooks are sent only to public \
                 addresses unless the server sets `AUTOXIDE_WEBHOOKS_ALLOW_PRIVATE_NETWORKS=true`",
            ));
        }
        match url.scheme() {
            "https" => Ok(url.to_string()),
            "http" if self.allow_http => Ok(url.to_string()),
            _ => Err(FieldError::new(
                "url",
                "invalid",
                "must start with `https://` (`http://` only when the server sets \
                 `AUTOXIDE_WEBHOOKS_ALLOW_HTTP=true`, for development)",
            )),
        }
    }

    /// Danh sách event đã sắp, bỏ trùng; lỗi (rỗng, quá dài, tên không có) ghi vào
    /// `problems`.
    fn check_events(&self, events: Vec<String>, problems: &mut Vec<FieldError>) -> Vec<String> {
        if events.is_empty() || events.len() > MAX_EVENTS {
            problems.push(
                FieldError::new(
                    "events",
                    "length",
                    "must list 1 to {max_events} event names, or `*` for every event",
                )
                .with_param("max_events", MAX_EVENTS),
            );
        }
        for (index, event) in events.iter().enumerate() {
            let event = event.trim();
            if event != ALL_EVENTS && !self.catalog.contains(event) {
                problems.push(FieldError::new(
                    format!("events[{index}]"),
                    "unknown",
                    "is not an event of this system; see `GET /api/v1/webhooks/events`",
                ));
            }
        }
        let mut events: Vec<String> =
            events.into_iter().map(|event| event.trim().to_owned()).collect();
        events.sort();
        events.dedup();
        events
    }
}

/// Những gì audit cần biết về một endpoint (không có secret). URL chỉ giữ scheme, host,
/// port: đường dẫn và query của URL webhook thường chính là secret (Slack, Discord…), mà event
/// đi tới audit log (người có `audit.read`) và tới webhook khác nhận `*` (D-157).
fn snapshot(endpoint: &Endpoint) -> Value {
    json!({
        "url": origin(&endpoint.url),
        "description": endpoint.description,
        "events": endpoint.events,
        "disabled": endpoint.disabled_at.is_some(),
    })
}

/// `scheme://host[:port]` của `url`, bỏ đường dẫn, query, thông tin đăng nhập.
fn origin(url: &str) -> String {
    reqwest::Url::parse(url)
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_else(|_| "invalid".to_owned())
}

fn delivery_not_found(id: Uuid) -> Error {
    Error::not_found("webhooks.delivery_not_found", "No webhook delivery `{id}`.")
        .with_param("id", id)
}

fn endpoint_not_found(id: Uuid) -> Error {
    Error::not_found("webhooks.endpoint_not_found", "There is no webhook `{id}`.")
        .with_param("id", id)
}

/// Bây giờ, cắt tới micro giây để hai database lưu cùng một giá trị.
pub(crate) fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

#[cfg(test)]
mod tests {
    use super::origin;

    /// Event về endpoint chỉ mang scheme, host, port của URL (D-157).
    #[test]
    fn url_trong_event_bo_duong_dan_va_query() {
        assert_eq!(
            origin("https://hooks.example.com/services/T0/B0/secret?token=abc"),
            "https://hooks.example.com"
        );
        assert_eq!(origin("https://user:pw@example.com:8443/h"), "https://example.com:8443");
        assert_eq!(origin("not a url"), "invalid");
    }
}
