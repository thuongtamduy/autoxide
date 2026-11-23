//! Per-tenant configuration and secrets (spec 33 "Per-tenant configuration and secrets", D-258).
//!
//! - A module declares the per-tenant configuration it accepts through `Module::tenant_settings`
//!   ([`TenantSetting`], name `<module>.<key>`, e.g. `comments.max_links`); the kernel declares
//!   `kernel.public_url`.
//! - A setting can be placed in a group ([`TenantSetting::group`]). The [`CONNECTION_GROUP`] group is
//!   connections to partners (SMTP server, AI provider, payment gateway…): the name must have the form
//!   `<module>.connection.<kind>.<field>` (e.g. `mail.connection.smtp.url`), and a secret field is declared
//!   with `.secret()`. `GET /api/v1/kernel/settings?group=connection` is the source of the "Partner
//!   connections" screen.
//! - Renaming a setting: declare the old name with [`TenantSetting::formerly`]. A value the tenant set under
//!   the old name is still read (a secret is opened with the exact name it was encrypted with), the API still
//!   accepts the old name; the next set writes under the new name and removes the old-name row. No data
//!   upgrade needed.
//! - Modules read through the [`TenantSettings`] trait (`context.require::<dyn TenantSettings>()`). When a
//!   tenant has not set a value, the module uses the shared configuration (environment variables) as before.
//! - Stored in the table `kernel_tenant_settings` (tenant level, in the tenant's database); a secret is
//!   encrypted with [`Sealer`](crate::secret::Sealer) and is **never** returned in the API, logs or events:
//!   the event `kernel.setting_changed` carries only the name.
//! - Managed through `GET /api/v1/kernel/settings`, `PUT` / `DELETE /api/v1/kernel/settings/{name}`
//!   (permission `kernel.settings.manage`) and the tool `kernel.list_settings`. Platform staff set values for
//!   any tenant through `…/kernel/tenants/{id}/settings…` (D-210, cross-check design 2).
//! - A setting can declare a checker ([`TenantSetting::checked`]): an invalid value is rejected when set (422).
//!   Modules read with [`parsed_setting`]: a stored value that is no longer valid (a stricter checker in a new
//!   version) logs a warning and uses the shared configuration, without breaking the request.
//! - Reads go through a per-tenant cache (one query fetches every setting of the tenant, kept for
//!   `AUTOXIDE_TENANCY_CACHE_SECONDS`): a module that reads on a hot path (the `content` language on every
//!   request) adds no query. A set / remove on this server clears the cache at once; other servers see it
//!   within the TTL.

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
use crate::migration::prelude::*;
use crate::response::{Data, Problem};
use crate::secret::SecretKey;

/// Reads / writes the per-tenant configuration of the caller's own tenant.
pub const SETTINGS_MANAGE: Permission = Permission::new(
    "kernel.settings.manage",
    "Read and change the settings of this tenant (mail server, AI provider keys…); secret values \
     can be replaced but never read back.",
)
.with_risk(Risk::SensitiveData);

/// Public URL of the tenant (links in emails…); when unset, `AUTOXIDE_HTTP_PUBLIC_URL`.
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

/// The `scheme://host[:port]` head of an http(s) address; `None` when it is not one.
fn origin_of(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let host_len = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    if host_len == 0 || rest[..host_len].contains(['@', ' ']) {
        return None;
    }
    Some(&url[..url.len() - rest.len() + host_len])
}

/// Per-tenant links in mail (UI brief): when a tenant has set `kernel.public_url`, the link opens on the
/// tenant's domain, keeping the path of the server address and replacing only the head. Cheap to clone; the
/// default (no per-tenant configuration, e.g. hand-built tests) returns the address unchanged.
#[derive(Clone, Default)]
pub struct SiteLinks(Option<Arc<dyn TenantSettings>>);

impl SiteLinks {
    pub fn new(settings: Arc<dyn TenantSettings>) -> Self {
        SiteLinks(Some(settings))
    }

    /// `template` (the server's configured address, may contain `{token}`) for tenant `tenant`.
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

/// Replaces the head of `template` with `site`; a `template` that is not an http(s) address stays unchanged.
fn rebase(template: &str, site: &str) -> String {
    let site = site.trim().trim_end_matches('/');
    match (origin_of(template), origin_of(site)) {
        (Some(origin), Some(_)) => format!("{site}{}", &template[origin.len()..]),
        _ => template.to_owned(),
    }
}

/// Business model of a tenant (D-185): the UI picks the setup steps and default screen accordingly.
/// Informational only; what may be used is still decided by the plan (step 35).
pub const BUSINESS_MODEL: TenantSetting = TenantSetting::new(
    "kernel.business_model",
    "How this tenant does business, so interfaces show the right setup steps and screens: \
     `independent` (one shop), `multi_store` (one business, several stores), `chain` (a parent \
     with child tenants), `marketplace` (sellers sell on the tenant's site) or `saas` (the tenant \
     sells software by plan). Information only: what the tenant may use comes from its plan.",
)
.checked(business_model);

/// Values accepted by `kernel.business_model`.
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

/// Event: a tenant setting was just set or removed. Carries only the name, never the value.
pub(crate) const SETTING_CHANGED: EventInfo = EventInfo::new(
    "kernel.setting_changed",
    "A setting of the tenant was set or removed; carries only its name.",
);

/// Maximum length of one value.
const VALUE_MAX: usize = 4000;

/// Value checker of a setting: `Err` with an error for the `value` field (the message template has a
/// translation like every `FieldError`).
pub type SettingCheck = fn(&str) -> Result<(), FieldError>;

/// Group of settings for connections to partners (cross-check design batch 2): the name must have the form
/// `<module>.connection.<kind>.<field>`.
pub const CONNECTION_GROUP: &str = "connection";

/// A setting a module accepts per tenant.
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
    /// Setting `name` (`<module>.<key>`; the key is one or more segments of lowercase letters, digits, `_`
    /// joined by dots), with an English description for administrators.
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

