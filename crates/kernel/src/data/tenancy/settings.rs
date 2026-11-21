//! Cấu hình và secret theo khách (đặc tả 33 "Cấu hình và secret theo khách", D-258).
//!
//! - Module khai cấu hình nó nhận theo khách qua `Module::tenant_settings` ([`TenantSetting`],
//!   tên `<module>.<khoá>`, vd `comments.max_links`); kernel khai `kernel.public_url`.
//! - Cấu hình xếp được vào một nhóm ([`TenantSetting::group`]). Nhóm [`CONNECTION_GROUP`] là kết
//!   nối tới đối tác (máy chủ SMTP, nhà cung cấp AI, cổng thanh toán…): tên bắt buộc theo dạng
//!   `<module>.connection.<loại>.<trường>` (vd `mail.connection.smtp.url`), trường bí mật khai
//!   `.secret()`. `GET /api/v1/kernel/settings?group=connection` là nguồn của màn "Kết nối đối tác".
//! - Đổi tên một cấu hình: khai tên cũ bằng [`TenantSetting::formerly`]. Giá trị khách đã đặt dưới
//!   tên cũ vẫn được đọc (secret mở bằng đúng tên nó được mã hoá cùng), API vẫn nhận tên cũ; lần
//!   đặt kế tiếp ghi dưới tên mới và bỏ dòng tên cũ. Không cần upgrade dữ liệu.
//! - Module đọc bằng trait [`TenantSettings`] (`context.require::<dyn TenantSettings>()`). Khách
//!   chưa đặt thì module dùng cấu hình chung (biến môi trường) như trước.
//! - Lưu ở bảng `kernel_tenant_settings` (cấp khách hàng, database của khách); secret mã hoá bằng
//!   [`Sealer`](crate::secret::Sealer) và **không bao giờ** trả ra API, log hay event: event
//!   `kernel.setting_changed` chỉ mang tên.
//! - Quản lý qua `GET /api/v1/kernel/settings`, `PUT` / `DELETE /api/v1/kernel/settings/{name}`
//!   (permission `kernel.settings.manage`) và tool `kernel.list_settings`. Nhân sự nền tảng đặt
//!   cho một khách bất kỳ qua `…/kernel/tenants/{id}/settings…` (D-210, đối chiếu thiết kế 2).
//! - Cấu hình có thể khai bộ kiểm ([`TenantSetting::checked`]): giá trị sai bị từ chối lúc đặt
//!   (422). Module đọc bằng [`parsed_setting`]: giá trị đã lưu mà nay không còn hợp lệ (bộ kiểm
//!   chặt hơn ở bản mới) thì ghi cảnh báo và dùng cấu hình chung — không làm hỏng request.
//! - Đọc qua cache theo khách (một câu truy vấn lấy mọi cấu hình của khách, giữ
//!   `AUTOXIDE_TENANCY_CACHE_SECONDS`): module đọc ở đường nóng (ngôn ngữ của `content` mỗi
//!   request) không thêm truy vấn. Đặt / bỏ trên máy chủ này xoá cache ngay; máy chủ khác thấy
//!   trong TTL.

use crate::time::Timestamp;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use validator::Validate;
use zeroize::Zeroizing;

use super::{DatabaseRegistry, ScopeExt, TenantContext, TenantId, TenantScoped, TenantStatus};
use crate::access::{Permission, Principal, Risk};
use crate::error::{Error, FieldError};
use crate::event::{Actor, Event, EventInfo, Outbox};
use crate::mcp::Tool;
use crate::migration::prelude::*;
use crate::response::{Data, Problem};
use crate::secret::SecretKey;

/// Đọc / ghi cấu hình theo khách của khách mình.
pub const SETTINGS_MANAGE: Permission = Permission::new(
    "kernel.settings.manage",
    "Read and change the settings of this tenant (mail server, AI provider keys…); secret values \
     can be replaced but never read back.",
)
.with_risk(Risk::SensitiveData);

/// URL công khai của khách (link trong email…); không đặt thì `AUTOXIDE_HTTP_PUBLIC_URL`.
pub const PUBLIC_URL: TenantSetting = TenantSetting::new(
    "kernel.public_url",
    "Public address of this tenant's site, e.g. `https://shop.example` (scheme and host only). \
     Links in mails sent for this tenant (sign-in links, invitations, password resets, email \
     checks, newsletter and comment confirmations) open on it: the server's link addresses keep \
     their path, only the start changes. When unset, the server's addresses are used as set.",
)
.checked(site_origin);

fn site_origin(raw: &str) -> Result<(), FieldError> {
    if origin_of(raw.trim()).is_some_and(|origin| origin == raw.trim().trim_end_matches('/')) {
        Ok(())
    } else {
        Err(FieldError::new(
            "value",
            "invalid",
            "must be an address such as `https://shop.example`, with no path",
        ))
    }
}

/// Phần đầu `scheme://host[:port]` của một địa chỉ http(s); `None` khi không phải.
fn origin_of(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let host_len = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    if host_len == 0 || rest[..host_len].contains(['@', ' ']) {
        return None;
    }
    Some(&url[..url.len() - rest.len() + host_len])
}

