//! Cấu hình và secret theo khách (đặc tả 33 "Cấu hình và secret theo khách", D-258).
//!
//! - Module khai cấu hình nó nhận theo khách qua `Module::tenant_settings` ([`TenantSetting`],
//!   tên `<module>.<khoá>`, vd `mail.smtp_url`); kernel khai `kernel.public_url`.
//! - Module đọc bằng trait [`TenantSettings`] (`context.require::<dyn TenantSettings>()`). Khách
//!   chưa đặt thì module dùng cấu hình chung (biến môi trường) như trước.
//! - Lưu ở bảng `kernel_tenant_settings` (cấp khách hàng, database của khách); secret mã hoá bằng
//!   [`Sealer`](crate::secret::Sealer) và **không bao giờ** trả ra API, log hay event: event
//!   `kernel.setting_changed` chỉ mang tên.
//! - Quản lý qua `GET /api/v1/kernel/settings`, `PUT` / `DELETE /api/v1/kernel/settings/{name}`
//!   (permission `kernel.settings.manage`) và tool `kernel.settings.list`. Nhân sự nền tảng đặt
//!   cho một khách bất kỳ qua `…/kernel/tenants/{id}/settings…` (D-210, đối chiếu thiết kế 2).
//! - Cấu hình có thể khai bộ kiểm ([`TenantSetting::checked`]): giá trị sai bị từ chối lúc đặt
//!   (422). Module đọc bằng [`parsed_setting`]: giá trị đã lưu mà nay không còn hợp lệ (bộ kiểm
//!   chặt hơn ở bản mới) thì ghi cảnh báo và dùng cấu hình chung — không làm hỏng request.
//! - Đọc qua cache theo khách (một câu truy vấn lấy mọi cấu hình của khách, giữ
//!   `AUTOXIDE_TENANCY_CACHE_SECONDS`): module đọc ở đường nóng (ngôn ngữ của `content` mỗi
//!   request) không thêm truy vấn. Đặt / bỏ trên máy chủ này xoá cache ngay; máy chủ khác thấy
//!   trong TTL.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use validator::Validate;
use zeroize::Zeroizing;

use super::{DatabaseRegistry, ScopeExt, TenantContext, TenantId, TenantScoped, TenantStatus};
use crate::access::{Permission, Principal, Risk};
use crate::error::{Error, FieldError};
use crate::event::{Actor, Event, EventInfo, Outbox};
use crate::mcp::{NoInput, Tool};
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
    "Public URL of this tenant's site, e.g. `https://shop.example`, used in links sent by mail. \
     When unset, the server-wide AUTOXIDE_HTTP_PUBLIC_URL is used.",
);

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

/// Một cấu hình module nhận theo khách.
#[derive(Debug, Clone, Copy)]
pub struct TenantSetting {
    name: &'static str,
    description: &'static str,
    secret: bool,
    check: Option<SettingCheck>,
}

// So theo tên (tên là duy nhất trong sổ); không so con trỏ hàm của bộ kiểm.
impl PartialEq for TenantSetting {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.secret == other.secret
    }
}

impl Eq for TenantSetting {}

impl TenantSetting {
    /// Cấu hình `name` (`<module>.<khoá>`: chữ thường, số, `_`), mô tả tiếng Anh cho người quản trị.
    pub const fn new(name: &'static str, description: &'static str) -> Self {
        TenantSetting { name, description, secret: false, check: None }
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

    pub fn name(&self) -> &'static str {
        self.name
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

    fn find(&self, name: &str) -> Result<TenantSetting, Error> {
        self.all()
            .iter()
            .find(|setting| setting.name == name)
            .copied()
            .ok_or_else(|| super::errors::tenant_setting_unknown(name))
    }
}

/// Kiểm khai báo của module `module`: tên `<module>.<khoá>`, không trùng.
pub(crate) fn declaration_problems(
    module: &str,
    settings: &[TenantSetting],
    seen: &mut Vec<&'static str>,
) -> Vec<String> {
    let mut problems = Vec::new();
    for setting in settings {
        let valid = setting
            .name
            .strip_prefix(module)
            .and_then(|rest| rest.strip_prefix('.'))
            .is_some_and(crate::module::is_identifier)
            && setting.name.len() <= 100;
        if !valid {
            problems.push(format!(
                "tenant setting `{}` of module `{module}` must be named `{module}.<key>` (lower \
                 case letters, digits and `_`, at most 100 characters)",
                setting.name
            ));
        }
        if setting.description.trim().is_empty() {
            problems.push(format!("tenant setting `{}` has no description", setting.name));
        }
        if seen.contains(&setting.name) {
            problems.push(format!("tenant setting `{}` is declared twice", setting.name));
        }
        seen.push(setting.name);
    }
    problems
}

mod entity {
    //! Bảng `kernel_tenant_settings`.
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
        pub updated_at: DateTimeUtc,
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
    /// `<module>.<key>`, e.g. `mail.smtp_url`.
    #[schema(example = "kernel.public_url")]
    name: String,
    description: String,
    /// Secret settings are stored encrypted and never returned: `value` is always `null`.
    secret: bool,
    /// Whether this tenant has set it; otherwise the server-wide configuration applies.
    is_set: bool,
    /// The value, for settings that are not secret and are set.
    value: Option<String>,
    updated_at: Option<DateTime<Utc>>,
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

/// Mục đích (AAD) khi mã hoá giá trị secret.
const PURPOSE: &str = "kernel.tenant_setting";

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

