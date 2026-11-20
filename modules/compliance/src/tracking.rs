//! Mã theo dõi quảng cáo và đồng ý cookie (D-235, D-361; Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15, NĐ
//! 356/2025/NĐ-CP, Luật Bảo vệ quyền lợi người tiêu dùng 19/2023/QH15 — cần đối chiếu văn bản gốc / người
//! có chuyên môn pháp lý):
//!
//! - Nhà cung cấp cho sẵn ([`PROVIDERS`]: GA4, GTM, Google Ads, Meta, TikTok, Zalo); khách chỉ điền mã
//!   ID, không dán script. Mã sai dạng → `compliance.tracking_id_invalid`.
//! - Banner theo website (nút): nhóm `necessary` (luôn bật, không mã nào), `preferences`, `analytics`,
//!   `marketing`. Đổi nhóm hay nhà cung cấp đang bật → `policy_version` mới (người xem được hỏi lại).
//! - **Không trả mã của nhóm chưa đồng ý**: `GET /public/consent-config?granted=…` chỉ trả mã của nhóm
//!   người xem đã chọn — mặc định không có gì.
//! - Bằng chứng đồng ý ẩn danh: `consent_id` do giao diện tạo, nhóm, phiên bản, IP băm; giữ theo quy tắc
//!   `privacy.consent_proof_days`. Người đã đăng nhập thì ghi thêm vào sổ đồng ý chung.

use std::collections::BTreeSet;
use std::net::IpAddr;
use std::sync::Arc;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, TenantId};
use autoxide_kernel::throttle::{Outcome, Throttle};
use chrono::{DateTime, Duration, Utc};
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, Set,
};
use serde_json::json;
use uuid::Uuid;

use crate::api::{ConsentEvidence, ConsentKey, ConsentSubject};
use crate::entity::{cookie_consent, cookie_setting, tracking_tag};
use crate::events;
use crate::permissions::TRACKING_MANAGE;
use crate::privacy::Recipient;
use crate::service::{ComplianceService, now};

/// Job dọn bằng chứng đồng ý cookie quá hạn giữ.
pub(crate) const CONSENT_PROOF_PURGE: &str = "compliance.consent_proof_purge";

/// Nhóm đồng ý tuỳ chọn (ngoài `necessary`).
pub(crate) const OPTIONAL: [&str; 3] = ["preferences", "analytics", "marketing"];
const POSITIONS: [&str; 3] = ["bottom", "top", "center"];

/// Dạng mã ID của một nhà cung cấp.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Format {
    /// Tiền tố + chữ hoa / số, độ dài phần sau tiền tố trong khoảng.
    Prefixed { prefix: &'static str, min: usize, max: usize, digits_only: bool },
    /// Chỉ chữ số.
    Digits { min: usize, max: usize },
    /// Chữ hoa và số.
    Alphanumeric { min: usize, max: usize },
}

impl Format {
    fn accepts(self, id: &str) -> bool {
        let (rest, min, max, digits_only) = match self {
            Format::Prefixed { prefix, min, max, digits_only } => match id.strip_prefix(prefix) {
                Some(rest) => (rest, min, max, digits_only),
                None => return false,
            },
            Format::Digits { min, max } => (id, min, max, true),
            Format::Alphanumeric { min, max } => (id, min, max, false),
        };
        (min..=max).contains(&rest.len())
            && rest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (!digits_only && byte.is_ascii_uppercase()))
    }
}

/// Một nhà cung cấp mã theo dõi cho sẵn. Thêm nhà cung cấp = thêm một mục + test.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Provider {
    pub key: &'static str,
    pub name: &'static str,
    /// `analytics` hay `marketing`.
    pub category: &'static str,
    pub format: Format,
    /// Dạng mong đợi, cho thông báo lỗi.
    pub example: &'static str,
    /// Host cần mở trong Content-Security-Policy khi mã được tải.
    pub hosts: &'static [&'static str],
    /// Nước nhận dữ liệu (ISO 3166-1 alpha-2).
    pub country: &'static str,
    /// Script tải, `{id}` thay bằng mã; `None` khi giao diện tự dựng theo tài liệu của nhà cung cấp.
    pub script: Option<&'static str>,
}