/// Link trong thư theo khách (đề bài giao diện): khách đã đặt `kernel.public_url` thì link mở trên
/// tên miền của khách — giữ đường dẫn của địa chỉ máy chủ, chỉ đổi phần đầu. Clone rẻ; mặc định
/// (không có cấu hình theo khách, vd test dựng tay) trả nguyên địa chỉ.
#[derive(Clone, Default)]
pub struct SiteLinks(Option<Arc<dyn TenantSettings>>);

impl SiteLinks {
    pub fn new(settings: Arc<dyn TenantSettings>) -> Self {
        SiteLinks(Some(settings))
    }

    /// `template` (địa chỉ đã cấu hình của máy chủ, có thể chứa `{token}`) cho khách `tenant`.
    pub async fn rewrite(&self, tenant: &TenantContext, template: &str) -> Result<String, Error> {
        let Some(settings) = &self.0 else { return Ok(template.to_owned()) };
        // Giá trị cũ đặt trước khi có bộ kiểm mà sai dạng: dùng địa chỉ của máy chủ.
        let site = parsed_setting(settings.as_ref(), tenant, &PUBLIC_URL, |raw| {
            site_origin(raw).map(|()| raw.trim().to_owned())
        })
        .await?;
        Ok(site.map_or_else(|| template.to_owned(), |site| rebase(template, &site)))
    }
}

/// Đổi phần đầu của `template` thành `site`; `template` không phải địa chỉ http(s) thì giữ nguyên.
fn rebase(template: &str, site: &str) -> String {
    let site = site.trim().trim_end_matches('/');
    match (origin_of(template), origin_of(site)) {
        (Some(origin), Some(_)) => format!("{site}{}", &template[origin.len()..]),
        _ => template.to_owned(),
    }
}

/// Mô hình kinh doanh của khách (D-185): giao diện chọn các bước thiết lập, màn mặc định theo đó.
/// Chỉ là thông tin; quyền dùng vẫn do gói (bước 35) quyết.
pub const BUSINESS_MODEL: TenantSetting = TenantSetting::new(
    "kernel.business_model",
    "How this tenant does business, so interfaces show the right setup steps and screens: \
     `independent` (one shop), `multi_store` (one business, several stores), `chain` (a parent \
     with child tenants), `marketplace` (sellers sell on the tenant's site) or `saas` (the tenant \
     sells software by plan). Information only: what the tenant may use comes from its plan.",
)
.checked(business_model);

/// Giá trị `kernel.business_model` nhận.
pub const BUSINESS_MODELS: [&str; 5] =
    ["independent", "multi_store", "chain", "marketplace", "saas"];

fn business_model(raw: &str) -> Result<(), FieldError> {
    if BUSINESS_MODELS.contains(&raw.trim()) {
        Ok(())
    } else {
        Err(FieldError::new(
            "value",
            "invalid",
            "must be `independent`, `multi_store`, `chain`, `marketplace` or `saas`",
        ))
    }
}

/// Event: một cấu hình của khách vừa được đặt hay bỏ. Chỉ mang tên, không bao giờ mang giá trị.
pub(crate) const SETTING_CHANGED: EventInfo = EventInfo::new(
    "kernel.setting_changed",
    "A setting of the tenant was set or removed; carries only its name.",
);

/// Độ dài tối đa của một giá trị.
const VALUE_MAX: usize = 4000;

/// Bộ kiểm giá trị của một cấu hình: `Err` với lỗi của ô `value` (câu mẫu có bản dịch như mọi
/// `FieldError`).
pub type SettingCheck = fn(&str) -> Result<(), FieldError>;

/// Nhóm của cấu hình kết nối tới đối tác (đối chiếu thiết kế đợt 2): tên phải theo dạng
/// `<module>.connection.<loại>.<trường>`.
pub const CONNECTION_GROUP: &str = "connection";

/// Một cấu hình module nhận theo khách.
#[derive(Debug, Clone, Copy)]
pub struct TenantSetting {
    name: &'static str,
    description: &'static str,
    secret: bool,
    check: Option<SettingCheck>,
    group: Option<&'static str>,
    former_name: Option<&'static str>,
}

// So theo tên (tên là duy nhất trong sổ); không so con trỏ hàm của bộ kiểm.
impl PartialEq for TenantSetting {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.secret == other.secret
    }
}

impl Eq for TenantSetting {}

impl TenantSetting {
    /// Cấu hình `name` (`<module>.<khoá>`; khoá là một hay nhiều đoạn chữ thường, số, `_` nối bằng
    /// dấu chấm), mô tả tiếng Anh cho người quản trị.
    pub const fn new(name: &'static str, description: &'static str) -> Self {
        TenantSetting {
            name,
            description,
            secret: false,
            check: None,
            group: None,
            former_name: None,
        }
    }

    /// Giá trị là secret: lưu mã hoá, không bao giờ trả ra.
    pub const fn secret(mut self) -> Self {
        self.secret = true;
        self
    }

    /// Giá trị phải qua `check` khi đặt (422 `validation_failed`, lỗi ở ô `value`). Module dùng
    /// cùng hàm dịch giá trị cho bộ kiểm và cho [`parsed_setting`].
    pub const fn checked(mut self, check: SettingCheck) -> Self {
        self.check = Some(check);
        self
    }

