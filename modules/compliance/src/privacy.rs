//! Dữ liệu cá nhân (Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15, NĐ 356/2025/NĐ-CP — cần đối chiếu văn bản
//! gốc / người có chuyên môn pháp lý; D-360):
//!
//! - **Sổ mục đích xử lý** và căn cứ xử lý: mặc định của quy tắc `privacy.default_purposes` cộng phần
//!   khách thêm / ghi đè (`compliance_purposes`); công khai ở `GET /public/purposes`.
//! - **Thời hạn lưu** theo loại dữ liệu: sổ `RetentionCatalog` của kernel (module khai qua
//!   `Module::retention`) đối chiếu giới hạn của quy tắc `privacy.retention_limits`.
//! - **Sổ hoạt động xử lý**: mục đích, căn cứ, loại dữ liệu, bên nhận, thời hạn, nơi lưu.
//! - **Nơi lưu** theo khách: đọc / đặt `data_residency` qua API kernel (chỉ nhân sự nền tảng đặt).
//! - Dữ liệu cá nhân module giữ về một người dùng: xuất (`PersonalData`) và xoá khi tài khoản bị xoá
//!   (`auth.user_deleted`) — số điện thoại đã xác thực và mã một lần (phần còn lại từ D-356).

use autoxide_kernel::time::Timestamp;
use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::doctor::{Finding, Probe};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Delivered, EventHandler};
use autoxide_kernel::pagination::{Page, PageRequest, page_of};
use autoxide_kernel::privacy::{DataCategory, Keep, PersonalData, deleted_user};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext, TenantId, require_platform};
use sea_orm::sea_query::{Expr, ExprTrait as _};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::{
    acceptance, account_verification, consent, kyc_profile, phone_challenge, purpose,
    subject_request,
};
use crate::events;
use crate::permissions::{PRIVACY_MANAGE, PRIVACY_READ};
use crate::service::{ComplianceService, now};

/// Hạng mục mục đích hợp lệ.
const CATEGORIES: [&str; 5] = ["service", "marketing", "analytics", "legal", "kyc"];
/// Căn cứ xử lý hợp lệ.
const LEGAL_BASES: [&str; 5] =
    ["consent", "contract", "legal_obligation", "vital_interest", "public_interest"];

/// Lời nhắc chung của mọi bản xuất pháp lý.
pub(crate) const DISCLAIMER: &str = "Generated from the configuration. Have a legal professional \
     review it before use (Personal Data Protection Law 91/2025/QH15, Decree 356/2025/ND-CP).";

pub(crate) fn purpose_not_found(key: &str) -> Error {
    Error::not_found("compliance.purpose_not_found", "There is no processing purpose `{key}`.")
        .with_param("key", key)
}

fn purpose_exists(key: &str) -> Error {
    Error::conflict(
        "compliance.purpose_exists",
        "The tenant already has its own purpose `{key}`; change it with `PATCH /purposes/{key}`.",
    )
    .with_param("key", key)
}

/// A party that receives personal data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceDataRecipient)]
pub(crate) struct Recipient {
    /// Who, e.g. `Google Analytics 4`.
    pub name: String,
    /// Country that receives the data (ISO 3166-1 alpha-2); data sent abroad when not `VN`.
    pub country: Option<String>,
    /// Where it comes from: `purpose` (set on the purpose) or `tracking` (an active tag).
    #[serde(default = "purpose_source")]
    pub source: String,
}

fn purpose_source() -> String {
    "purpose".to_owned()
}

/// A processing purpose, with its legal basis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = CompliancePurpose)]
pub(crate) struct Purpose {
    /// e.g. `marketing`.
    pub key: String,
    /// `service`, `marketing`, `analytics`, `legal` or `kyc`.
    pub category: String,
    /// `consent`, `contract`, `legal_obligation`, `vital_interest` or `public_interest`.
    pub legal_basis: String,
    /// Kinds of personal data processed, e.g. `["email", "name"]`.
    pub data_categories: Vec<String>,
    /// Who receives the data.
    pub recipients: Vec<Recipient>,
    /// Description by language, e.g. `{"en": "…", "vi": "…"}`.
    #[schema(value_type = Object)]
    pub description: BTreeMap<String, String>,
    /// How long the data is kept, in words.
    pub retention: Option<String>,
    pub active: bool,
    /// `default` (from the legal rules) or `tenant` (added or changed by the tenant).
    pub source: String,
}

/// A processing purpose with how many people consented to it and how many withdrew.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = CompliancePurposeWithCounts)]
pub(crate) struct PurposeWithCounts {
    #[serde(flatten)]
    pub purpose: Purpose,
    /// People (distinct email, phone or account) with a consent to this purpose now given, on at
    /// least one channel.
    #[schema(example = 1520)]
    pub consented: i64,
    /// People with a consent to this purpose withdrawn, on at least one channel.
    #[schema(example = 34)]
    pub withdrawn: i64,
}

/// Mục đích mặc định trong quy tắc.
#[derive(Debug, Deserialize)]
struct DefaultPurpose {
    key: String,
    category: String,
    legal_basis: String,
    #[serde(default)]
    data_categories: Vec<String>,
    #[serde(default)]
    description: BTreeMap<String, String>,
}