pub(crate) const PROVIDERS: [Provider; 6] = [
    Provider {
        key: "ga4",
        name: "Google Analytics 4",
        category: "analytics",
        format: Format::Prefixed { prefix: "G-", min: 4, max: 12, digits_only: false },
        example: "G-XXXXXXXXXX",
        hosts: &["www.googletagmanager.com", "www.google-analytics.com"],
        country: "US",
        script: Some("https://www.googletagmanager.com/gtag/js?id={id}"),
    },
    Provider {
        key: "gtm",
        name: "Google Tag Manager",
        // Một container tải được mọi thứ: xếp vào nhóm chặt nhất.
        category: "marketing",
        format: Format::Prefixed { prefix: "GTM-", min: 4, max: 10, digits_only: false },
        example: "GTM-XXXXXXX",
        hosts: &["www.googletagmanager.com"],
        country: "US",
        script: Some("https://www.googletagmanager.com/gtm.js?id={id}"),
    },
    Provider {
        key: "google_ads",
        name: "Google Ads",
        category: "marketing",
        format: Format::Prefixed { prefix: "AW-", min: 6, max: 12, digits_only: true },
        example: "AW-123456789",
        hosts: &[
            "www.googletagmanager.com",
            "googleads.g.doubleclick.net",
            "www.googleadservices.com",
        ],
        country: "US",
        script: Some("https://www.googletagmanager.com/gtag/js?id={id}"),
    },
    Provider {
        key: "meta",
        name: "Meta Pixel",
        category: "marketing",
        format: Format::Digits { min: 10, max: 20 },
        example: "1234567890123456",
        hosts: &["connect.facebook.net", "www.facebook.com"],
        country: "US",
        script: Some("https://connect.facebook.net/en_US/fbevents.js"),
    },
    Provider {
        key: "tiktok",
        name: "TikTok Pixel",
        category: "marketing",
        format: Format::Alphanumeric { min: 15, max: 25 },
        example: "C1A2B3C4D5E6F7G8H9I0",
        hosts: &["analytics.tiktok.com"],
        country: "SG",
        script: Some("https://analytics.tiktok.com/i18n/pixel/events.js?sdkid={id}"),
    },
    Provider {
        key: "zalo",
        name: "Zalo Ads Pixel",
        category: "marketing",
        format: Format::Digits { min: 10, max: 25 },
        example: "1234567890123456789",
        hosts: &["sp.zalo.me"],
        country: "VN",
        script: None,
    },
];

pub(crate) fn provider(key: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|item| item.key == key)
}

fn provider_unknown(key: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "compliance.tracking_provider_unknown",
        "There is no built-in tracking provider `{provider}`; use `ga4`, `gtm`, `google_ads`, `meta`, \
         `tiktok` or `zalo`.",
    )
    .with_param("provider", key)
}

fn tracking_id_invalid(expected: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "compliance.tracking_id_invalid",
        "The tracking ID does not have the expected form `{expected}`.",
    )
    .with_param("expected", expected)
}

pub(crate) fn tag_not_found() -> Error {
    Error::not_found("compliance.tracking_tag_not_found", "There is no such tracking tag.")
}

fn tag_exists() -> Error {
    Error::conflict(
        "compliance.tracking_tag_exists",
        "This website already has this tracking ID for this provider.",
    )
}

fn policy_changed(current: i32) -> Error {
    Error::conflict(
        "compliance.cookie_policy_changed",
        "The cookie policy changed (now version {version}); show the banner again and ask anew.",
    )
    .with_param("version", current)
}

fn consents_throttled(retry_after: u64) -> Error {
    Error::too_many_requests(
        "compliance.cookie_consents_throttled",
        "Too many consent records from this address. Wait {seconds} seconds, then try again.",
    )
    .with_param("seconds", retry_after)
    .with_retry_after(retry_after)
}