    /// Xếp cấu hình vào nhóm `group` (chữ thường, số, `_`): danh sách lọc được theo nhóm
    /// (`?group=`). Nhóm [`CONNECTION_GROUP`] buộc tên dạng `<module>.connection.<loại>.<trường>`.
    pub const fn group(mut self, group: &'static str) -> Self {
        self.group = Some(group);
        self
    }

    /// Cấu hình này trước đây tên là `name`: giá trị khách đã đặt dưới tên cũ vẫn được đọc, API vẫn
    /// nhận tên cũ; lần đặt kế tiếp ghi dưới tên mới.
    pub const fn formerly(mut self, name: &'static str) -> Self {
        self.former_name = Some(name);
        self
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Nhóm của cấu hình, nếu có.
    pub fn group_name(&self) -> Option<&'static str> {
        self.group
    }

    /// Tên cũ của cấu hình, nếu nó từng đổi tên.
    pub fn former_name(&self) -> Option<&'static str> {
        self.former_name
    }

    /// `name` là tên (hiện tại hay cũ) của cấu hình này.
    fn answers_to(&self, name: &str) -> bool {
        self.name == name || self.former_name == Some(name)
    }

    /// Dòng đã lưu của cấu hình này trong `rows`: dòng tên hiện tại trước, không có thì dòng tên cũ.
    fn row_in<'a>(&self, rows: &'a [entity::Model]) -> Option<&'a entity::Model> {
        rows.iter()
            .find(|row| row.name == self.name)
            .or_else(|| rows.iter().find(|row| Some(row.name.as_str()) == self.former_name))
    }

    pub fn description(&self) -> &'static str {
        self.description
    }

    pub fn is_secret(&self) -> bool {
        self.secret
    }
}

/// Giá trị một cấu hình của khách. `Debug` không in secret; bộ nhớ được xoá khi bỏ.
#[derive(Clone)]
pub struct SettingValue {
    value: Zeroizing<String>,
    secret: bool,
}

impl SettingValue {
    /// Giá trị không phải secret (cho bản giả trong test; module đọc qua [`TenantSettings`]).
    pub fn new(value: impl Into<String>) -> Self {
        SettingValue { value: Zeroizing::new(value.into()), secret: false }
    }

    pub fn as_str(&self) -> &str {
        &self.value
    }
}

impl std::fmt::Debug for SettingValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.secret { f.write_str("SettingValue(<secret>)") } else { self.value.fmt(f) }
    }
}

/// Giá trị `setting` của `tenant`, dịch bằng `parse`; `None` khi khách chưa đặt. Giá trị đã lưu mà
/// `parse` từ chối (vd bộ kiểm của bản mới chặt hơn) → cảnh báo trong log và `None`: module dùng
/// cấu hình chung thay vì làm hỏng request của khách.
pub async fn parsed_setting<T>(
    settings: &dyn TenantSettings,
    tenant: &TenantContext,
    setting: &TenantSetting,
    parse: impl FnOnce(&str) -> Result<T, FieldError>,
) -> Result<Option<T>, Error> {
    let Some(value) = settings.get(tenant, setting).await? else {
        return Ok(None);
    };
    match parse(value.as_str()) {
        Ok(parsed) => Ok(Some(parsed)),
        Err(problem) => {
            tracing::warn!(
                setting = setting.name,
                tenant_id = %tenant.id(),
                problem = %problem.message,
                "a tenant setting is not valid any more; the server-wide value is used"
            );
            Ok(None)
        }
    }
}

/// Đọc cấu hình của một khách. Module `kernel` công bố.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait TenantSettings: Send + Sync {
    /// Giá trị `setting` của khách `tenant`; `None` khi khách chưa đặt.
    async fn get(
        &self,
        tenant: &TenantContext,
        setting: &TenantSetting,
    ) -> Result<Option<SettingValue>, Error>;
}

/// Sổ cấu hình đã khai của mọi module, điền một lần khi dựng `Modules`. Clone rẻ.
#[derive(Clone, Default)]
pub(crate) struct Catalog(Arc<OnceLock<Vec<TenantSetting>>>);

impl Catalog {
    pub(crate) fn fill(&self, settings: Vec<TenantSetting>) {
        let _ = self.0.set(settings);
    }

    fn all(&self) -> &[TenantSetting] {
        self.0.get().map_or(&[], Vec::as_slice)
    }

    /// Cấu hình tên `name` — tên hiện tại hay tên cũ ([`TenantSetting::formerly`]).
    fn find(&self, name: &str) -> Result<TenantSetting, Error> {
        self.all()
            .iter()
            .find(|setting| setting.answers_to(name))
            .copied()
            .ok_or_else(|| super::errors::tenant_setting_unknown(name))
    }
}

/// `name` có dạng `<module>.<khoá>`; khoá là một hay nhiều đoạn (chữ thường, số, `_`) nối bằng `.`.
fn well_named(module: &str, name: &str) -> bool {
    name.len() <= 100
        && name
            .strip_prefix(module)
            .and_then(|rest| rest.strip_prefix('.'))
            .is_some_and(|key| key.split('.').all(crate::module::is_identifier))
}