fn describe_purpose(row: &purpose::Model) -> Purpose {
    Purpose {
        key: row.key.clone(),
        category: row.category.clone(),
        legal_basis: row.legal_basis.clone(),
        data_categories: serde_json::from_str(&row.data_categories).unwrap_or_default(),
        recipients: serde_json::from_str(&row.recipients).unwrap_or_default(),
        description: serde_json::from_str(&row.description).unwrap_or_default(),
        retention: row.retention.clone(),
        active: row.active,
        source: "tenant".to_owned(),
    }
}

/// Mục đích mới hay thay đổi (mọi trường tuỳ chọn khi sửa).
#[derive(Debug, Clone, Default)]
pub(crate) struct PurposeInput {
    pub key: Option<String>,
    pub category: Option<String>,
    pub legal_basis: Option<String>,
    pub data_categories: Option<Vec<String>>,
    pub recipients: Option<Vec<Recipient>>,
    pub description: Option<BTreeMap<String, String>>,
    pub retention: Option<Option<String>>,
    pub active: Option<bool>,
}

fn check_purpose(input: &PurposeInput, creating: bool, problems: &mut Vec<FieldError>) {
    if creating || input.key.is_some() {
        let key = input.key.as_deref().unwrap_or_default();
        let valid = (1..=64).contains(&key.len())
            && key
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
        if !valid {
            problems.push(FieldError::new(
                "key",
                "invalid",
                "must be 1 to 64 lower-case letters, digits or `_`",
            ));
        }
    }
    if creating && input.category.is_none() {
        problems.push(FieldError::new("category", "required", "is required"));
    }
    if input.category.as_deref().is_some_and(|value| !CATEGORIES.contains(&value)) {
        problems.push(FieldError::new(
            "category",
            "invalid",
            "must be `service`, `marketing`, `analytics`, `legal` or `kyc`",
        ));
    }
    if creating && input.legal_basis.is_none() {
        problems.push(FieldError::new("legal_basis", "required", "is required"));
    }
    if input.legal_basis.as_deref().is_some_and(|value| !LEGAL_BASES.contains(&value)) {
        problems.push(FieldError::new(
            "legal_basis",
            "invalid",
            "must be `consent`, `contract`, `legal_obligation`, `vital_interest` or `public_interest`",
        ));
    }
    if let Some(items) = &input.data_categories
        && (items.len() > 50 || items.iter().any(|item| item.trim().is_empty() || item.len() > 64))
    {
        problems.push(FieldError::new(
            "data_categories",
            "invalid",
            "must be at most 50 names of 1 to 64 characters",
        ));
    }
    if let Some(items) = &input.recipients
        && (items.len() > 50
            || items.iter().any(|item| {
                item.name.trim().is_empty()
                    || item.name.len() > 200
                    || item.country.as_deref().is_some_and(|country| {
                        autoxide_kernel::tenancy::parse_country(country, "country").is_err()
                    })
            }))
    {
        problems.push(FieldError::new(
            "recipients",
            "invalid",
            "must be at most 50 recipients with a name of 1 to 200 characters and a two-letter country code",
        ));
    }
    if let Some(description) = &input.description
        && (description.is_empty()
            || description.len() > 20
            || description.iter().any(|(language, text)| {
                language.is_empty()
                    || language.len() > 16
                    || text.trim().is_empty()
                    || text.len() > 2000
            }))
    {
        problems.push(FieldError::new(
            "description",
            "invalid",
            "must give a text of 1 to 2000 characters for at least one language",
        ));
    }
    if input
        .retention
        .as_ref()
        .is_some_and(|text| text.as_ref().is_some_and(|text| text.len() > 200))
    {
        problems.push(FieldError::new("retention", "length", "must be at most 200 characters"));
    }
}

/// A kind of personal data and how long it is kept.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceRetentionItem)]
pub(crate) struct RetentionItem {
    /// Module that keeps the data.
    pub module: String,
    /// `<module>.<kind>`, e.g. `forms.submissions`.
    pub name: String,
    pub description: String,
    /// `basic`, `sensitive`, `security` or `accounting`.
    pub category: DataCategory,
    /// `days`, `until_deleted` or `forever`.
    pub keep: String,
    /// Days, when `keep` is `days`.
    pub days: Option<u32>,
    /// Server setting that sets it, if any.
    pub setting: Option<String>,
    /// Longest the legal rule `privacy.retention_limits` allows for the category, in days.
    pub limit_days: Option<u32>,
    /// `ok`, `exceeds_limit` (kept longer than the limit) or `forever_not_allowed` (kept forever
    /// although the category should not be).
    pub status: String,
}

/// Where the tenant's data is kept.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceResidency)]
pub(crate) struct ResidencyView {
    pub tenant_id: Uuid,
    /// Country the tenant must keep its data in (ISO 3166-1 alpha-2, e.g. `VN`); `null` when free.
    pub data_residency: Option<String>,
    /// The placement (database) that keeps the data.
    pub placement: String,
    /// Country of that placement; `null` when unknown.
    pub placement_country: Option<String>,
    /// `true` when there is no requirement or the placement is in the required country.
    pub compliant: bool,
}