/// Kiểm nhà cung cấp và mã; trả mã đã chuẩn hoá (bỏ khoảng trắng, chữ hoa).
pub(crate) fn check_tag(provider_key: &str, tracking_id: &str) -> Result<String, Error> {
    let found = provider(provider_key).ok_or_else(|| provider_unknown(provider_key))?;
    let id = tracking_id.trim().to_ascii_uppercase();
    if found.format.accepts(&id) { Ok(id) } else { Err(tracking_id_invalid(found.example)) }
}

/// Banner của một website.
#[derive(Debug, Clone)]
pub(crate) struct Banner {
    pub org_id: Uuid,
    pub enabled: bool,
    /// Nhóm tuỳ chọn khách bật.
    pub configured: Vec<String>,
    /// Nhóm hiệu lực: đã bật cộng nhóm của mã đang chạy.
    pub categories: Vec<String>,
    pub policy_version: i32,
    pub position: String,
    pub updated_at: DateTime<Utc>,
    /// Mã đang bật.
    pub tags: Vec<tracking_tag::Model>,
}

/// Thay đổi banner.
#[derive(Debug, Clone, Default)]
pub(crate) struct BannerChange {
    pub enabled: Option<bool>,
    pub categories: Option<Vec<String>>,
    pub position: Option<String>,
}

fn effective(configured: &[String], tags: &[tracking_tag::Model]) -> Vec<String> {
    let mut all: BTreeSet<&str> = configured.iter().map(String::as_str).collect();
    for tag in tags {
        if let Some(found) = provider(&tag.provider) {
            all.insert(found.category);
        }
    }
    // Thứ tự cố định để giao diện hiện giống nhau.
    OPTIONAL.iter().filter(|key| all.contains(**key)).map(|key| (*key).to_owned()).collect()
}

fn signature(categories: &[String], tags: &[tracking_tag::Model]) -> String {
    let providers: BTreeSet<&str> = tags.iter().map(|tag| tag.provider.as_str()).collect();
    autoxide_kernel::secret::sha256(&json!([categories, providers]).to_string())
}

/// Thống kê đồng ý của một website.
#[derive(Debug, Clone)]
pub(crate) struct Stats {
    pub total: u64,
    pub by_category: Vec<(String, u64)>,
    pub since: DateTime<Utc>,
}

