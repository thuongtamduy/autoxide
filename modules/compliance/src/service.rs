//! Service của module `compliance`: lỗi chung, quy tắc pháp lý (dữ liệu có ngày hiệu lực, mặt điều
//! khiển), chính sách của khách. KYC ở [`crate::kyc`], văn bản ở [`crate::documents`], đồng ý ở
//! [`crate::consents`].

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::identity::IdentityVerifier;
use autoxide_kernel::job::JobQueue;
use autoxide_kernel::tenancy::{DatabaseRegistry, ScopeExt, TenantContext, TenantId};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::Rule;
use crate::entity::{policy, rule};
use crate::events;
use crate::permissions::{POLICIES_MANAGE, RULES_MANAGE};

pub(crate) fn now() -> Timestamp {
    Timestamp::now()
}

/// Gói quy tắc `vn` đi kèm máy chủ.
pub(crate) const BUNDLED_VN: &str = include_str!("../rules/vn.json");

pub(crate) struct ComplianceService {
    pub databases: DatabaseRegistry,
    pub outbox: Outbox,
    pub jobs: JobQueue,
    /// Nhà cung cấp xác thực danh tính; `None` thì chỉ duyệt tay.
    pub verifier: Option<Arc<dyn IdentityVerifier>>,
    /// `AUTOXIDE_COMPLIANCE_KYC_CALLBACK_RETENTION_DAYS`.
    pub callback_retention_days: i64,
    /// Nhà cung cấp SMS (gửi mã một lần); `None` thì xác thực số trả `sms_disabled`.
    pub sms: Option<Arc<dyn autoxide_kernel::sms::SmsSender>>,
    /// Cấu hình theo khách (mẫu câu SMS mang mã, D-20261125-sms-templates).
    pub settings: Arc<dyn autoxide_kernel::tenancy::TenantSettings>,
    /// Khoá gốc, để dẫn pepper của mã một lần.
    pub secret: autoxide_kernel::secret::SecretKey,
    /// Giới hạn của mã một lần (`AUTOXIDE_COMPLIANCE_OTP_*`).
    pub otp: crate::config::Otp,
    /// Kiểm địa chỉ của hồ sơ pháp lý (module `address`); `None` thì không nhận địa chỉ.
    pub units: Option<Arc<dyn autoxide_address::api::AdministrativeUnits>>,
    /// Sổ đối tượng (D-123): kiểm trang bắt buộc gắn với website (`content.entry`).
    pub resources: autoxide_kernel::resource::ResourceDirectory,
    /// Bộ dựng mẫu của kernel (D-351): bản nháp đề án, quy chế của bộ hồ sơ.
    pub templates: autoxide_kernel::template::TemplateEngine,
    /// "Fact" của danh sách kiểm do module khác đăng ký.
    pub facts: Arc<crate::checklist::FactRegistry>,
    /// Gửi mail nhắc hạn sự cố (module `mail`); `None` thì chỉ phát event.
    pub mailer: Option<Arc<dyn autoxide_kernel::mail::Mailer>>,
    /// Bản dịch (mẫu mail, bản nháp thông báo sự cố, câu chữ của banner cookie).
    pub translations: autoxide_kernel::i18n::Translations,
    /// Sổ dữ liệu cá nhân của mọi module (yêu cầu `access` của chủ thể).
    pub personal_data: autoxide_kernel::privacy::PersonalDataExport,
    /// Sổ thời hạn lưu của mọi module.
    pub retention: autoxide_kernel::privacy::RetentionCatalog,
    /// `AUTOXIDE_COMPLIANCE_COOKIE_CONSENTS_PER_IP_PER_HOUR`.
    pub cookie_consents_per_hour: u32,
    /// Chống spam cho hai kênh báo công khai (D-119, D-363).
    pub guard: autoxide_kernel::antispam::SpamGuard,
    /// `AUTOXIDE_COMPLIANCE_PUBLIC_REPORTS_PER_IP_PER_HOUR`.
    pub public_reports_per_hour: u32,
    /// `AUTOXIDE_COMPLIANCE_CASE_TRACKING_URL`.
    pub case_tracking_url: Option<String>,
    /// Link trong thư theo khách (`kernel.public_url`, `compliance.link.case_tracking`).
    pub links: autoxide_kernel::tenancy::SiteLinks,
}

/// Duyệt hay quyết định cần phiên đăng nhập của người, không nhận API key.
pub(crate) fn require_session(principal: &Principal) -> Result<(), Error> {
    if principal.api_key_id().is_some() {
        return Err(Error::forbidden(
            "compliance.session_required",
            "This needs a signed-in person; API keys cannot do it.",
        ));
    }
    Ok(())
}