/// Kiểm khai báo của module `module`: tên `<module>.<khoá>`, không trùng (kể cả với tên cũ); cấu
/// hình nhóm `connection` — và chỉ chúng — mang tên `<module>.connection.<loại>.<trường>`.
pub(crate) fn declaration_problems(
    module: &str,
    settings: &[TenantSetting],
    seen: &mut Vec<&'static str>,
) -> Vec<String> {
    let mut problems = Vec::new();
    for setting in settings {
        if !well_named(module, setting.name) {
            problems.push(format!(
                "tenant setting `{}` of module `{module}` must be named `{module}.<key>` (lower \
                 case letters, digits and `_`; several parts are joined with `.`; at most 100 \
                 characters)",
                setting.name
            ));
        }
        if setting.description.trim().is_empty() {
            problems.push(format!("tenant setting `{}` has no description", setting.name));
        }
        if let Some(group) = setting.group
            && !crate::module::is_identifier(group)
        {
            problems.push(format!(
                "tenant setting `{}` is in group `{group}`: a group is lower case letters, digits \
                 and `_`",
                setting.name
            ));
        }
        // `<module>.connection.<loại>.<trường>`: ít nhất hai đoạn sau `connection`.
        let mut parts = setting.name.split('.').skip(1);
        let connection_name = parts.next() == Some(CONNECTION_GROUP);
        let in_group = setting.group == Some(CONNECTION_GROUP);
        if in_group && !(connection_name && parts.count() >= 2) {
            problems.push(format!(
                "tenant setting `{}` is in group `connection`, so it must be named \
                 `{module}.connection.<kind>.<field>`",
                setting.name
            ));
        } else if connection_name && !in_group {
            problems.push(format!(
                "tenant setting `{}` is named like a partner connection: declare it with \
                 `.group(CONNECTION_GROUP)`",
                setting.name
            ));
        }
        if let Some(former) = setting.former_name
            && !well_named(module, former)
        {
            problems.push(format!(
                "tenant setting `{}` has the former name `{former}`, which is not a name of \
                 module `{module}`",
                setting.name
            ));
        }
        for name in std::iter::once(setting.name).chain(setting.former_name) {
            if seen.contains(&name) {
                problems.push(format!("tenant setting `{name}` is declared twice"));
            }
            seen.push(name);
        }
    }
    problems
}