    /// Mọi cấu hình đã khai, theo tên, với trạng thái ở khách `tenant`.
    pub(crate) async fn list(&self, tenant: &TenantContext) -> Result<Vec<SettingState>, Error> {
        // Trang quản trị đọc thẳng database: thấy ngay giá trị vừa đặt ở máy chủ khác.
        let rows = Entity::find().scoped(tenant).all(tenant.database().read()).await?;
        let mut states: Vec<SettingState> = self
            .catalog
            .all()
            .iter()
            .map(|setting| {
                let row = rows.iter().find(|row| row.name == setting.name);
                SettingState {
                    name: setting.name.to_owned(),
                    description: setting.description.to_owned(),
                    secret: setting.secret,
                    is_set: row.is_some(),
                    value: row.filter(|_| !setting.secret).map(|row| row.value.clone()),
                    updated_at: row.map(|row| row.updated_at),
                }
            })
            .collect();
        states.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(states)
    }

    /// Đặt cấu hình `name` của khách `tenant` (ghi đè nếu có).
    pub(crate) async fn put(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        name: &str,
        value: &str,
    ) -> Result<SettingState, Error> {
        let setting = self.catalog.find(name)?;
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
        let now = Utc::now().trunc_subsecs(6);
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
        let event = SettingChanged { name: name.to_owned(), set: true };
        self.outbox
            .record(&transaction, tenant, Actor::from(principal), Some(subject(name)), &event)
            .await?;
        transaction.commit().await?;
        self.forget(tenant.id());
        tracing::info!(setting = name, tenant_id = %tenant.id(), "tenant setting changed");
        Ok(SettingState {
            name: name.to_owned(),
            description: setting.description.to_owned(),
            secret: setting.secret,
            is_set: true,
            value: (!setting.secret).then(|| value.to_owned()),
            updated_at: Some(now),
        })
    }

    /// Bỏ cấu hình `name` của khách `tenant` (về cấu hình chung). Không có thì không đổi gì.
    pub(crate) async fn delete(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        name: &str,
    ) -> Result<(), Error> {
        self.catalog.find(name)?;
        let transaction = tenant.database().begin_write().await?;
        let deleted = Entity::delete_many()
            .filter(Column::Name.eq(name))
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
        let Some(row) = rows.iter().find(|row| row.name == setting.name) else {
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

/// List this tenant's settings.
///
/// Requires `kernel.settings.manage`. Every setting the installed modules take per tenant, with
/// whether this tenant has set it; unset ones use the server-wide configuration. Values of
/// secret settings are never returned.
#[utoipa::path(
    get,
    path = "/settings",
    extensions(("x-permissions" = json!(["kernel.settings.manage"]))),
    operation_id = "list_tenant_settings",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Every setting, sorted by name.", body = Data<Vec<SettingState>>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.settings.manage`; code `permission_denied`.", body = Problem),
    )
)]
async fn list_settings(
    State(service): State<Arc<SettingsService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Vec<SettingState>>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    Ok(Data { data: service.list(&tenant).await? })
}

/// Set one of this tenant's settings.
///
/// Requires `kernel.settings.manage`. Creates or replaces the value; sending it again is safe.
/// A secret value is stored encrypted and the answer does not contain it.
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
/// returned.
#[utoipa::path(
    get,
    path = "/tenants/{id}/settings",
    extensions(("x-permissions" = json!(["kernel.tenants.read"]))),
    operation_id = "list_settings_of_tenant",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id or slug of the tenant.")),
    responses(
        (status = 200, description = "Every setting, sorted by name.", body = Data<Vec<SettingState>>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tenants.read` (`permission_denied`), or not an account of the platform tenant (`tenant_platform_only`).", body = Problem),
        (status = 404, description = "No such tenant; code `tenant_not_found`.", body = Problem),
    )
)]
async fn list_tenant_settings(
    State(platform): State<PlatformSettings>,
    principal: Principal,
    Path(id): Path<String>,
) -> Result<Data<Vec<SettingState>>, Error> {
    principal.require(&super::TENANTS_READ)?;
    super::require_platform(&principal)?;
    let tenant = platform.tenant(&id, false).await?;
    Ok(Data { data: platform.service.list(&tenant).await? })
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

/// Tool MCP — chỉ đọc; không có tool ghi secret (đặc tả 33).
pub(crate) fn tools(service: Arc<SettingsService>) -> Vec<Tool> {
    vec![
        Tool::new(
            "list_settings",
            "List the settings of the current tenant: every setting the installed modules take \
             per tenant, whether it is set, and its value unless it is secret.",
            move |tenant: TenantContext, _principal: Principal, _: NoInput| {
                let service = service.clone();
                async move { service.list(&tenant).await }
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