    /// The value is a secret: stored encrypted, never returned.
    pub const fn secret(mut self) -> Self {
        self.secret = true;
        self
    }

    /// The value must pass `check` when set (422 `validation_failed`, the error on the `value` field). A module
    /// uses the same value-parsing function for the checker and for [`parsed_setting`].
    pub const fn checked(mut self, check: SettingCheck) -> Self {
        self.check = Some(check);
        self
    }

    /// Places the setting in group `group` (lowercase, digits, `_`): the list can be filtered by group
    /// (`?group=`). The [`CONNECTION_GROUP`] group requires names of the form
    /// `<module>.connection.<kind>.<field>`.
    pub const fn group(mut self, group: &'static str) -> Self {
        self.group = Some(group);
        self
    }

    /// This setting used to be named `name`: a value the tenant set under the old name is still read, the API
    /// still accepts the old name; the next set writes under the new name.
    pub const fn formerly(mut self, name: &'static str) -> Self {
        self.former_name = Some(name);
        self
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The setting's group, if any.
    pub fn group_name(&self) -> Option<&'static str> {
        self.group
    }

    /// The setting's former name, if it was renamed.
    pub fn former_name(&self) -> Option<&'static str> {
        self.former_name
    }

    /// `name` is the name (current or former) of this setting.
    fn answers_to(&self, name: &str) -> bool {
        self.name == name || self.former_name == Some(name)
    }

    /// The stored row of this setting in `rows`: the current-name row first, otherwise the former-name row.
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

/// The value of a tenant setting. `Debug` does not print the secret; memory is wiped on drop.
#[derive(Clone)]
pub struct SettingValue {
    value: Zeroizing<String>,
    secret: bool,
}

impl SettingValue {
    /// A non-secret value (for fakes in tests; modules read through [`TenantSettings`]).
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

/// The value of `setting` for `tenant`, parsed with `parse`; `None` when the tenant has not set it. A stored
/// value that `parse` rejects (e.g. the checker of a newer version is stricter) → a warning in the log and
/// `None`: the module uses the shared configuration instead of breaking the tenant's request.
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

/// Reads a tenant's settings. Published by the `kernel` module.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait TenantSettings: Send + Sync {
    /// The value of `setting` for tenant `tenant`; `None` when the tenant has not set it.
    async fn get(
        &self,
        tenant: &TenantContext,
        setting: &TenantSetting,
    ) -> Result<Option<SettingValue>, Error>;
}

/// Registry of the declared settings of every module, filled once when `Modules` is built. Cheap to clone.
#[derive(Clone, Default)]
pub(crate) struct Catalog(Arc<OnceLock<Vec<TenantSetting>>>);

impl Catalog {
    pub(crate) fn fill(&self, settings: Vec<TenantSetting>) {
        let _ = self.0.set(settings);
    }

    fn all(&self) -> &[TenantSetting] {
        self.0.get().map_or(&[], Vec::as_slice)
    }

    /// The setting named `name`, current or former ([`TenantSetting::formerly`]).
    fn find(&self, name: &str) -> Result<TenantSetting, Error> {
        self.all()
            .iter()
            .find(|setting| setting.answers_to(name))
            .copied()
            .ok_or_else(|| super::errors::tenant_setting_unknown(name))
    }
}

/// `name` has the form `<module>.<key>`; the key is one or more segments (lowercase, digits, `_`) joined by `.`.
fn well_named(module: &str, name: &str) -> bool {
    name.len() <= 100
        && name
            .strip_prefix(module)
            .and_then(|rest| rest.strip_prefix('.'))
            .is_some_and(|key| key.split('.').all(crate::module::is_identifier))
}

/// Checks the declarations of module `module`: name `<module>.<key>`, no duplicates (former names included);
/// settings of the `connection` group, and only they, have the name `<module>.connection.<kind>.<field>`.
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