impl ComplianceService {
    async fn active_tags<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        org: Uuid,
    ) -> Result<Vec<tracking_tag::Model>, Error> {
        Ok(tracking_tag::Entity::find()
            .scoped(tenant)
            .filter(tracking_tag::Column::OrgId.eq(org))
            .filter(tracking_tag::Column::Status.eq("active"))
            .order_by_asc(tracking_tag::Column::Id)
            .all(connection)
            .await?)
    }

    /// Cập nhật banner của nút `org` trong `transaction` (tạo nếu chưa có): đổi nhóm hiệu lực hay nhà
    /// cung cấp đang bật thì tăng `policy_version`.
    async fn refresh_banner<C: ConnectionTrait>(
        transaction: &C,
        tenant: &TenantContext,
        org: Uuid,
        change: &BannerChange,
        by: Option<Uuid>,
    ) -> Result<(), Error> {
        // Khoá dòng banner rồi mới đọc tag đang bật (D-181): hai lần thêm / sửa tag cùng lúc nối
        // đuôi nhau, bên sau thấy tag bên trước vừa commit và chữ ký bên trước vừa ghi — không thì
        // Postgres cho cả hai cùng tính chữ ký thiếu tag của bên kia, `policy_version` chỉ tăng một
        // lần và người dùng không bị hỏi đồng ý lại cho nhà cung cấp mới.
        let key = (tenant.id().as_uuid(), org);
        let mut current =
            rows::lock_one(transaction, cookie_setting::Entity::find_by_id(key)).await?;
        let at = now();
        if current.is_none() {
            // Chưa có banner: tạo. Hai bên cùng tạo thì bên sau không chèn được (khoá chính), khoá
            // dòng bên kia vừa tạo rồi cập nhật như thường.
            let tags = Self::active_tags(transaction, tenant, org).await?;
            let configured = change.categories.clone().unwrap_or_default();
            let categories = effective(&configured, &tags);
            let inserted = cookie_setting::Entity::insert(cookie_setting::ActiveModel::from(
                cookie_setting::Model {
                    tenant_id: tenant.id().as_uuid(),
                    org_id: org,
                    enabled: change.enabled.unwrap_or(true),
                    categories: json!(configured).to_string(),
                    policy_version: 1,
                    signature: signature(&categories, &tags),
                    position: change.position.clone().unwrap_or_else(|| "bottom".to_owned()),
                    updated_by: by,
                    updated_at: at,
                },
            ))
            .on_conflict(
                OnConflict::columns([
                    cookie_setting::Column::TenantId,
                    cookie_setting::Column::OrgId,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(transaction)
            .await?;
            if inserted > 0 {
                return Ok(());
            }
            current = rows::lock_one(transaction, cookie_setting::Entity::find_by_id(key)).await?;
        }
        let row = current.ok_or_else(|| Error::internal("cookie banner row vanished"))?;
        let tags = Self::active_tags(transaction, tenant, org).await?;
        let configured: Vec<String> = match &change.categories {
            Some(categories) => categories.clone(),
            None => serde_json::from_str(&row.categories).unwrap_or_default(),
        };
        let categories = effective(&configured, &tags);
        let signature = signature(&categories, &tags);
        let bump = row.signature != signature;
        let mut active: cookie_setting::ActiveModel = row.clone().into();
        if let Some(enabled) = change.enabled {
            active.enabled = Set(enabled);
        }
        if let Some(position) = &change.position {
            active.position = Set(position.clone());
        }
        active.categories = Set(json!(configured).to_string());
        if bump {
            active.policy_version = Set(row.policy_version + 1);
            active.signature = Set(signature);
        }
        active.updated_by = Set(by);
        active.updated_at = Set(at);
        active.update(transaction).await?;
        Ok(())
    }

    async fn banner_of(&self, tenant: &TenantContext, org: Uuid) -> Result<Banner, Error> {
        let read = tenant.database().read();
        let row =
            cookie_setting::Entity::find_by_id((tenant.id().as_uuid(), org)).one(read).await?;
        let tags = Self::active_tags(read, tenant, org).await?;
        Ok(match row {
            Some(row) => {
                let configured: Vec<String> =
                    serde_json::from_str(&row.categories).unwrap_or_default();
                Banner {
                    org_id: org,
                    enabled: row.enabled,
                    categories: effective(&configured, &tags),
                    configured,
                    policy_version: row.policy_version,
                    position: row.position,
                    updated_at: row.updated_at,
                    tags,
                }
            }
            // Chưa đặt gì: không banner, không mã.
            None => Banner {
                org_id: org,
                enabled: false,
                configured: Vec::new(),
                categories: effective(&[], &tags),
                policy_version: 0,
                position: "bottom".to_owned(),
                updated_at: DateTime::<Utc>::UNIX_EPOCH,
                tags,
            },
        })
    }

    /// Nút của website người gọi được quản lý mã theo dõi; ngoài phạm vi → lỗi phạm vi.
    async fn tracking_org(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Option<Uuid>,
    ) -> Result<OrgId, Error> {
        let org = self.website_org(tenant, org).await?;
        principal
            .require_at(tenant.database().read(), &TRACKING_MANAGE, OrgTarget::new(org, None))
            .await?;
        Ok(org)
    }

    pub(crate) async fn list_tags(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Option<Uuid>,
        page: &PageRequest,
    ) -> Result<Page<tracking_tag::Model>, Error> {
        principal.require(&TRACKING_MANAGE)?;
        let scope =
            principal.scope_condition(&TRACKING_MANAGE, tracking_tag::Column::OrgId, None)?;
        let mut select = tracking_tag::Entity::find().scoped(tenant).filter(scope);
        if let Some(org) = org {
            select = select.filter(tracking_tag::Column::OrgId.eq(org));
        }
        fetch_page(select, tracking_tag::Column::Id, |row| row.id, page, tenant.database().read())
            .await
    }

    async fn tag_in_scope(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<tracking_tag::Model, Error> {
        principal.require(&TRACKING_MANAGE)?;
        let row = tracking_tag::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(tag_not_found)?;
        principal
            .require_at(
                tenant.database().read(),
                &TRACKING_MANAGE,
                OrgTarget::new(OrgId::from_uuid(row.org_id), None),
            )
            .await
            .map_err(
                |error| if error.code() == "org_out_of_scope" { tag_not_found() } else { error },
            )?;
        Ok(row)
    }

    pub(crate) async fn tag(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<tracking_tag::Model, Error> {
        self.tag_in_scope(tenant, principal, id).await
    }

    async fn record_tracking_change(
        &self,
        transaction: &impl ConnectionTrait,
        tenant: &TenantContext,
        principal: &Principal,
        row: &tracking_tag::Model,
        action: &str,
    ) -> Result<(), Error> {
        let event = events::TrackingChanged(json!({
            "org_id": row.org_id, "tag_id": row.id, "provider": row.provider, "status": row.status,
            "action": action,
        }));
        self.outbox
            .record(
                transaction,
                tenant,
                Actor::from(principal),
                Some(format!("compliance.tracking_tag:{}", row.id)),
                &event,
            )
            .await
    }

    pub(crate) async fn create_tag(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Option<Uuid>,
        provider_key: &str,
        tracking_id: &str,
    ) -> Result<tracking_tag::Model, Error> {
        principal.require(&TRACKING_MANAGE)?;
        let tracking_id = check_tag(provider_key, tracking_id)?;
        let org = self.tracking_org(tenant, principal, org).await?;
        let at = now();
        let row = tracking_tag::Model {
            id: Uuid::now_v7(),
            tenant_id: tenant.id().as_uuid(),
            org_id: org.as_uuid(),
            provider: provider_key.to_owned(),
            tracking_id,
            status: "active".to_owned(),
            created_by: Some(principal.user_id()),
            created_at: at,
            updated_at: at,
        };
        let transaction = tenant.database().begin_write().await?;
        let saved =
            tracking_tag::ActiveModel::from(row).insert(&transaction).await.map_err(|error| {
                if crate::service::is_unique_violation(&error) {
                    tag_exists()
                } else {
                    error.into()
                }
            })?;
        Self::refresh_banner(
            &transaction,
            tenant,
            saved.org_id,
            &BannerChange::default(),
            Some(principal.user_id()),
        )
        .await?;
        self.record_tracking_change(&transaction, tenant, principal, &saved, "created").await?;
        transaction.commit().await?;
        Ok(saved)
    }

    pub(crate) async fn update_tag(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        status: Option<String>,
        tracking_id: Option<String>,
    ) -> Result<tracking_tag::Model, Error> {
        let before = self.tag_in_scope(tenant, principal, id).await?;
        if status.as_deref().is_some_and(|status| !["active", "paused"].contains(&status)) {
            return Err(Error::validation(vec![FieldError::new(
                "status",
                "invalid",
                "must be `active` or `paused`",
            )]));
        }
        let mut row = before.clone();
        if let Some(raw) = tracking_id {
            row.tracking_id = check_tag(&row.provider, &raw)?;
        }
        if let Some(status) = status {
            row.status = status;
        }
        row.updated_at = now();
        let transaction = tenant.database().begin_write().await?;
        let saved =
            tracking_tag::ActiveModel::from(row).reset_all().update(&transaction).await.map_err(
                |error| {
                    if crate::service::is_unique_violation(&error) {
                        tag_exists()
                    } else {
                        error.into()
                    }
                },
            )?;
        Self::refresh_banner(
            &transaction,
            tenant,
            saved.org_id,
            &BannerChange::default(),
            Some(principal.user_id()),
        )
        .await?;
        self.record_tracking_change(&transaction, tenant, principal, &saved, "updated").await?;
        transaction.commit().await?;
        Ok(saved)
    }

    pub(crate) async fn delete_tag(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        let row = self.tag_in_scope(tenant, principal, id).await?;
        let transaction = tenant.database().begin_write().await?;
        tracking_tag::Entity::delete_by_id(row.id).exec(&transaction).await?;
        Self::refresh_banner(
            &transaction,
            tenant,
            row.org_id,
            &BannerChange::default(),
            Some(principal.user_id()),
        )
        .await?;
        self.record_tracking_change(&transaction, tenant, principal, &row, "deleted").await?;
        transaction.commit().await?;
        Ok(())
    }

    pub(crate) async fn cookie_settings(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Uuid,
    ) -> Result<Banner, Error> {
        principal.require(&TRACKING_MANAGE)?;
        let org = self.tracking_org(tenant, principal, Some(org)).await?;
        self.banner_of(tenant, org.as_uuid()).await
    }

    pub(crate) async fn set_cookie_settings(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Uuid,
        change: BannerChange,
    ) -> Result<Banner, Error> {
        principal.require(&TRACKING_MANAGE)?;
        let mut problems = Vec::new();
        if let Some(categories) = &change.categories {
            let unique: BTreeSet<&String> = categories.iter().collect();
            if unique.len() != categories.len()
                || categories.iter().any(|item| !OPTIONAL.contains(&item.as_str()))
            {
                problems.push(FieldError::new(
                    "categories",
                    "invalid",
                    "must list each of `preferences`, `analytics`, `marketing` at most once (`necessary` is always on)",
                ));
            }
        }
        if change.position.as_deref().is_some_and(|position| !POSITIONS.contains(&position)) {
            problems.push(FieldError::new(
                "position",
                "invalid",
                "must be `bottom`, `top` or `center`",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let org = self.tracking_org(tenant, principal, Some(org)).await?;
        let transaction = tenant.database().begin_write().await?;
        Self::refresh_banner(
            &transaction,
            tenant,
            org.as_uuid(),
            &change,
            Some(principal.user_id()),
        )
        .await?;
        let event = events::TrackingChanged(json!({
            "org_id": org.as_uuid(), "tag_id": null, "provider": null, "status": null, "action": "banner_updated",
        }));
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                Some(format!("compliance.cookie_settings:{}", org.as_uuid())),
                &event,
            )
            .await?;
        transaction.commit().await?;
        self.banner_of(tenant, org.as_uuid()).await
    }

    /// Banner công khai của website `org`.
    pub(crate) async fn public_banner(
        &self,
        tenant: &TenantContext,
        org: OrgId,
    ) -> Result<Banner, Error> {
        self.banner_of(tenant, org.as_uuid()).await
    }

    /// Nhà cung cấp đang bật ở mọi website của khách, theo nhóm (bên nhận dữ liệu của sổ mục đích).
    pub(crate) async fn tracking_recipients(
        &self,
        tenant: &TenantContext,
    ) -> Result<Vec<(String, Recipient)>, Error> {
        let providers: Vec<String> = tracking_tag::Entity::find()
            .scoped(tenant)
            .filter(tracking_tag::Column::Status.eq("active"))
            .select_only()
            .column(tracking_tag::Column::Provider)
            .distinct()
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        let mut found: Vec<(String, Recipient)> = providers
            .iter()
            .filter_map(|key| provider(key))
            .map(|item| {
                (
                    item.category.to_owned(),
                    Recipient {
                        name: item.name.to_owned(),
                        country: Some(item.country.to_owned()),
                        source: "tracking".to_owned(),
                    },
                )
            })
            .collect();
        found.sort_by(|a, b| a.1.name.cmp(&b.1.name));
        Ok(found)
    }

    /// Ghi bằng chứng đồng ý cookie của người xem trên website `org`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn record_cookie_consent(
        &self,
        tenant: &TenantContext,
        org: OrgId,
        principal: Option<&Principal>,
        ip: Option<IpAddr>,
        consent_id: &str,
        categories: Vec<String>,
        policy_version: i32,
    ) -> Result<cookie_consent::Model, Error> {
        let mut problems = Vec::new();
        let valid_id = (16..=64).contains(&consent_id.len())
            && consent_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
        if !valid_id {
            problems.push(FieldError::new(
                "consent_id",
                "invalid",
                "must be 16 to 64 letters, digits, `-` or `_` (a random value kept in a necessary cookie)",
            ));
        }
        let chosen: BTreeSet<String> =
            categories.into_iter().filter(|item| item != "necessary").collect();
        if chosen.iter().any(|item| !OPTIONAL.contains(&item.as_str())) {
            problems.push(FieldError::new(
                "categories",
                "invalid",
                "must only contain `necessary`, `preferences`, `analytics` or `marketing`",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let banner = self.banner_of(tenant, org.as_uuid()).await?;
        if !banner.enabled || banner.policy_version != policy_version {
            return Err(policy_changed(banner.policy_version));
        }
        let ip_hash = match ip {
            Some(ip) => {
                let bucket = autoxide_kernel::ip_hash::bucket(
                    &self.secret,
                    "compliance.cookies",
                    ip,
                    Utc::now(),
                )
                .await?;
                let throttle = Throttle::new(
                    "compliance",
                    self.cookie_consents_per_hour,
                    std::time::Duration::from_secs(3600),
                )?;
                if let Outcome::Limited { retry_after } =
                    throttle.hit(tenant, &format!("cookie-ip:{bucket}")).await?
                {
                    return Err(consents_throttled(retry_after));
                }
                Some(
                    autoxide_kernel::ip_hash::fingerprint(&self.secret, "compliance.cookies", ip)
                        .await?,
                )
            }
            None => None,
        };
        // Chỉ nhóm banner đang hỏi; nhóm khác (đã bỏ) không ghi.
        let chosen: Vec<String> =
            banner.categories.iter().filter(|item| chosen.contains(*item)).cloned().collect();
        let row = cookie_consent::Model {
            id: Uuid::now_v7(),
            tenant_id: tenant.id().as_uuid(),
            org_id: org.as_uuid(),
            consent_id: consent_id.to_owned(),
            categories: json!(chosen).to_string(),
            policy_version,
            ip_hash,
            user_id: principal.map(Principal::user_id),
            created_at: now(),
        };
        let saved =
            cookie_consent::ActiveModel::from(row).insert(tenant.database().write()).await?;
        // Người đã đăng nhập: ghi thêm vào sổ đồng ý chung (mục đích `analytics` / `marketing`).
        if let Some(principal) = principal {
            let subject = ConsentSubject::User(principal.user_id());
            for purpose in ["analytics", "marketing"] {
                if !banner.categories.iter().any(|item| item == purpose) {
                    continue;
                }
                let key = ConsentKey::new(purpose, format!("website:{}", org.as_uuid()), "");
                let source_ref = format!("compliance.cookie_consent:{}:{purpose}", saved.id);
                if chosen.iter().any(|item| item == purpose) {
                    let evidence = ConsentEvidence {
                        text: format!("Cookie banner, category `{purpose}`"),
                        text_version: format!("cookie-policy-v{policy_version}"),
                        source: "cookie_banner".to_owned(),
                        ip_fingerprint: saved.ip_hash.clone(),
                    };
                    self.grant_consent(
                        tenant,
                        Actor::from(principal),
                        &subject,
                        &key,
                        &evidence,
                        &source_ref,
                    )
                    .await?;
                } else {
                    self.withdraw_consent(
                        tenant,
                        Actor::from(principal),
                        &subject,
                        &key,
                        "cookie_banner",
                        &source_ref,
                    )
                    .await?;
                }
            }
        }
        Ok(saved)
    }

    /// Tỉ lệ đồng ý theo nhóm trong 30 ngày (số gộp, không bản ghi riêng).
    pub(crate) async fn cookie_stats(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Uuid,
    ) -> Result<Stats, Error> {
        principal.require(&TRACKING_MANAGE)?;
        let org = self.tracking_org(tenant, principal, Some(org)).await?;
        let since = Utc::now() - Duration::days(30);
        let read = tenant.database().read();
        let base = || {
            cookie_consent::Entity::find()
                .scoped(tenant)
                .filter(cookie_consent::Column::OrgId.eq(org.as_uuid()))
                .filter(cookie_consent::Column::CreatedAt.gte(since))
        };
        let total = base().count(read).await?;
        let mut by_category = Vec::new();
        for category in OPTIONAL {
            let count = base()
                .filter(cookie_consent::Column::Categories.like(format!("%\"{category}\"%")))
                .count(read)
                .await?;
            by_category.push((category.to_owned(), count));
        }
        Ok(Stats { total, by_category, since })
    }

    /// Xoá bằng chứng đồng ý cookie quá hạn giữ (quy tắc `privacy.consent_proof_days` của từng khách).
    pub(crate) async fn purge_cookie_consents(
        &self,
        database: &autoxide_kernel::database::Database,
    ) -> Result<u64, Error> {
        let tenants: Vec<Uuid> = cookie_consent::Entity::find()
            .select_only()
            .column(cookie_consent::Column::TenantId)
            .distinct()
            .limit(1000)
            .into_tuple()
            .all(database.read())
            .await?;
        let mut deleted = 0;
        for tenant_id in tenants {
            let Ok(tenant) = self.tenant(TenantId::from_uuid(tenant_id)).await else {
                continue;
            };
            let jurisdiction = self.jurisdiction(&tenant).await?;
            let days = self.consent_proof_days_in(&jurisdiction).await?;
            deleted += cookie_consent::Entity::delete_many()
                .filter(
                    cookie_consent::Column::CreatedAt
                        .lt(Utc::now() - Duration::days(i64::from(days))),
                )
                .scoped(&tenant)
                .exec(database.write())
                .await?
                .rows_affected;
        }
        Ok(deleted)
    }
}

/// Job hằng ngày `compliance.consent_proof_purge`.
pub(crate) struct PurgeJob(pub Arc<ComplianceService>);

#[async_trait::async_trait]
impl JobHandler for PurgeJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.purge_cookie_consents(&job.database).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ma_id_dung_dang_theo_nha_cung_cap() {
        assert_eq!(check_tag("ga4", " g-ab12cd34ef ").unwrap(), "G-AB12CD34EF");
        assert!(check_tag("ga4", "UA-12345-1").is_err());
        assert!(check_tag("gtm", "GTM-ABC1234").is_ok());
        assert!(check_tag("google_ads", "AW-1234567890").is_ok());
        assert!(check_tag("google_ads", "AW-12AB").is_err());
        assert!(check_tag("meta", "1234567890123456").is_ok());
        assert!(check_tag("meta", "<script>").is_err());
        assert!(check_tag("tiktok", "C1A2B3C4D5E6F7G8H9I0").is_ok());
        assert!(check_tag("zalo", "1234567890123").is_ok());
        let error = check_tag("ga4", "nope").unwrap_err();
        assert_eq!(error.code(), "compliance.tracking_id_invalid");
        assert_eq!(
            check_tag("hotjar", "1").unwrap_err().code(),
            "compliance.tracking_provider_unknown"
        );
    }

    #[test]
    fn moi_nha_cung_cap_co_vi_du_hop_le_va_nhom_dung() {
        for item in PROVIDERS {
            assert!(item.format.accepts(item.example), "{}", item.key);
            assert!(["analytics", "marketing"].contains(&item.category), "{}", item.key);
            assert!(!item.hosts.is_empty(), "{}", item.key);
        }
    }
}
