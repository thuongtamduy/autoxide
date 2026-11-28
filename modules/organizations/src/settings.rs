//! Cài đặt kế thừa theo cây (bước 34 mục 4): ba cài đặt chung do module khai (múi giờ, tiền tệ,
//! ngôn ngữ), và service lưu / đọc giá trị theo nút cho MỌI cài đặt module khác khai
//! (`Module::org_settings`). Giá trị hiệu lực của một nút = giá trị của nút gần nhất trên chuỗi
//! tổ tiên (gồm chính nó) có đặt; không nút nào đặt → mặc định của cài đặt.

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::{
    OrgId, OrgSetting, OrgSettingStore, ScopeExt, SettingSystem, StoredOrgSetting, TenantContext,
};
use autoxide_kernel::time::Timestamp;
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::EffectiveSetting;
use crate::entity::setting;
use crate::events::{self, Change};
use crate::permissions::{SETTINGS_WRITE, UNITS_READ};
use crate::service::OrgService;

/// Múi giờ IANA của nút (giờ mở cửa, báo cáo theo ngày, nhắc lịch).
pub(crate) const TIMEZONE: OrgSetting = OrgSetting::new(
    "organizations.timezone",
    "IANA time zone of the unit, e.g. `Asia/Ho_Chi_Minh`: opening hours, daily reports and \
     reminders use it.",
)
.with_default("\"UTC\"")
.with_schema(r#"{"type":"string","format":"iana-time-zone","examples":["Asia/Ho_Chi_Minh"]}"#)
.checked(time_zone);

/// Tiền tệ của nút (ISO 4217).
pub(crate) const CURRENCY: OrgSetting = OrgSetting::new(
    "organizations.currency",
    "Currency of the unit (ISO 4217 code, e.g. `VND`): prices and amounts are in it.",
)
.with_schema(r#"{"type":"string","pattern":"^[A-Z]{3}$","examples":["VND"]}"#)
.checked(currency);

/// Ngôn ngữ của nút (thư gửi khách, giao diện mặc định).
pub(crate) const LANGUAGE: OrgSetting = OrgSetting::new(
    "organizations.language",
    "Language of the unit (`en` or `vi`): mails to its customers and its default interface \
     language.",
)
.with_default("\"en\"")
.with_schema(r#"{"type":"string","enum":["en","vi"]}"#)
.checked(language);

/// Ngôn ngữ website của nút đưa ra (D-307): `content`, `menus` đọc qua `OrgDirectory::setting`.
pub(crate) const LANGUAGES: OrgSetting = OrgSetting::new(
    "organizations.languages",
    "Languages the unit's website offers, the first being its default, as a list of BCP 47 codes \
     (e.g. `[\"vi\", \"en\"]`). Unset: the tenant's languages. Content written in other languages \
     is not shown on that website.",
)
.with_schema(r#"{"type":"array","items":{"type":"string"},"minItems":1,"uniqueItems":true}"#)
.checked(languages);

pub(crate) fn all() -> Vec<OrgSetting> {
    let mut all = vec![TIMEZONE, CURRENCY, LANGUAGE, LANGUAGES];
    // Thương hiệu của website (bước 55): kế thừa theo cây như mọi cài đặt.
    all.extend(crate::storefront::settings());
    all
}

fn languages(value: &Value) -> Result<Value, String> {
    let wrong = || "must be a non-empty list of language codes like `vi` or `pt-BR`".to_owned();
    let items = value.as_array().filter(|items| !items.is_empty()).ok_or_else(wrong)?;
    let mut found: Vec<String> = Vec::new();
    for item in items {
        let code = item
            .as_str()
            .and_then(|raw| autoxide_kernel::i18n::canonical_locale(raw.trim()))
            .ok_or_else(wrong)?;
        if !found.contains(&code) {
            found.push(code);
        }
    }
    Ok(Value::from(found))
}

fn text(value: &Value) -> Result<&str, String> {
    value.as_str().map(str::trim).ok_or_else(|| "must be a string".to_owned())
}

fn time_zone(value: &Value) -> Result<Value, String> {
    let raw = text(value)?;
    autoxide_kernel::time::zone(raw).map(|_| Value::from(raw)).ok_or_else(|| {
        "must be an IANA time zone name, e.g. `Asia/Ho_Chi_Minh` or `UTC`".to_owned()
    })
}

fn currency(value: &Value) -> Result<Value, String> {
    let raw = text(value)?;
    autoxide_kernel::money::Currency::parse(raw)
        .map(|currency| Value::from(currency.code()))
        .map_err(|_| "must be an ISO 4217 currency code, e.g. `VND` or `USD`".to_owned())
}

fn language(value: &Value) -> Result<Value, String> {
    let raw = text(value)?.to_ascii_lowercase();
    let known = [autoxide_kernel::i18n::Language::En, autoxide_kernel::i18n::Language::Vi];
    known
        .iter()
        .find(|language| language.tag() == raw)
        .map(|language| Value::from(language.tag()))
        .ok_or_else(|| "must be `en` or `vi`".to_owned())
}

impl OrgService {
    /// Giá trị hiệu lực của mọi cài đặt ở nút `unit_id` (cần `organizations.units.read` phủ nút).
    pub(crate) async fn effective_settings(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        unit_id: Uuid,
    ) -> Result<Vec<EffectiveSetting>, Error> {
        principal.require(&UNITS_READ)?;
        self.get(tenant, principal, unit_id).await?;
        self.resolve_settings(tenant, OrgId::from_uuid(unit_id), None).await
    }

    /// Giá trị hiệu lực (một cài đặt `only`, hay mọi cài đặt) ở nút `org`, qua hệ cài đặt của kernel (cache theo
    /// khách: lần đọc trúng cache không chạy câu nào).
    pub(crate) async fn resolve_settings(
        &self,
        tenant: &TenantContext,
        org: OrgId,
        only: Option<&str>,
    ) -> Result<Vec<EffectiveSetting>, Error> {
        let settings = self.setting_system()?;
        let found = match only {
            Some(key) => settings.effective(tenant, org, key).await?.into_iter().collect(),
            None => settings.effective_all(tenant, org).await?,
        };
        Ok(found
            .into_iter()
            .map(|effective| EffectiveSetting {
                key: effective.key,
                value: effective.value,
                source_unit_id: effective.source,
                inherited: effective.inherited,
            })
            .collect())
    }

    fn setting_system(&self) -> Result<&SettingSystem, Error> {
        self.settings.as_ref().ok_or_else(|| {
            Error::internal("module `organizations` needs the kernel's settings system")
        })
    }

    /// Đặt cài đặt `key` của nút `unit_id` thành `value` (cần `organizations.settings.write` phủ
    /// nút; nút phải hoạt động). Trả giá trị hiệu lực mới.
    pub(crate) async fn set_setting(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        unit_id: Uuid,
        key: &str,
        value: Option<Value>,
    ) -> Result<EffectiveSetting, Error> {
        principal.require(&SETTINGS_WRITE)?;
        let declared = self.catalog.get(key).copied().ok_or_else(|| setting_unknown(key))?;
        let checked = match &value {
            Some(value) => {
                Some(declared.check(value).map_err(|reason| setting_invalid(key, &reason))?)
            }
            None => None,
        };
        self.get(tenant, principal, unit_id).await?;
        let org = OrgId::from_uuid(unit_id);
        let transaction = tenant.database().begin_write().await?;
        principal
            .require_write_at(&transaction, &SETTINGS_WRITE, OrgTarget::new(org, None))
            .await?;
        // Luật chéo của kernel (vd lý do cần ảnh ⊆ lý do được phép): khoá theo khách rồi đọc giá trị trong
        // transaction, nên hai lần lưu đồng thời không cùng lọt luật.
        let system = self.setting_system()?;
        if system.has_rules(key) {
            system.lock(&transaction, tenant).await?;
            let stored = stored(&transaction, tenant).await?;
            system
                .check_org_change(&transaction, tenant, org, key, checked.as_ref(), stored)
                .await?;
        }
        let before = setting::Entity::find_by_id((tenant.id().as_uuid(), unit_id, key.to_owned()))
            .scoped(tenant)
            .one(&transaction)
            .await?;
        match &checked {
            Some(value) => {
                let row = setting::ActiveModel {
                    tenant_id: Set(tenant.id().as_uuid()),
                    org_id: Set(unit_id),
                    key: Set(key.to_owned()),
                    value: Set(value.to_string()),
                    updated_by: Set(Some(principal.user_id())),
                    updated_at: Set(Timestamp::now()),
                };
                let conflict = OnConflict::columns([
                    setting::Column::TenantId,
                    setting::Column::OrgId,
                    setting::Column::Key,
                ])
                .update_columns([
                    setting::Column::Value,
                    setting::Column::UpdatedBy,
                    setting::Column::UpdatedAt,
                ])
                .to_owned();
                setting::Entity::insert(row)
                    .on_conflict(conflict)
                    .exec_without_returning(&transaction)
                    .await?;
            }
            None => {
                setting::Entity::delete_many()
                    .filter(setting::Column::OrgId.eq(unit_id))
                    .filter(setting::Column::Key.eq(key))
                    .scoped(tenant)
                    .exec(&transaction)
                    .await?;
            }
        }
        let old = before.and_then(|row| serde_json::from_str::<Value>(&row.value).ok());
        let change = Change {
            before: Some(json!({"key": key, "value": old})),
            after: Some(json!({"key": key, "value": checked})),
        };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::unit_subject(unit_id),
                &events::SettingChanged(change),
            )
            .await?;
        transaction.commit().await?;
        system.changed(tenant.id());
        let effective = self.resolve_settings(tenant, org, Some(key)).await?;
        effective.into_iter().next().ok_or_else(|| setting_unknown(key))
    }
}

/// Kho giá trị theo nút cắm vào hệ cài đặt của kernel: mọi giá trị của một khách trong một câu.
pub(crate) struct Store;

#[async_trait::async_trait]
impl OrgSettingStore for Store {
    async fn stored(&self, tenant: &TenantContext) -> Result<Vec<StoredOrgSetting>, Error> {
        stored(tenant.database().read(), tenant).await
    }
}

// Mọi giá trị đã đặt của khách, đọc qua `connection` (kết nối đọc, hay transaction của lần lưu).
async fn stored<C: sea_orm::ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
) -> Result<Vec<StoredOrgSetting>, Error> {
    let rows = setting::Entity::find().scoped(tenant).all(connection).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let value = serde_json::from_str(&row.value).ok()?;
            Some(StoredOrgSetting::new(OrgId::from_uuid(row.org_id), row.key, value))
        })
        .collect())
}

pub(crate) fn setting_unknown(key: &str) -> Error {
    Error::not_found(
        "organizations.setting_unknown",
        "There is no setting `{key}`; see `GET /api/v1/organizations/settings`.",
    )
    .with_param("key", key)
}

fn setting_invalid(key: &str, reason: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "organizations.setting_invalid",
        "The value of `{key}` is invalid: {reason}",
    )
    .with_param("key", key)
    .with_param("reason", reason)
}
