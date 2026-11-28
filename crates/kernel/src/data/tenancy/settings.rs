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
     their path, only the start changes. When unset, the server's addresses are used as set. \
     With several tenants it must be `https`, on the default port, on a domain this tenant has \
     verified (or the platform's own host); a value stored before that no longer meets this is \
     ignored and flagged with `problem`. With one tenant, `https` except on `localhost`.",
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
    if host_len == 0 || rest[..host_len].contains(['@', ' ', '{', '}']) {
        return None;
    }
    Some(&url[..url.len() - rest.len() + host_len])
}

/// Business model of a tenant (D-185): the UI picks the setup steps and default screen accordingly.
/// What may be used is still decided by the plan (step 35); `marketplace` also turns on the seller
/// identity check before listing (D-20261127-seller-kyc-listing).
pub const BUSINESS_MODEL: TenantSetting = TenantSetting::new(
    "kernel.business_model",
    "How this tenant does business, so interfaces show the right setup steps and screens: \
     `independent` (one shop), `multi_store` (one business, several stores), `chain` (a parent \
     with child tenants), `marketplace` (sellers sell on the tenant's site) or `saas` (the tenant \
     sells software by plan). What the tenant may use still comes from its plan; `marketplace` also makes \
     `catalog` require a verified identity of a seller before listing its goods.",
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
    /// Placeholders a link setting must contain ([`TenantSetting::link`]).
    placeholders: &'static [&'static str],
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
            placeholders: &[],
        }
    }

    /// A link in mail the tenant may point at its own page (group [`LINK_GROUP`], see [`SiteLinks::link`]):
    /// the value is a whole `https://` address on one of the tenant's hosts that contains every one of
    /// `placeholders` (e.g. `{token}`). Name it `<module>.link.<key>`.
    pub const fn link(
        name: &'static str,
        description: &'static str,
        placeholders: &'static [&'static str],
    ) -> Self {
        let mut setting = TenantSetting::new(name, description).checked(links::link_template);
        setting.group = Some(LINK_GROUP);
        setting.placeholders = placeholders;
        setting
    }

    /// A domain of the tenant's own (group [`DOMAIN_GROUP`], see [`SiteLinks::own_domain`]): a bare domain name
    /// that, in multi-tenant mode, this tenant has verified (`POST /api/v1/kernel/domains`) — checked when set
    /// and again every time it is used, so a domain the tenant loses or that moves to another tenant stops
    /// counting at once. Name it `<module>.<key>`.
    pub const fn own_domain(name: &'static str, description: &'static str) -> Self {
        let mut setting = TenantSetting::new(name, description).checked(links::domain_name);
        setting.group = Some(DOMAIN_GROUP);
        setting
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
    /// The stored value is not used any more, and why: `kernel.public_url_unverified` /
    /// `kernel.link_unverified` / `kernel.domain_unverified` (its host is not a verified domain of this
    /// tenant — verify the domain or change the value). Omitted when the value is used.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "kernel.public_url_unverified")]
    problem: Option<String>,
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
    /// What a link setting's host is checked against (filled once the domains service exists).
    links: OnceLock<LinkTrust>,
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
        problem: None,
    }
}

impl SettingsService {
    pub(crate) fn new(catalog: Catalog, secret: SecretKey, outbox: Outbox, ttl: Duration) -> Self {
        SettingsService {
            catalog,
            secret,
            outbox,
            ttl,
            cache: Mutex::default(),
            links: OnceLock::new(),
        }
    }

    /// Sets what link settings are checked against when set (once, at start).
    pub(crate) fn use_link_trust(&self, trust: LinkTrust) {
        let _ = self.links.set(trust);
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

    /// One page of [`list`](Self::list), by name. The settings are declared in code by the modules (a fixed
    /// set per build) and joined with the tenant's few stored values, so the page is cut in memory
    /// (D-20261125-list-pages item 4).
    pub(crate) async fn page(
        &self,
        tenant: &TenantContext,
        group: Option<&str>,
        page: &crate::pagination::PageRequest,
    ) -> Result<crate::pagination::Page<SettingState>, Error> {
        crate::pagination::page_of(
            self.list(tenant, group).await?,
            |state| state.name.clone(),
            page,
        )
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
        // Giá trị đã lưu mà nay không dùng được (tên miền chưa / không còn xác minh): báo cho quản trị.
        let trust = self.links.get().cloned().unwrap_or_default();
        for state in &mut states {
            if let (Some(value), Ok(setting)) = (&state.value, self.catalog.find(&state.name)) {
                // Không đọc được ảnh chụp tên miền: trang vẫn trả, chỉ bỏ trống cờ (gửi thư vẫn từ chối — fail closed).
                match links::stored_problem(&trust, self, tenant, &setting, value).await {
                    Ok(problem) => state.problem = problem.map(str::to_owned),
                    Err(error) => tracing::warn!(
                        %error,
                        setting = setting.name(),
                        "cannot check a stored setting against the verified domains"
                    ),
                }
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
        let trust = self.links.get().cloned().unwrap_or_default();
        if setting.group == Some(LINK_GROUP) {
            links::check_link(&trust, self, tenant, &setting, value)
                .await?
                .map_err(|problem| Error::validation(vec![problem]))?;
        }
        if setting.group == Some(DOMAIN_GROUP) {
            links::check_own_domain(&trust, tenant, value)
                .await?
                .map_err(|problem| Error::validation(vec![problem]))?;
        }
        if setting.name == PUBLIC_URL.name {
            links::check_public_url(&trust, tenant, value)
                .await?
                .map_err(|problem| Error::validation(vec![problem]))?;
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

mod catalog;
mod links;
mod rest;
mod tools;
pub(crate) use catalog::{Catalog, copy_plain, declaration_problems};
pub(crate) use rest::routes;
pub(crate) use links::LinkTrust;
pub use links::{DOMAIN_GROUP, LINK_GROUP, SiteLinks};
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