/// Sao mọi cài đặt KHÔNG bí mật của `source` sang `target` (sandbox, D-232): secret (thông tin kết
/// nối đối tác) không bao giờ sang — sandbox dùng bộ giả. Cài đặt `target` đã có thì giữ. Trả số dòng
/// mới.
pub(crate) async fn copy_plain(
    source: &TenantContext,
    target: &TenantContext,
) -> Result<u64, Error> {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};

    let rows = entity::Entity::find()
        .filter(entity::Column::TenantId.eq(source.id().as_uuid()))
        .filter(entity::Column::Secret.eq(false))
        .all(source.database().read())
        .await?;
    let mut copied = 0;
    for row in rows {
        let insert = entity::Entity::insert(entity::ActiveModel {
            tenant_id: Set(target.id().as_uuid()),
            name: Set(row.name),
            value: Set(row.value),
            secret: Set(false),
            updated_by: Set(None),
            updated_at: Set(row.updated_at),
        })
        .on_conflict(
            sea_orm::sea_query::OnConflict::columns([
                entity::Column::TenantId,
                entity::Column::Name,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(target.database().write())
        .await?;
        copied += insert;
    }
    Ok(copied)
}

mod entity {
    //! Bảng `kernel_tenant_settings`.
    use crate::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "kernel_tenant_settings")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub name: String,
        /// Chữ thường với cấu hình thường, `sealed:…` với secret.
        pub value: String,
        pub secret: bool,
        pub updated_by: Option<Uuid>,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

use entity::{Column, Entity};

impl TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}

// Một cấu hình đã khai và trạng thái của nó ở khách hiện tại.
/// A setting the installed modules take per tenant, and its state in the current tenant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = TenantSettingState)]
pub(crate) struct SettingState {
    /// `<module>.<key>`, e.g. `kernel.public_url`; a partner connection is named
    /// `<module>.connection.<kind>.<field>`, e.g. `mail.connection.smtp.url`.
    #[schema(example = "kernel.public_url")]
    name: String,
    description: String,
    /// Group of the setting, to filter the list by (`?group=`); `connection`: a connection to a
    /// partner service (mail server, AI provider, payment gateway…). `null`: no group.
    #[schema(example = "connection")]
    group: Option<String>,
    /// Secret settings are stored encrypted and never returned: `value` is always `null`.
    secret: bool,
    /// Whether this tenant has set it; otherwise the server-wide configuration applies.
    is_set: bool,
    /// The value, for settings that are not secret and are set.
    value: Option<String>,
    updated_at: Option<Timestamp>,
    /// For a setting of a partner connection: the last time the connection was used (or tested
    /// with `POST /api/v1/kernel/settings/{name}/test`).
    #[serde(skip_serializing_if = "Option::is_none")]
    last_used_at: Option<Timestamp>,
    /// For a setting of a partner connection: why its last use failed; omitted when it worked.
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
}

/// Payload of `kernel.setting_changed`: the name only.
#[derive(Debug, Serialize)]
struct SettingChanged {
    name: String,
    /// `true` when set, `false` when removed.
    set: bool,
}

impl Event for SettingChanged {
    const NAME: &'static str = "kernel.setting_changed";
}

/// Cấu hình đã lưu của một khách (giá trị secret vẫn mã hoá), giữ trong cache.
type Stored = Arc<Vec<entity::Model>>;

/// Số khách trong cache quá mức này thì bỏ các mục đã hết hạn khi thêm mục mới.
const CACHE_PRUNE_AT: usize = 10_000;

/// Dịch vụ cấu hình theo khách.
pub(crate) struct SettingsService {
    catalog: Catalog,
    secret: SecretKey,
    outbox: Outbox,
    /// Thời gian giữ cache (`AUTOXIDE_TENANCY_CACHE_SECONDS`).
    ttl: Duration,
    cache: Mutex<HashMap<TenantId, (Instant, Stored)>>,
}

/// Mục đích (AAD) khi mã hoá giá trị secret. Tên của dòng cũng nằm trong AAD: secret đặt dưới tên
/// cũ của một cấu hình chỉ mở được bằng tên cũ đó — luôn mở bằng `row.name`.
pub(crate) const PURPOSE: &str = "kernel.tenant_setting";

/// Trạng thái của `setting` ở một khách, từ dòng đã lưu `row` (nếu khách đã đặt).
fn state(setting: &TenantSetting, row: Option<&entity::Model>) -> SettingState {
    SettingState {
        name: setting.name.to_owned(),
        description: setting.description.to_owned(),
        group: setting.group.map(str::to_owned),
        secret: setting.secret,
        is_set: row.is_some(),
        value: row.filter(|_| !setting.secret).map(|row| row.value.clone()),
        updated_at: row.map(|row| row.updated_at),
        last_used_at: None,
        last_error: None,
    }
}

impl SettingsService {
    pub(crate) fn new(catalog: Catalog, secret: SecretKey, outbox: Outbox, ttl: Duration) -> Self {
        SettingsService { catalog, secret, outbox, ttl, cache: Mutex::default() }
    }

    /// Mọi cấu hình đã lưu của khách `tenant`, qua cache. Khoá `mutex` chỉ giữ trong từng câu,
    /// không qua `await`.
    async fn stored(&self, tenant: &TenantContext) -> Result<Stored, Error> {
        let cached =
            self.cache.lock().unwrap_or_else(PoisonError::into_inner).get(&tenant.id()).cloned();
        if let Some((at, rows)) = cached
            && at.elapsed() < self.ttl
        {
            return Ok(rows);
        }
        let rows: Stored =
            Arc::new(Entity::find().scoped(tenant).all(tenant.database().read()).await?);
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if cache.len() >= CACHE_PRUNE_AT {
            let ttl = self.ttl;
            cache.retain(|_, (at, _)| at.elapsed() < ttl);
        }
        cache.insert(tenant.id(), (Instant::now(), rows.clone()));
        Ok(rows)
    }

    /// Bỏ cấu hình của khách `tenant` khỏi cache (sau khi đặt / bỏ trên máy chủ này).
    fn forget(&self, tenant: TenantId) {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner).remove(&tenant);
    }

    /// Mọi cấu hình đã khai, theo tên, với trạng thái ở khách `tenant`; `group`: chỉ cấu hình của
    /// nhóm đó (nhóm không ai khai: danh sách rỗng).
    pub(crate) async fn list(
        &self,
        tenant: &TenantContext,
        group: Option<&str>,
    ) -> Result<Vec<SettingState>, Error> {
        // Trang quản trị đọc thẳng database: thấy ngay giá trị vừa đặt ở máy chủ khác.
        let rows = Entity::find().scoped(tenant).all(tenant.database().read()).await?;
        let mut states: Vec<SettingState> = self
            .catalog
            .all()
            .iter()
            .filter(|setting| group.is_none_or(|group| setting.group == Some(group)))
            .map(|setting| state(setting, setting.row_in(&rows)))
            .collect();
        // Lần dùng / lỗi cuối của kết nối ra ngoài, gắn vào từng cài đặt của kết nối.
        let connections = crate::ops::connections::statuses(tenant).await?;
        for state in &mut states {
            if let Some(status) = crate::ops::connections::connection_of(&state.name)
                .and_then(|connection| connections.get(connection))
            {
                state.last_used_at = status.last_used_at;
                state.last_error = status.last_error.clone();
            }
        }
        states.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(states)
    }

    /// Đặt cấu hình `name` của khách `tenant` (ghi đè nếu có). `name` là tên cũ của một cấu hình
    /// thì đặt chính cấu hình đó, dưới tên hiện tại.
    pub(crate) async fn put(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        name: &str,
        value: &str,
    ) -> Result<SettingState, Error> {
        let setting = self.catalog.find(name)?;
        let name = setting.name;
        let value = value.trim();
        if value.is_empty() || value.chars().count() > VALUE_MAX {
            return Err(Error::validation(vec![
                FieldError::new(
                    "value",
                    "length",
                    "must be 1 to {value_max} characters long; remove the setting with `DELETE` \
                     instead of setting it empty",
                )
                .with_param("value_max", VALUE_MAX),
            ]));
        }
        if let Some(check) = setting.check
            && let Err(mut problem) = check(value)
        {
            problem.field = "value".to_owned();
            return Err(Error::validation(vec![problem]));
        }
        let stored = if setting.secret {
            // Mã hoá TRƯỚC transaction: dẫn khoá có thể đọc database (khoá gốc giữ trong đó).
            let sealer = self.secret.sealer().await?;
            sealer.seal(tenant.id(), PURPOSE, name, value.as_bytes())?
        } else {
            value.to_owned()
        };
        let now = Timestamp::now();
        let row = entity::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            name: Set(name.to_owned()),
            value: Set(stored),
            secret: Set(setting.secret),
            updated_by: Set(Some(principal.user_id())),
            updated_at: Set(now),
        };
        let transaction = tenant.database().begin_write().await?;
        Entity::insert(row)
            .on_conflict(
                OnConflict::columns([Column::TenantId, Column::Name])
                    .update_columns([
                        Column::Value,
                        Column::Secret,
                        Column::UpdatedBy,
                        Column::UpdatedAt,
                    ])
                    .to_owned(),
            )
            .exec_without_returning(&transaction)
            .await?;
        if let Some(former) = setting.former_name {
            // Giá trị đặt dưới tên cũ đã được thay: bỏ dòng cũ để không còn hai dòng cho một cấu hình.
            Entity::delete_many()
                .filter(Column::Name.eq(former))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
        }
        let event = SettingChanged { name: name.to_owned(), set: true };
        self.outbox
            .record(&transaction, tenant, Actor::from(principal), Some(subject(name)), &event)
            .await?;
        transaction.commit().await?;
        self.forget(tenant.id());
        tracing::info!(setting = name, tenant_id = %tenant.id(), "tenant setting changed");
        Ok(SettingState {
            is_set: true,
            value: (!setting.secret).then(|| value.to_owned()),
            updated_at: Some(now),
            ..state(&setting, None)
        })
    }

    /// Bỏ cấu hình `name` của khách `tenant` (về cấu hình chung), kể cả giá trị đặt dưới tên cũ.
    /// Không có thì không đổi gì.
    pub(crate) async fn delete(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        name: &str,
    ) -> Result<(), Error> {
        let setting = self.catalog.find(name)?;
        let name = setting.name;
        let transaction = tenant.database().begin_write().await?;
        let deleted = Entity::delete_many()
            .filter(Column::Name.is_in(std::iter::once(name).chain(setting.former_name)))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if deleted.rows_affected > 0 {
            let event = SettingChanged { name: name.to_owned(), set: false };
            self.outbox
                .record(&transaction, tenant, Actor::from(principal), Some(subject(name)), &event)
                .await?;
        }
        transaction.commit().await?;
        self.forget(tenant.id());
        Ok(())
    }
}