impl From<autoxide_kernel::tenancy::Residency> for ResidencyView {
    fn from(found: autoxide_kernel::tenancy::Residency) -> Self {
        let compliant = found.compliant();
        ResidencyView {
            tenant_id: found.tenant.as_uuid(),
            data_residency: found.required,
            placement: found.placement,
            placement_country: found.placement_country,
            compliant,
        }
    }
}

/// The register of processing activities.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceProcessingRegister)]
pub(crate) struct ProcessingRegister {
    /// Name of the tenant (the data controller).
    pub controller: String,
    /// The `privacy.contact` policy of the tenant, if set.
    #[schema(value_type = Option<Object>)]
    pub contact: Option<Value>,
    pub purposes: Vec<Purpose>,
    pub retention: Vec<RetentionItem>,
    pub residency: ResidencyView,
    pub generated_at: Timestamp,
    /// Reminder that a legal professional should review it.
    pub disclaimer: String,
}

fn keep_parts(keep: Keep) -> (&'static str, Option<u32>) {
    match keep {
        Keep::Days(days) => ("days", Some(days)),
        Keep::UntilDeleted => ("until_deleted", None),
        Keep::Forever => ("forever", None),
    }
}

/// Trạng thái của một mục so với giới hạn của quy tắc.
pub(crate) fn retention_status(
    category: DataCategory,
    keep: Keep,
    limits: &Value,
) -> (Option<u32>, &'static str) {
    let category = category.as_str();
    let limit = limits
        .get("max_days")
        .and_then(|max| max.get(category))
        .and_then(Value::as_u64)
        .and_then(|days| u32::try_from(days).ok());
    let forever_allowed = limits
        .get("forever_allowed")
        .and_then(Value::as_array)
        .is_some_and(|allowed| allowed.iter().any(|item| item.as_str() == Some(category)));
    let status = match keep {
        Keep::Forever if !forever_allowed => "forever_not_allowed",
        Keep::Days(days) if limit.is_some_and(|limit| days > limit) => "exceeds_limit",
        _ => "ok",
    };
    (limit, status)
}

/// Mục của sổ thời hạn lưu mà thời hạn thật là quy tắc `privacy.consent_proof_days`.
const CONSENT_PROOF_ITEMS: [&str; 1] = ["compliance.cookie_consents"];

impl ComplianceService {
    // ---- Mục đích ----------------------------------------------------------------------------

    /// Mục đích hiệu lực của khách: mặc định của quy tắc, phần khách thêm / ghi đè thắng theo `key`.
    pub(crate) async fn purposes(&self, tenant: &TenantContext) -> Result<Vec<Purpose>, Error> {
        let rule = self.rule_for(tenant, "privacy.default_purposes", Timestamp::now()).await?;
        let body: Value =
            rule.and_then(|row| serde_json::from_str(&row.body).ok()).unwrap_or_default();
        let defaults: Vec<DefaultPurpose> = body
            .get("purposes")
            .cloned()
            .and_then(|items| serde_json::from_value(items).ok())
            .unwrap_or_default();
        let own = purpose::Entity::find()
            .scoped(tenant)
            .order_by_asc(purpose::Column::Key)
            .all(tenant.database().read())
            .await?;
        let mut all: BTreeMap<String, Purpose> = defaults
            .into_iter()
            .map(|item| {
                (
                    item.key.clone(),
                    Purpose {
                        key: item.key,
                        category: item.category,
                        legal_basis: item.legal_basis,
                        data_categories: item.data_categories,
                        recipients: Vec::new(),
                        description: item.description,
                        retention: None,
                        active: true,
                        source: "default".to_owned(),
                    },
                )
            })
            .collect();
        for row in &own {
            all.insert(row.key.clone(), describe_purpose(row));
        }
        Ok(all.into_values().collect())
    }