/// Lỗi vi phạm unique của database (hai bên báo khác nhau; SeaORM gom về một loại).
pub(crate) fn is_unique_violation(error: &sea_orm::DbErr) -> bool {
    matches!(error.sql_err(), Some(sea_orm::SqlErr::UniqueConstraintViolation(_)))
}

pub(crate) fn rules_invalid(detail: impl ToString) -> Error {
    Error::new(
        ErrorKind::Validation,
        "compliance.rules_invalid",
        "The rules are not valid: {detail}.",
    )
    .with_param("detail", detail)
}

/// Gói quy tắc.
#[derive(Debug, Deserialize)]
struct Pack {
    jurisdiction: String,
    rules: Vec<PackRule>,
}

#[derive(Debug, Deserialize)]
struct PackRule {
    key: String,
    version: i64,
    effective_from: Timestamp,
    effective_to: Option<Timestamp>,
    body: Value,
    legal_basis: String,
    #[serde(default = "yes")]
    needs_legal_review: bool,
}

const fn yes() -> bool {
    true
}

/// Các bản quy tắc `key` trong gói đi kèm của vùng `jurisdiction` (chưa nhập vào database).
fn bundled_rules(jurisdiction: &str, key: &str) -> Vec<rule::Model> {
    let Ok(pack) = serde_json::from_str::<Pack>(BUNDLED_VN) else {
        return Vec::new();
    };
    if pack.jurisdiction != jurisdiction {
        return Vec::new();
    }
    pack.rules
        .into_iter()
        .filter(|item| item.key == key)
        .map(|item| rule::Model {
            id: Uuid::nil(),
            jurisdiction: pack.jurisdiction.clone(),
            key: item.key,
            version: item.version,
            effective_from: item.effective_from,
            effective_to: item.effective_to,
            body: item.body.to_string(),
            legal_basis: item.legal_basis,
            needs_legal_review: item.needs_legal_review,
            source: "bundled".to_owned(),
            imported_at: item.effective_from,
        })
        .collect()
}

pub(crate) fn describe_rule(row: &rule::Model) -> Rule {
    Rule {
        jurisdiction: row.jurisdiction.clone(),
        key: row.key.clone(),
        version: row.version,
        effective_from: row.effective_from,
        effective_to: row.effective_to,
        body: serde_json::from_str(&row.body).unwrap_or_default(),
        legal_basis: row.legal_basis.clone(),
        needs_legal_review: row.needs_legal_review,
    }
}

impl ComplianceService {
    // ---- Quy tắc ---------------------------------------------------------------------------