fn subject(name: &str) -> String {
    format!("kernel.setting:{name}")
}

#[async_trait::async_trait]
impl TenantSettings for SettingsService {
    async fn get(
        &self,
        tenant: &TenantContext,
        setting: &TenantSetting,
    ) -> Result<Option<SettingValue>, Error> {
        let rows = self.stored(tenant).await?;
        // Dòng tên hiện tại, không có thì dòng tên cũ (khách đặt trước khi cấu hình đổi tên).
        let Some(row) = setting.row_in(&rows) else {
            return Ok(None);
        };
        let value = if row.secret {
            let sealer = self.secret.sealer().await?;
            let plain = sealer.open(tenant.id(), PURPOSE, &row.name, &row.value)?;
            Zeroizing::new(String::from_utf8_lossy(&plain).into_owned())
        } else {
            Zeroizing::new(row.value.clone())
        };
        Ok(Some(SettingValue { value, secret: row.secret }))
    }
}

// --- REST -----------------------------------------------------------------------------------

/// The new value of a setting.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = TenantSettingValue)]
pub(crate) struct SettingInput {
    /// 1 to 4000 characters. Secret values are stored encrypted and never returned.
    #[schema(example = "https://shop.example")]
    value: String,
}

/// Which settings to list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SettingFilter {
    /// Only settings of this group, e.g. `connection` (connections to partner services: mail
    /// server, AI provider…). Omit for every setting; a group no setting is in gives an empty
    /// list.
    #[param(example = "connection")]
    group: Option<String>,
}