    pub(crate) async fn list_purposes(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<PurposeWithCounts>, Error> {
        principal.require(&PRIVACY_READ)?;
        let purposes = self.purposes(tenant).await?;
        // Một câu đếm cho mọi mục đích: số người (khoá chủ thể khác nhau) theo mục đích và trạng thái.
        let rows: Vec<(String, String, i64)> = consent::Entity::find()
            .scoped(tenant)
            .select_only()
            .column(consent::Column::Purpose)
            .column(consent::Column::State)
            .column_as(Expr::col(consent::Column::SubjectKey).count_distinct(), "people")
            .group_by(consent::Column::Purpose)
            .group_by(consent::Column::State)
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        let mut counts: BTreeMap<String, (i64, i64)> = BTreeMap::new();
        for (purpose, state, people) in rows {
            let entry = counts.entry(purpose).or_default();
            match state.as_str() {
                "granted" => entry.0 = people,
                "withdrawn" => entry.1 = people,
                _ => {}
            }
        }
        let all = purposes
            .into_iter()
            .map(|purpose| {
                let (consented, withdrawn) = counts.get(&purpose.key).copied().unwrap_or_default();
                PurposeWithCounts { purpose, consented, withdrawn }
            })
            .collect();
        // Một trang (`page_of`): sổ mục đích là mục đích mặc định của quy tắc pháp lý gộp với mục đích khách thêm
        // hay sửa (dòng của khách thay mục đích mặc định cùng khoá) — gộp hai nguồn, không phải một keyset trong
        // database; sổ do người quản trị khai, vài chục mục. Theo khoá (duy nhất sau khi gộp).
        page_of(all, |item| item.purpose.key.clone(), page)
    }

    /// Mục đích công khai: một trang (`page_of`, cùng lý do với [`Self::list_purposes`]), theo khoá.
    pub(crate) async fn public_purposes_page(
        &self,
        tenant: &TenantContext,
        page: &PageRequest,
    ) -> Result<Page<Purpose>, Error> {
        page_of(self.public_purposes(tenant).await?, |item| item.key.clone(), page)
    }

    /// Mục đích công khai (đang dùng), kèm nhà cung cấp mã theo dõi đang bật là bên nhận dữ liệu.
    pub(crate) async fn public_purposes(
        &self,
        tenant: &TenantContext,
    ) -> Result<Vec<Purpose>, Error> {
        let mut purposes: Vec<Purpose> =
            self.purposes(tenant).await?.into_iter().filter(|item| item.active).collect();
        let recipients = self.tracking_recipients(tenant).await?;
        for item in &mut purposes {
            for (category, recipient) in &recipients {
                if item.key == *category && !item.recipients.contains(recipient) {
                    item.recipients.push(recipient.clone());
                }
            }
        }
        Ok(purposes)
    }

    async fn save_purpose(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        row: purpose::Model,
        insert: bool,
        cap: Option<u64>,
    ) -> Result<Purpose, Error> {
        let transaction = tenant.database().begin_write().await?;
        if let Some(max) = cap {
            // Đếm và chèn trong cùng một transaction ghi, sau khi khoá theo tên của khách: hai request tạo cùng
            // lúc xếp hàng (Postgres: advisory lock tới hết transaction; SQLite: `BEGIN IMMEDIATE` đã giữ cả
            // database), nên trần là chính xác — không vượt dù gọi đồng thời (D-20261126-purpose-cap-strict).
            autoxide_kernel::rows::lock_key(
                &transaction,
                &format!("compliance.purposes:{}", tenant.id().as_uuid()),
            )
            .await?;
            let own = purpose::Entity::find().scoped(tenant).count(&transaction).await?;
            if own >= max {
                return Err(Error::conflict(
                    "compliance.purpose_limit_reached",
                    "This tenant already has {max} processing purposes of its own.",
                )
                .with_param("max", max));
            }
        }
        let saved = if insert {
            purpose::ActiveModel::from(row).insert(&transaction).await.map_err(|error| {
                if crate::service::is_unique_violation(&error) {
                    purpose_exists("")
                } else {
                    error.into()
                }
            })?
        } else {
            purpose::ActiveModel::from(row).reset_all().update(&transaction).await?
        };
        let event = events::PurposeUpdated(json!({
            "key": saved.key, "legal_basis": saved.legal_basis, "active": saved.active,
        }));
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                Some(format!("compliance.purpose:{}", saved.key)),
                &event,
            )
            .await?;
        transaction.commit().await?;
        Ok(describe_purpose(&saved))
    }

    pub(crate) async fn create_purpose(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: PurposeInput,
    ) -> Result<Purpose, Error> {
        principal.require(&PRIVACY_MANAGE)?;
        let mut problems = Vec::new();
        check_purpose(&input, true, &mut problems);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let key = input.key.clone().unwrap_or_default();
        let exists = purpose::Entity::find()
            .scoped(tenant)
            .filter(purpose::Column::Key.eq(key.as_str()))
            .one(tenant.database().read())
            .await?;
        if exists.is_some() {
            return Err(purpose_exists(&key));
        }
        // Sổ mục đích đọc hết rồi chia trang trong bộ nhớ (gộp với mặc định của quy tắc), kể cả ở trang công khai:
        // trần ở chỗ ghi giữ nó nhỏ. Bản ghi đè mục đích mặc định cũng là dòng của khách nên được đếm; chúng có
        // trần tự nhiên là số mục đích mặc định, chỉ tạo mới bị chặn. `save_purpose` đếm trong transaction ghi.
        let row = purpose::Model {
            id: Uuid::now_v7(),
            tenant_id: tenant.id().as_uuid(),
            key: key.clone(),
            category: input.category.unwrap_or_default(),
            legal_basis: input.legal_basis.unwrap_or_default(),
            data_categories: json!(input.data_categories.unwrap_or_default()).to_string(),
            recipients: json!(input.recipients.unwrap_or_default()).to_string(),
            description: json!(input.description.unwrap_or_default()).to_string(),
            retention: input.retention.flatten(),
            active: input.active.unwrap_or(true),
            updated_by: Some(principal.user_id()),
            updated_at: now(),
        };
        self.save_purpose(tenant, principal, row, true, Some(self.purposes_tenant_max))
            .await
            .map_err(|error| {
                if error.code() == "compliance.purpose_exists" {
                    purpose_exists(&key)
                } else {
                    error
                }
            })
    }