    /// Nhập gói quy tắc `text` (JSON). Bản đã có (vùng, khoá, phiên bản) giữ nguyên — quy tắc chỉ thêm,
    /// không sửa. Trả số bản mới.
    pub(crate) async fn import_rules(
        &self,
        actor: Actor,
        text: &str,
        source: &str,
    ) -> Result<u64, Error> {
        let pack: Pack = serde_json::from_str(text).map_err(rules_invalid)?;
        let mut problems = Vec::new();
        if !(1..=8).contains(&pack.jurisdiction.len()) {
            problems.push(FieldError::new("jurisdiction", "length", "must be 1 to 8 characters"));
        }
        for (index, item) in pack.rules.iter().enumerate() {
            if item.key.is_empty() || item.key.len() > 64 {
                problems.push(FieldError::new(
                    format!("rules[{index}].key"),
                    "length",
                    "must be 1 to 64 characters",
                ));
            }
            if item.version < 1 {
                problems.push(FieldError::new(
                    format!("rules[{index}].version"),
                    "range",
                    "must be at least 1",
                ));
            }
            if item.effective_to.is_some_and(|to| to <= item.effective_from) {
                problems.push(FieldError::new(
                    format!("rules[{index}].effective_to"),
                    "range",
                    "must be after `effective_from`",
                ));
            }
            if item.legal_basis.trim().is_empty() {
                problems.push(FieldError::new(
                    format!("rules[{index}].legal_basis"),
                    "required",
                    "is required",
                ));
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let control = self.databases.control();
        let transaction = control.begin_write().await?;
        let at = now();
        let mut inserted = 0;
        for item in &pack.rules {
            inserted += rule::Entity::insert(rule::ActiveModel {
                id: Set(Uuid::now_v7()),
                jurisdiction: Set(pack.jurisdiction.clone()),
                key: Set(item.key.clone()),
                version: Set(item.version),
                effective_from: Set(item.effective_from),
                effective_to: Set(item.effective_to),
                body: Set(item.body.to_string()),
                legal_basis: Set(item.legal_basis.clone()),
                needs_legal_review: Set(item.needs_legal_review),
                source: Set(source.to_owned()),
                imported_at: Set(at),
            })
            .on_conflict(
                OnConflict::columns([
                    rule::Column::Jurisdiction,
                    rule::Column::Key,
                    rule::Column::Version,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(&transaction)
            .await?;
        }
        if inserted > 0 {
            let platform = self.databases.default_tenant();
            let event = events::RulesImported(
                json!({"jurisdiction": pack.jurisdiction, "rules": inserted}),
            );
            self.outbox.record(&transaction, &platform, actor, None, &event).await?;
        }
        transaction.commit().await?;
        Ok(inserted)
    }

    /// Nhập quy tắc qua REST: nhân sự nền tảng có `compliance.rules.manage`.
    pub(crate) async fn import_rules_manual(
        &self,
        principal: &Principal,
        text: &str,
    ) -> Result<u64, Error> {
        autoxide_kernel::tenancy::require_platform(principal)?;
        principal.require(&RULES_MANAGE)?;
        self.import_rules(Actor::from(principal), text, "import").await
    }

    /// Nhập gói đi kèm (job hằng giờ; bản đã có giữ nguyên).
    pub(crate) async fn import_bundled(&self) -> Result<u64, Error> {
        self.import_rules(Actor::system(), BUNDLED_VN, "bundled").await
    }

    /// Bản quy tắc `key` hiệu lực lúc `at` ở vùng `jurisdiction`: bản `effective_from` mới nhất ≤ `at`
    /// (và chưa hết hạn); cùng ngày thì phiên bản lớn nhất.
    pub(crate) async fn effective_rule(
        &self,
        jurisdiction: &str,
        key: &str,
        at: Timestamp,
    ) -> Result<Option<rule::Model>, Error> {
        let rows = rule::Entity::find()
            .filter(rule::Column::Jurisdiction.eq(jurisdiction))
            .filter(rule::Column::Key.eq(key))
            .filter(rule::Column::EffectiveFrom.lte(at))
            .order_by_desc(rule::Column::EffectiveFrom)
            .order_by_desc(rule::Column::Version)
            .all(self.databases.control().read())
            .await?;
        // Gói đi kèm có hiệu lực ngay khi máy chủ chạy bản mới, không chờ job nhập hằng giờ: bản nào
        // database chưa có thì lấy từ gói.
        let mut rows = rows;
        for bundled in bundled_rules(jurisdiction, key) {
            if bundled.effective_from <= at
                && !rows.iter().any(|row| row.version == bundled.version)
            {
                rows.push(bundled);
            }
        }
        rows.sort_by_key(|row| std::cmp::Reverse((row.effective_from, row.version)));
        Ok(rows.into_iter().find(|row| row.effective_to.is_none_or(|to| at < to)))
    }

    /// Vùng pháp lý của khách (chính sách `jurisdiction`, mặc định `vn`).
    pub(crate) async fn jurisdiction(&self, tenant: &TenantContext) -> Result<String, Error> {
        Ok(self
            .policy(tenant, "jurisdiction")
            .await?
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| "vn".to_owned()))
    }

    pub(crate) async fn rule_for(
        &self,
        tenant: &TenantContext,
        key: &str,
        at: Timestamp,
    ) -> Result<Option<rule::Model>, Error> {
        let jurisdiction = self.jurisdiction(tenant).await?;
        self.effective_rule(&jurisdiction, key, at).await
    }

    /// Quy tắc theo bộ lọc. Có `as_of`: mỗi khoá (vùng pháp lý, tên) chỉ còn bản có hiệu lực lúc đó,
    /// chọn như [`Self::effective_rule`] (hiệu lực muộn nhất, rồi phiên bản lớn nhất); không có thì mọi
    /// phiên bản (lịch sử), mới trước.
    pub(crate) async fn list_rules(
        &self,
        principal: &Principal,
        filter: RuleFilter,
    ) -> Result<Vec<rule::Model>, Error> {
        principal.require(&crate::permissions::RULES_READ)?;
        let mut select = rule::Entity::find();
        if let Some(jurisdiction) = filter.jurisdiction {
            select = select.filter(rule::Column::Jurisdiction.eq(jurisdiction));
        }
        if let Some(key) = filter.key {
            select = select.filter(rule::Column::Key.eq(key));
        }
        if let Some(at) = filter.as_of {
            select = select.filter(rule::Column::EffectiveFrom.lte(at));
        }
        let rows = select
            .order_by_asc(rule::Column::Jurisdiction)
            .order_by_asc(rule::Column::Key)
            .order_by_desc(rule::Column::EffectiveFrom)
            .order_by_desc(rule::Column::Version)
            .all(self.databases.control().read())
            .await?;
        let Some(at) = filter.as_of else {
            return Ok(rows);
        };
        // Đã sắp theo khoá rồi theo hiệu lực / phiên bản giảm dần: bản đầu tiên còn hiệu lực của
        // mỗi khoá là bản cần.
        let mut effective: Vec<rule::Model> = Vec::new();
        for row in rows {
            let same_key = effective
                .last()
                .is_some_and(|last| last.jurisdiction == row.jurisdiction && last.key == row.key);
            if !same_key && row.effective_to.is_none_or(|to| at < to) {
                effective.push(row);
            }
        }
        Ok(effective)
    }

    pub(crate) async fn tenant(&self, id: TenantId) -> Result<TenantContext, Error> {
        self.databases.tenant(id).await
    }

    // ---- Chính sách --------------------------------------------------------------------------

    pub(crate) async fn policy(
        &self,
        tenant: &TenantContext,
        key: &str,
    ) -> Result<Option<Value>, Error> {
        let found = policy::Entity::find_by_id((tenant.id().as_uuid(), key.to_owned()))
            .one(tenant.database().read())
            .await?;
        Ok(found.and_then(|row| serde_json::from_str(&row.value).ok()))
    }

    pub(crate) async fn list_policies(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<Vec<policy::Model>, Error> {
        principal.require(&POLICIES_MANAGE)?;
        Ok(policy::Entity::find().scoped(tenant).all(tenant.database().read()).await?)
    }

    pub(crate) async fn set_policy(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        key: &str,
        value: Value,
    ) -> Result<policy::Model, Error> {
        principal.require(&POLICIES_MANAGE)?;
        let valid = match key {
            "jurisdiction" => value.as_str().is_some_and(|text| (1..=8).contains(&text.len())),
            "posting.comments" | "posting.reviews" | "posting.livestream" => {
                value.as_str().is_some_and(|text| ["none", "phone", "identity"].contains(&text))
            }
            "posting.max_accounts_per_phone" => {
                value.as_i64().is_some_and(|count| (1..=100).contains(&count))
            }
            "incident.alert_emails" => value.as_array().is_some_and(|items| {
                items.len() <= 20
                    && items
                        .iter()
                        .all(|item| item.as_str().is_some_and(autoxide_kernel::email::is_valid))
            }),
            "complaints.contact" | "privacy.contact" => value.is_object(),
            "cases.auto_restrict" => value.is_boolean(),
            _ => {
                return Err(Error::not_found(
                    "compliance.policy_unknown",
                    "There is no compliance policy `{key}`.",
                )
                .with_param("key", key));
            }
        };
        if !valid {
            return Err(Error::validation(vec![FieldError::new(
                "value",
                "invalid",
                "is not a valid value for this policy",
            )]));
        }
        let transaction = tenant.database().begin_write().await?;
        let row = policy::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            key: Set(key.to_owned()),
            value: Set(value.to_string()),
            updated_by: Set(Some(principal.user_id())),
            updated_at: Set(now()),
        };
        policy::Entity::insert(row)
            .on_conflict(
                OnConflict::columns([policy::Column::TenantId, policy::Column::Key])
                    .update_columns([
                        policy::Column::Value,
                        policy::Column::UpdatedBy,
                        policy::Column::UpdatedAt,
                    ])
                    .to_owned(),
            )
            .exec_without_returning(&transaction)
            .await?;
        let event = events::PolicyUpdated(json!({"key": key}));
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                Some(format!("compliance.policy:{key}")),
                &event,
            )
            .await?;
        let saved = policy::Entity::find_by_id((tenant.id().as_uuid(), key.to_owned()))
            .one(&transaction)
            .await?
            .ok_or_else(|| Error::internal("a policy vanished"))?;
        transaction.commit().await?;
        Ok(saved)
    }
}

/// Bộ lọc danh sách quy tắc.
#[derive(Debug, Clone, Default)]
pub(crate) struct RuleFilter {
    pub jurisdiction: Option<String>,
    pub key: Option<String>,
    pub as_of: Option<Timestamp>,
}