/// List this tenant's settings.
///
/// Requires `kernel.settings.manage`. Every setting the installed modules take per tenant, with
/// whether this tenant has set it; unset ones use the server-wide configuration. Values of
/// secret settings are never returned. `?group=connection` lists only the connections to partner
/// services, named `<module>.connection.<kind>.<field>`.
#[utoipa::path(
    get,
    path = "/settings",
    extensions(("x-permissions" = json!(["kernel.settings.manage"]))),
    operation_id = "list_tenant_settings",
    security(("bearer" = [])),
    params(SettingFilter),
    responses(
        (status = 200, description = "Every setting (of the group, when one is given), sorted by name.", body = Data<Vec<SettingState>>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.settings.manage`; code `permission_denied`.", body = Problem),
    )
)]
async fn list_settings(
    State(service): State<Arc<SettingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(filter): Query<SettingFilter>,
) -> Result<Data<Vec<SettingState>>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    Ok(Data { data: service.list(&tenant, filter.group.as_deref()).await? })
}

/// Set one of this tenant's settings.
///
/// Requires `kernel.settings.manage`. Creates or replaces the value; sending it again is safe.
/// A secret value is stored encrypted and the answer does not contain it. The former name of a
/// renamed setting is still accepted; the answer carries the current name.
#[utoipa::path(
    put,
    path = "/settings/{name}",
    extensions(("x-permissions" = json!(["kernel.settings.manage"]))),
    operation_id = "set_tenant_setting",
    security(("bearer" = [])),
    params(("name" = String, Path, description = "Name of the setting, `<module>.<key>`, e.g. `kernel.public_url`.")),
    request_body = SettingInput,
    responses(
        (status = 200, description = "The setting as stored (without a secret value).", body = Data<SettingState>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.settings.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "No module takes this setting (code `tenant_setting_unknown`) or the value is empty or too long; see `errors`.", body = Problem),
    )
)]
async fn set_setting(
    State(service): State<Arc<SettingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(name): Path<String>,
    crate::http::ValidJson(input): crate::http::ValidJson<SettingInput>,
) -> Result<Data<SettingState>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    Ok(Data { data: service.put(&tenant, &principal, &name, &input.value).await? })
}

/// Remove one of this tenant's settings.
///
/// Requires `kernel.settings.manage`. The server-wide configuration applies again. Removing a
/// setting this tenant never set changes nothing.
#[utoipa::path(
    delete,
    path = "/settings/{name}",
    extensions(("x-permissions" = json!(["kernel.settings.manage"]))),
    operation_id = "delete_tenant_setting",
    security(("bearer" = [])),
    params(("name" = String, Path, description = "Name of the setting.")),
    responses(
        (status = 204, description = "The setting is unset (or never was)."),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.settings.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "No module takes this setting; code `tenant_setting_unknown`.", body = Problem),
    )
)]
async fn delete_setting(
    State(service): State<Arc<SettingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(name): Path<String>,
) -> Result<StatusCode, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    service.delete(&tenant, &principal, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

// --- REST cho nhân sự nền tảng: cấu hình của một khách bất kỳ ------------------------------

/// Thứ handler của nhân sự nền tảng cần: service và sổ database (tìm khách theo ID / slug).
#[derive(Clone)]
pub(crate) struct PlatformSettings {
    service: Arc<SettingsService>,
    registry: DatabaseRegistry,
}

impl PlatformSettings {
    /// Khách `id_or_slug`. Đang dời: ghi lúc này sẽ vào database cũ → `tenant_moving`.
    async fn tenant(&self, id_or_slug: &str, writing: bool) -> Result<TenantContext, Error> {
        let tenant = match id_or_slug.parse::<uuid::Uuid>() {
            Ok(id) => self.registry.tenant(TenantId::from_uuid(id)).await?,
            Err(_) => self.registry.tenant_by_slug(id_or_slug).await?,
        };
        if writing && tenant.status() == TenantStatus::Moving {
            return Err(super::errors::tenant_moving());
        }
        Ok(tenant)
    }
}

/// List a tenant's settings (platform staff).
///
/// Requires `kernel.tenants.read` and an account of the platform tenant. Like
/// `GET /api/v1/kernel/settings`, for the tenant in the path: every setting the installed modules
/// take per tenant and whether that tenant has set it. Values of secret settings are never
/// returned. `?group=connection` lists only the connections to partner services.
#[utoipa::path(
    get,
    path = "/tenants/{id}/settings",
    extensions(("x-permissions" = json!(["kernel.tenants.read"]))),
    operation_id = "list_settings_of_tenant",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id or slug of the tenant."), SettingFilter),
    responses(
        (status = 200, description = "Every setting (of the group, when one is given), sorted by name.", body = Data<Vec<SettingState>>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tenants.read` (`permission_denied`), or not an account of the platform tenant (`tenant_platform_only`).", body = Problem),
        (status = 404, description = "No such tenant; code `tenant_not_found`.", body = Problem),
    )
)]
async fn list_tenant_settings(
    State(platform): State<PlatformSettings>,
    principal: Principal,
    Path(id): Path<String>,
    Query(filter): Query<SettingFilter>,
) -> Result<Data<Vec<SettingState>>, Error> {
    principal.require(&super::TENANTS_READ)?;
    super::require_platform(&principal)?;
    let tenant = platform.tenant(&id, false).await?;
    Ok(Data { data: platform.service.list(&tenant, filter.group.as_deref()).await? })
}