    /// Sửa mục đích `key`: của khách thì sửa; mặc định thì tạo bản ghi đè từ bản mặc định.
    pub(crate) async fn update_purpose(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        key: &str,
        input: PurposeInput,
    ) -> Result<Purpose, Error> {
        principal.require(&PRIVACY_MANAGE)?;
        let mut problems = Vec::new();
        check_purpose(&PurposeInput { key: None, ..input.clone() }, false, &mut problems);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let own = purpose::Entity::find()
            .scoped(tenant)
            .filter(purpose::Column::Key.eq(key))
            .one(tenant.database().read())
            .await?;
        let (mut row, insert) = match own {
            Some(row) => (row, false),
            None => {
                let base = self
                    .purposes(tenant)
                    .await?
                    .into_iter()
                    .find(|item| item.key == key)
                    .ok_or_else(|| purpose_not_found(key))?;
                (
                    purpose::Model {
                        id: Uuid::now_v7(),
                        tenant_id: tenant.id().as_uuid(),
                        key: base.key,
                        category: base.category,
                        legal_basis: base.legal_basis,
                        data_categories: json!(base.data_categories).to_string(),
                        recipients: json!(base.recipients).to_string(),
                        description: json!(base.description).to_string(),
                        retention: base.retention,
                        active: base.active,
                        updated_by: None,
                        updated_at: now(),
                    },
                    true,
                )
            }
        };
        if let Some(category) = input.category {
            row.category = category;
        }
        if let Some(basis) = input.legal_basis {
            row.legal_basis = basis;
        }
        if let Some(items) = input.data_categories {
            row.data_categories = json!(items).to_string();
        }
        if let Some(items) = input.recipients {
            row.recipients = json!(items).to_string();
        }
        if let Some(description) = input.description {
            row.description = json!(description).to_string();
        }
        if let Some(retention) = input.retention {
            row.retention = retention;
        }
        if let Some(active) = input.active {
            row.active = active;
        }
        row.updated_by = Some(principal.user_id());
        row.updated_at = now();
        self.save_purpose(tenant, principal, row, insert, None).await
    }

    // ---- Thời hạn lưu --------------------------------------------------------------------------

    /// Sổ thời hạn lưu (mọi module) đối chiếu giới hạn của quy tắc `privacy.retention_limits` của vùng
    /// `jurisdiction`.
    pub(crate) async fn retention_items(
        &self,
        jurisdiction: &str,
    ) -> Result<Vec<RetentionItem>, Error> {
        let at = Timestamp::now();
        let limits: Value = self
            .effective_rule(jurisdiction, "privacy.retention_limits", at)
            .await?
            .and_then(|row| serde_json::from_str(&row.body).ok())
            .unwrap_or_default();
        let proof_days = self.consent_proof_days_in(jurisdiction).await?;
        Ok(self
            .retention
            .all()
            .iter()
            .map(|(module, policy)| {
                let keep = if CONSENT_PROOF_ITEMS.contains(&policy.name) {
                    Keep::Days(proof_days)
                } else {
                    policy.keep
                };
                let (keep_name, days) = keep_parts(keep);
                let (limit_days, status) = retention_status(policy.category, keep, &limits);
                RetentionItem {
                    module: (*module).to_owned(),
                    name: policy.name.to_owned(),
                    description: policy.description.to_owned(),
                    category: policy.category,
                    keep: keep_name.to_owned(),
                    days,
                    setting: policy.setting.map(str::to_owned),
                    limit_days,
                    status: status.to_owned(),
                }
            })
            .collect())
    }

    /// Số ngày giữ bằng chứng đồng ý (quy tắc `privacy.consent_proof_days`, mặc định 1825).
    pub(crate) async fn consent_proof_days_in(&self, jurisdiction: &str) -> Result<u32, Error> {
        Ok(self
            .effective_rule(jurisdiction, "privacy.consent_proof_days", Timestamp::now())
            .await?
            .and_then(|row| serde_json::from_str::<Value>(&row.body).ok())
            .and_then(|body| body.get("days").and_then(Value::as_u64))
            .and_then(|days| u32::try_from(days).ok())
            .unwrap_or(1825))
    }