/// Copies every NON-secret setting from `source` to `target` (sandbox, D-232): a secret (partner connection
/// information) never crosses; the sandbox uses fakes. A setting `target` already has is kept. Returns the
/// number of new rows.
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
    //! The `kernel_tenant_settings` table.
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
        /// Plain text for ordinary settings, `sealed:…` for secrets.
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

/// The stored settings of one tenant (secret values still encrypted), kept in the cache.
type Stored = Arc<Vec<entity::Model>>;

/// When the cache holds more tenants than this, expired entries are dropped when a new entry is added.
const CACHE_PRUNE_AT: usize = 10_000;

/// Per-tenant settings service.
pub(crate) struct SettingsService {
    catalog: Catalog,
    secret: SecretKey,
    outbox: Outbox,
    /// How long the cache is kept (`AUTOXIDE_TENANCY_CACHE_SECONDS`).
    ttl: Duration,
    cache: Mutex<HashMap<TenantId, (Instant, Stored)>>,
}

/// Purpose (AAD) when encrypting a secret value. The row name is also in the AAD: a secret set under the old
/// name of a setting can only be opened with that old name, so always open with `row.name`.
pub(crate) const PURPOSE: &str = "kernel.tenant_setting";

/// State of `setting` at a tenant, from the stored row `row` (if the tenant has set it).
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

    /// Every stored setting of tenant `tenant`, through the cache. The `mutex` lock is held only within each
    /// statement, never across an `await`.
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

    /// Drops tenant `tenant`'s settings from the cache (after a set / remove on this server).
    fn forget(&self, tenant: TenantId) {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner).remove(&tenant);
    }

    /// Every declared setting, by name, with its state at tenant `tenant`; `group`: only the settings of that
    /// group (a group nobody declares: an empty list).
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

    /// Sets setting `name` of tenant `tenant` (overwriting if present). If `name` is the former name of a
    /// setting, sets that setting itself, under the current name.
    pub(crate) async fn put(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        name: &str,
        value: &str,
    ) -> Result<SettingState, Error> {
        let setting = self.catalog.find(name)?;
        connection_step_up(&setting, principal)?;
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

    /// Removes setting `name` of tenant `tenant` (back to the shared configuration), including a value set
    /// under a former name. Changes nothing if it is absent. Like [`put`](Self::put) but rejects secret
    /// settings (MCP tool): a secret is set through REST only.
    pub(crate) async fn put_plain(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        name: &str,
        value: &str,
    ) -> Result<SettingState, Error> {
        if self.catalog.find(name)?.secret {
            return Err(Error::forbidden(
                "kernel.setting_secret_rest_only",
                "Setting `{name}` is secret: set it with `PUT /api/v1/kernel/settings/{name}`, \
                 not through a tool.",
            )
            .with_param("name", name));
        }
        self.put(tenant, principal, name, value).await
    }

    pub(crate) async fn delete(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        name: &str,
    ) -> Result<(), Error> {
        let setting = self.catalog.find(name)?;
        connection_step_up(&setting, principal)?;
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

/// Setting / removing a field of a partner connection (group `connection`) changes where money arrives or
/// where data goes: it requires the session's recent authentication (step-up) like a sensitive action, even
/// though `kernel.settings.manage` is not a high-risk kind (D-20261122-provider-environments). Ordinary
/// settings are not required to.
fn connection_step_up(setting: &TenantSetting, principal: &Principal) -> Result<(), Error> {
    if setting.group == Some(CONNECTION_GROUP) {
        principal.require_fresh(&SETTINGS_MANAGE)?;
    }
    Ok(())
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
        (status = 401, description = "Not signed in (code `unauthorized` or the code of the authentication module), or a setting of the `connection` group needs a recent sign-in: code `step_up_required`, confirm with `POST /api/v1/auth/me/reauth` and retry.", body = Problem),
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
        (status = 401, description = "Not signed in (code `unauthorized` or the code of the authentication module), or a setting of the `connection` group needs a recent sign-in: code `step_up_required`, confirm with `POST /api/v1/auth/me/reauth` and retry.", body = Problem),
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

/// What the platform staff handlers need: the service and the database registry (find a tenant by ID / slug).
#[derive(Clone)]
pub(crate) struct PlatformSettings {
    service: Arc<SettingsService>,
    registry: DatabaseRegistry,
}

impl PlatformSettings {
    /// Tenant `id_or_slug`. Being moved: a write now would go to the old database → `tenant_moving`.
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
        (status = 401, description = "Not signed in (code `unauthorized` or the code of the authentication module), or a setting of the `connection` group needs a recent sign-in: code `step_up_required`, confirm with `POST /api/v1/auth/me/reauth` and retry.", body = Problem),
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
        (status = 401, description = "Not signed in (code `unauthorized` or the code of the authentication module), or a setting of the `connection` group needs a recent sign-in: code `step_up_required`, confirm with `POST /api/v1/auth/me/reauth` and retry.", body = Problem),
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

/// Routes under `/api/v1/kernel`.
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

mod tools;
pub(crate) use tools::tools;

/// The `kernel_tenant_settings` table (tenant level, has `tenant_id` from the start).
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