/// Set a setting of a tenant (platform staff).
///
/// Requires `kernel.tenants.manage` and an account of the platform tenant. Like
/// `PUT /api/v1/kernel/settings/{name}`, for the tenant in the path: creates or replaces the
/// value; sending it again is safe. The tenant's servers use the new value within
/// `AUTOXIDE_TENANCY_CACHE_SECONDS`.
#[utoipa::path(
    put,
    path = "/tenants/{id}/settings/{name}",
    extensions(("x-permissions" = json!(["kernel.tenants.manage"]))),
    operation_id = "set_setting_of_tenant",
    security(("bearer" = [])),
    params(
        ("id" = String, Path, description = "Id or slug of the tenant."),
        ("name" = String, Path, description = "Name of the setting, `<module>.<key>`, e.g. `comments.max_links`."),
    ),
    request_body = SettingInput,
    responses(
        (status = 200, description = "The setting as stored (without a secret value).", body = Data<SettingState>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tenants.manage` (`permission_denied`), or not an account of the platform tenant (`tenant_platform_only`).", body = Problem),
        (status = 404, description = "No such tenant; code `tenant_not_found`.", body = Problem),
        (status = 409, description = "The tenant is being moved to another placement; code `tenant_moving`.", body = Problem),
        (status = 422, description = "No module takes this setting (code `tenant_setting_unknown`) or the value is not valid for it; see `errors`.", body = Problem),
    )
)]
async fn set_tenant_setting(
    State(platform): State<PlatformSettings>,
    principal: Principal,
    Path((id, name)): Path<(String, String)>,
    crate::http::ValidJson(input): crate::http::ValidJson<SettingInput>,
) -> Result<Data<SettingState>, Error> {
    principal.require(&super::TENANTS_MANAGE)?;
    super::require_platform(&principal)?;
    let tenant = platform.tenant(&id, true).await?;
    Ok(Data { data: platform.service.put(&tenant, &principal, &name, &input.value).await? })
}

/// Remove a setting of a tenant (platform staff).
///
/// Requires `kernel.tenants.manage` and an account of the platform tenant. The server-wide
/// configuration applies to that tenant again. Removing a setting the tenant never set changes
/// nothing.
#[utoipa::path(
    delete,
    path = "/tenants/{id}/settings/{name}",
    extensions(("x-permissions" = json!(["kernel.tenants.manage"]))),
    operation_id = "delete_setting_of_tenant",
    security(("bearer" = [])),
    params(
        ("id" = String, Path, description = "Id or slug of the tenant."),
        ("name" = String, Path, description = "Name of the setting."),
    ),
    responses(
        (status = 204, description = "The setting is unset (or never was)."),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tenants.manage` (`permission_denied`), or not an account of the platform tenant (`tenant_platform_only`).", body = Problem),
        (status = 404, description = "No such tenant; code `tenant_not_found`.", body = Problem),
        (status = 409, description = "The tenant is being moved to another placement; code `tenant_moving`.", body = Problem),
        (status = 422, description = "No module takes this setting; code `tenant_setting_unknown`.", body = Problem),
    )
)]
async fn delete_tenant_setting(
    State(platform): State<PlatformSettings>,
    principal: Principal,
    Path((id, name)): Path<(String, String)>,
) -> Result<StatusCode, Error> {
    principal.require(&super::TENANTS_MANAGE)?;
    super::require_platform(&principal)?;
    let tenant = platform.tenant(&id, true).await?;
    platform.service.delete(&tenant, &principal, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Route dưới `/api/v1/kernel`.
pub(crate) fn routes(service: Arc<SettingsService>, registry: DatabaseRegistry) -> OpenApiRouter {
    let platform = PlatformSettings { service: service.clone(), registry };
    OpenApiRouter::new()
        .routes(routes!(list_settings))
        .routes(routes!(set_setting, delete_setting))
        .with_state(service)
        .merge(
            OpenApiRouter::new()
                .routes(routes!(list_tenant_settings))
                .routes(routes!(set_tenant_setting, delete_tenant_setting))
                .with_state(platform),
        )
}

// Đầu vào của tool `list_settings`: cùng bộ lọc với REST.
/// Which settings to list.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListSettings {
    /// Only settings of this group, e.g. `connection` (connections to partner services). Omit
    /// for every setting.
    #[schema(example = "connection")]
    group: Option<String>,
}

/// Tool MCP — chỉ đọc; không có tool ghi secret (đặc tả 33).
pub(crate) fn tools(service: Arc<SettingsService>) -> Vec<Tool> {
    vec![
        Tool::new(
            "list_settings",
            "List the settings of the current tenant: every setting the installed modules take \
             per tenant, whether it is set, and its value unless it is secret. `group: \
             \"connection\"` lists only the connections to partner services (mail server, AI \
             provider…).",
            move |tenant: TenantContext, _principal: Principal, filter: ListSettings| {
                let service = service.clone();
                async move { service.list(&tenant, filter.group.as_deref()).await }
            },
        )
        .requires(SETTINGS_MANAGE)
        .read_only(),
    ]
}

// --- Migration ------------------------------------------------------------------------------

/// Bảng `kernel_tenant_settings` (cấp khách hàng, có `tenant_id` từ đầu).
pub(crate) struct CreateTenantSettings;

impl MigrationName for CreateTenantSettings {
    fn name(&self) -> &str {
        "m0005_create_tenant_settings"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateTenantSettings {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("kernel_tenant_settings")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("name", 100))
                    .col(schema::text("value"))
                    .col(schema::boolean("secret"))
                    .col(schema::uuid_null("updated_by"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("tenant_id").col("name"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("kernel_tenant_settings").to_owned()).await
    }
}

#[cfg(test)]
mod site_link_tests {
    use super::*;

    #[test]
    fn doi_phan_dau_giu_duong_dan() {
        assert_eq!(
            rebase("https://www.example.com/confirm?token={token}", "https://shop.acme.vn/"),
            "https://shop.acme.vn/confirm?token={token}"
        );
        assert_eq!(
            rebase("https://api.example.com", "http://localhost:3400"),
            "http://localhost:3400"
        );
        assert_eq!(rebase("not a url {token}", "https://shop.acme.vn"), "not a url {token}");
        assert!(site_origin("https://shop.acme.vn").is_ok());
        assert!(site_origin("https://shop.acme.vn/").is_ok());
        assert!(site_origin("https://shop.acme.vn/vi").is_err());
        assert!(site_origin("ftp://shop").is_err());
    }
}