    pub(crate) async fn retention(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<RetentionItem>, Error> {
        principal.require(&PRIVACY_READ)?;
        let jurisdiction = self.jurisdiction(tenant).await?;
        // Một trang (`page_of`): mỗi module khai dữ liệu cá nhân nó giữ trong code (sổ dựng lúc khởi động), không
        // nằm trong database. Theo module rồi tên.
        let items = self.retention_items(&jurisdiction).await?;
        page_of(items, |item| format!("{}\u{1f}{}", item.module, item.name), page)
    }

    // ---- Nơi lưu ---------------------------------------------------------------------------

    pub(crate) async fn residency(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<ResidencyView, Error> {
        principal.require(&PRIVACY_READ)?;
        Ok(self.databases.residency(tenant.id()).await?.into())
    }

    /// Đặt nơi lưu của khách `target` (mặc định khách của người gọi): nhân sự nền tảng có
    /// `compliance.privacy.manage`.
    pub(crate) async fn set_residency(
        &self,
        principal: &Principal,
        target: Option<Uuid>,
        country: Option<&str>,
    ) -> Result<ResidencyView, Error> {
        principal.require(&PRIVACY_MANAGE)?;
        require_platform(principal)?;
        let target = target.map_or_else(|| principal.tenant_id(), TenantId::from_uuid);
        let before = self.databases.residency(target).await?;
        let after = self.databases.set_residency(target, country).await?;
        if before.required != after.required {
            let tenant = self.databases.tenant(target).await?;
            let transaction = tenant.database().begin_write().await?;
            let event = events::ResidencyChanged(json!({
                "data_residency": after.required, "previous": before.required,
            }));
            self.outbox.record(&transaction, &tenant, Actor::from(principal), None, &event).await?;
            transaction.commit().await?;
        }
        Ok(after.into())
    }

    // ---- Sổ hoạt động xử lý ------------------------------------------------------------------

    pub(crate) async fn processing_register(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<ProcessingRegister, Error> {
        principal.require(&PRIVACY_READ)?;
        let jurisdiction = self.jurisdiction(tenant).await?;
        Ok(ProcessingRegister {
            controller: tenant.name().to_owned(),
            contact: self.policy(tenant, "privacy.contact").await?,
            purposes: self.public_purposes(tenant).await?,
            retention: self.retention_items(&jurisdiction).await?,
            residency: self.databases.residency(tenant.id()).await?.into(),
            generated_at: now(),
            disclaimer: DISCLAIMER.to_owned(),
        })
    }

    // ---- Người dùng bị xoá -------------------------------------------------------------------

    /// Xoá dữ liệu `compliance` giữ về người dùng `user` đã bị xoá: số điện thoại đã xác thực, mã một
    /// lần; gỡ người dùng khỏi bằng chứng đồng ý cookie. Bằng chứng đồng ý / chấp nhận văn bản và hồ sơ
    /// KYC giữ theo luật (không có email / số trong đó). Gọi lại vô hại.
    // Không dùng `self` (cả `export_user`): module đang tắt (chế độ "nghỉ", bước 70 P9) gọi được mà không
    // có service.
    pub(crate) async fn erase_user(tenant: &TenantContext, user: Uuid) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        account_verification::Entity::delete_many()
            .filter(account_verification::Column::UserId.eq(user))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        phone_challenge::Entity::delete_many()
            .filter(phone_challenge::Column::UserId.eq(user))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        crate::entity::cookie_consent::Entity::update_many()
            .col_expr(
                crate::entity::cookie_consent::Column::UserId,
                Expr::value(Option::<Uuid>::None),
            )
            .filter(crate::entity::cookie_consent::Column::UserId.eq(user))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        // Vụ việc người đó gửi: bỏ liên kết tài khoản và cách liên lạc; nội dung báo là chứng cứ của
        // vụ (D-363). Người đó thôi là bên bị phản ánh của vụ nào.
        crate::entity::case::Entity::update_many()
            .col_expr(
                crate::entity::case::Column::ReporterUserId,
                Expr::value(Option::<Uuid>::None),
            )
            .col_expr(
                crate::entity::case::Column::ReporterEmail,
                Expr::value(Option::<String>::None),
            )
            .col_expr(
                crate::entity::case::Column::ReporterPhone,
                Expr::value(Option::<String>::None),
            )
            .filter(crate::entity::case::Column::ReporterUserId.eq(user))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        crate::entity::case_respondent::Entity::delete_many()
            .filter(crate::entity::case_respondent::Column::UserId.eq(user))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Xoá giá trị email / số điện thoại của đối tượng `kind`/`key` trong sổ đồng ý (giữ khoá băm và
    /// bản ghi để chứng minh, không gửi lại) và số đã xác thực trùng số đó. Trả số dòng đã đổi.
    pub(crate) async fn erase_contact(
        &self,
        tenant: &TenantContext,
        subject: &crate::api::ConsentSubject,
    ) -> Result<u64, Error> {
        let (kind, key, _, _) = crate::consents::subject_columns(subject);
        let transaction = tenant.database().begin_write().await?;
        let mut changed = consent::Entity::update_many()
            .col_expr(consent::Column::SubjectValue, Expr::value(Option::<String>::None))
            .filter(consent::Column::SubjectKind.eq(kind))
            .filter(consent::Column::SubjectKey.eq(key))
            .scoped(tenant)
            .exec(&transaction)
            .await?
            .rows_affected;
        if let crate::api::ConsentSubject::Phone(phone) = subject {
            changed += account_verification::Entity::delete_many()
                .filter(
                    account_verification::Column::PhoneKey
                        .eq(autoxide_kernel::secret::sha256(phone)),
                )
                .scoped(tenant)
                .exec(&transaction)
                .await?
                .rows_affected;
        }
        transaction.commit().await?;
        Ok(changed)
    }

    /// Dữ liệu `compliance` giữ về người dùng `user_id` (bản xuất dữ liệu cá nhân).
    pub(crate) async fn export_user(tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
        let read = tenant.database().read();
        let verification = account_verification::Entity::find()
            .scoped(tenant)
            .filter(account_verification::Column::UserId.eq(user_id))
            .one(read)
            .await?;
        let consents = consent::Entity::find()
            .scoped(tenant)
            .filter(consent::Column::SubjectKind.eq("user"))
            .filter(consent::Column::SubjectKey.eq(user_id.to_string()))
            .order_by_asc(consent::Column::Id)
            .all(read)
            .await?;
        let kyc = kyc_profile::Entity::find()
            .scoped(tenant)
            .filter(kyc_profile::Column::SubjectKind.eq("user"))
            .filter(kyc_profile::Column::SubjectId.eq(user_id.to_string()))
            .one(read)
            .await?;
        let acceptances = acceptance::Entity::find()
            .scoped(tenant)
            .filter(acceptance::Column::SubjectKind.eq("user"))
            .filter(acceptance::Column::SubjectId.eq(user_id.to_string()))
            .order_by_asc(acceptance::Column::Id)
            .all(read)
            .await?;
        let requests = subject_request::Entity::find()
            .scoped(tenant)
            .filter(subject_request::Column::SubjectKind.eq("user"))
            .filter(subject_request::Column::Subject.eq(user_id.to_string()))
            .order_by_asc(subject_request::Column::Id)
            .all(read)
            .await?;
        let cookies = crate::entity::cookie_consent::Entity::find()
            .scoped(tenant)
            .filter(crate::entity::cookie_consent::Column::UserId.eq(user_id))
            .order_by_asc(crate::entity::cookie_consent::Column::Id)
            .all(read)
            .await?;
        Ok(json!({
            "phone_verification": verification.map(|row| json!({
                "phone": row.phone, "phone_verified_at": row.phone_verified_at,
            })),
            "consents": consents.iter().map(|row| json!({
                "purpose": row.purpose, "scope": row.scope, "channel": row.channel, "state": row.state,
                "granted_at": row.granted_at, "withdrawn_at": row.withdrawn_at,
            })).collect::<Vec<_>>(),
            "identity_verification": kyc.map(|row| json!({
                "status": row.status, "verified_name": row.verified_name, "document_kind": row.document_kind,
                "document_last4": row.document_last4, "verified_at": row.verified_at, "expires_at": row.expires_at,
            })),
            "document_acceptances": acceptances.iter().map(|row| json!({
                "document_id": row.document_id, "version": row.version, "channel": row.channel,
                "accepted_at": row.accepted_at,
            })).collect::<Vec<_>>(),
            "cookie_consents": cookies.iter().map(|row| json!({
                "consent_id": row.consent_id, "categories": serde_json::from_str::<Value>(&row.categories).unwrap_or_default(),
                "policy_version": row.policy_version, "created_at": row.created_at,
            })).collect::<Vec<_>>(),
            "subject_requests": requests.iter().map(|row| json!({
                "id": row.id, "kind": row.kind, "status": row.status, "received_at": row.received_at,
                "due_at": row.due_at, "closed_at": row.closed_at,
            })).collect::<Vec<_>>(),
            "kept_after_deletion": "Proof of consents, of accepted legal documents and the identity \
                verification result are kept for the time the law requires; the phone number is \
                deleted with the account.",
        }))
    }
}

/// Phần dữ liệu cá nhân của `compliance` trong bản xuất của người dùng.
pub(crate) struct CompliancePersonalData;

#[async_trait::async_trait]
impl PersonalData for CompliancePersonalData {
    async fn export(&self, tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
        ComplianceService::export_user(tenant, user_id).await
    }
}

/// Nghe `auth.user_deleted` — cả khi module đang tắt.
pub(crate) struct UserDeletedListener;

#[async_trait::async_trait]
impl EventHandler for UserDeletedListener {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        match deleted_user(event) {
            Some(user) => ComplianceService::erase_user(&event.tenant, user).await,
            None => Ok(()),
        }
    }
}

/// `autoxide doctor`: loại dữ liệu cá nhân giữ mãi hay quá giới hạn của quy tắc (vùng `vn`).
pub(crate) struct RetentionProbe(pub Arc<ComplianceService>);

#[async_trait::async_trait]
impl Probe for RetentionProbe {
    async fn probe(&self) -> Finding {
        let items = match self.0.retention_items("vn").await {
            Ok(items) => items,
            Err(error) => {
                return Finding::failed(format!("cannot read the retention rules: {error}"));
            }
        };
        let wrong: Vec<String> = items
            .iter()
            .filter(|item| item.status != "ok")
            .map(|item| format!("{} ({})", item.name, item.status))
            .collect();
        if wrong.is_empty() {
            Finding::ok(format!(
                "{} kinds of personal data have a retention within the limits.",
                items.len()
            ))
        } else {
            Finding::warning(format!(
                "Some personal data is kept longer than the legal rule `privacy.retention_limits` allows: {}.",
                wrong.join(", ")
            ))
            .with_hint("Lower the retention setting of each item (GET /api/v1/compliance/retention shows them).")
        }
    }
}

/// `autoxide doctor`: khách buộc lưu dữ liệu ở một nước mà placement không đáp ứng.
pub(crate) struct ResidencyProbe(pub Arc<ComplianceService>);

#[async_trait::async_trait]
impl Probe for ResidencyProbe {
    async fn probe(&self) -> Finding {
        let all = match self.0.databases.residencies().await {
            Ok(all) => all,
            Err(error) => return Finding::failed(format!("cannot read the tenants: {error}")),
        };
        let wrong: Vec<String> = all
            .iter()
            .filter(|found| !found.compliant())
            .map(|found| {
                format!(
                    "tenant {} must keep its data in {} but placement `{}` is in {}",
                    found.tenant,
                    found.required.as_deref().unwrap_or("?"),
                    found.placement,
                    found.placement_country.as_deref().unwrap_or("an unknown country")
                )
            })
            .collect();
        if wrong.is_empty() {
            Finding::ok(format!(
                "{} tenant(s) with a data residency requirement, all met.",
                all.len()
            ))
        } else {
            Finding::warning(wrong.join("; ")).with_hint(
                "Record the placement's country (`autoxide placement country <name> VN`) or move the \
                 tenant to a placement in that country (`autoxide tenant move`).",
            )
        }
    }
}

/// `autoxide doctor`: sự cố quá hạn báo cáo, yêu cầu của chủ thể quá hạn trả lời.
pub(crate) struct DeadlinesProbe(pub Arc<ComplianceService>);

#[async_trait::async_trait]
impl Probe for DeadlinesProbe {
    async fn probe(&self) -> Finding {
        let databases = match self.0.databases.databases().await {
            Ok(databases) => databases,
            Err(error) => return Finding::failed(format!("cannot list the databases: {error}")),
        };
        let at = Timestamp::now();
        let (mut incidents, mut requests) = (0usize, 0usize);
        for (_, name, database) in databases {
            // tenant-scope: việc nền / kiểm tra quét mọi khách của database, chủ ý không lọc khách (rà soát bảo mật 2026-11)
            let late = crate::entity::incident::Entity::find()
                .filter(crate::entity::incident::Column::Status.is_in(["open", "contained"]))
                .filter(crate::entity::incident::Column::AuthorityNotifiedAt.is_null())
                .filter(crate::entity::incident::Column::ReportDueAt.lt(at))
                .all(database.read())
                .await;
            // tenant-scope: việc nền / kiểm tra quét mọi khách của database, chủ ý không lọc khách (rà soát bảo mật 2026-11)
            let overdue = subject_request::Entity::find()
                .filter(subject_request::Column::Status.is_in(["received", "in_progress"]))
                .filter(subject_request::Column::DueAt.lt(at))
                .all(database.read())
                .await;
            match (late, overdue) {
                (Ok(late), Ok(overdue)) => {
                    incidents += late.len();
                    requests += overdue.len();
                }
                (Err(error), _) | (_, Err(error)) => {
                    return Finding::failed(format!("cannot read database `{name}`: {error}"));
                }
            }
        }
        if incidents == 0 && requests == 0 {
            return Finding::ok(
                "No personal data breach or data subject request is past its deadline.",
            );
        }
        Finding::warning(format!(
            "{incidents} personal data breach(es) not reported in time and {requests} data subject \
             request(s) past their due date."
        ))
        .with_hint(
            "See GET /api/v1/compliance/incidents?overdue=true and \
             GET /api/v1/compliance/subject-requests?overdue=true of each tenant; docs/COMPLIANCE.md has the 72-hour runbook.",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gioi_han_thoi_han_luu_theo_loai() {
        let limits = json!({"forever_allowed": ["accounting"], "max_days": {"basic": 365}});
        assert_eq!(
            retention_status(DataCategory::Basic, Keep::Days(30), &limits),
            (Some(365), "ok")
        );
        assert_eq!(
            retention_status(DataCategory::Basic, Keep::Days(400), &limits).1,
            "exceeds_limit"
        );
        assert_eq!(
            retention_status(DataCategory::Basic, Keep::Forever, &limits).1,
            "forever_not_allowed"
        );
        assert_eq!(retention_status(DataCategory::Accounting, Keep::Forever, &limits).1, "ok");
        assert_eq!(
            retention_status(DataCategory::Sensitive, Keep::UntilDeleted, &limits),
            (None, "ok")
        );
    }

    #[test]
    fn muc_dich_sai_bao_moi_loi_mot_lan() {
        let mut problems = Vec::new();
        let input = PurposeInput {
            key: Some("Bad Key".into()),
            category: Some("ads".into()),
            legal_basis: Some("legitimate".into()),
            ..PurposeInput::default()
        };
        check_purpose(&input, true, &mut problems);
        let fields: Vec<&str> = problems.iter().map(|problem| problem.field.as_str()).collect();
        assert_eq!(fields, vec!["key", "category", "legal_basis"]);
    }
}
