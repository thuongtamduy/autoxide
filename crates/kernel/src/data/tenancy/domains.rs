//! Tên miền riêng của khách (đặc tả 33 "API REST", D-190): khách gắn tên miền của mình (vd
//! `shop.example`), chứng minh sở hữu bằng một bản ghi TXT, rồi request tới tên miền đó vào thẳng
//! khách (`request.rs`, bước 4).
//!
//! - Bảng `kernel_tenant_domains` thuộc mặt điều khiển (chỉ database điều khiển có dữ liệu); một tên
//!   miền thuộc đúng một khách trên cả nền tảng (unique index).
//! - Trạng thái: `pending` (chờ bản ghi TXT) → `verified`; `pending` quá 7 ngày → `failed`; xoá →
//!   `revoked`. `failed` xác minh lại được (`POST /domains/{id}/verify`). Tên miền `failed` /
//!   `revoked` của khách khác thì khách mới nhận lại được — người chưa chứng minh sở hữu không giữ
//!   chỗ mãi (chỉ `pending` và `verified` chặn khách khác).
//! - Bản ghi phải đặt: TXT `_autoxide-challenge.<domain>` = `autoxide-verify=<verification_token>`;
//!   tra qua trait [`DnsTxtLookup`] (bản thật dùng `hickory-resolver`, `dns.rs`; test dùng bản giả).
//! - `updated_at` là lúc trạng thái đổi lần cuối: tên miền `pending` hỏng sau 7 ngày tính từ đó.
//! - Event `kernel.domain_added` / `kernel.domain_verified` / `kernel.domain_revoked` ghi vào
//!   outbox của `kernel` trong database của khách (như event của `TenantAdmin`).
//! - Job `kernel.verify_domains` (mỗi 10 phút) tra lại tên miền `pending`.
//! - Xác minh / thu hồi xong thì bỏ tên miền khỏi cache của sổ database ngay; máy chủ khác thấy sau
//!   tối đa `AUTOXIDE_TENANCY_CACHE_SECONDS`.

mod dns;
mod http;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::errors::{
    self, tenant_domain_dns_failed, tenant_domain_invalid, tenant_domain_not_found,
    tenant_domain_taken,
};
use super::{DatabaseRegistry, TenancyMode, TenantContext, TenantId, TenantStatus};
use crate::access::Permission;
use crate::database::Database;
use crate::error::{Error, ErrorKind};
use crate::event::{Actor, Event, EventInfo, Outbox};
use crate::job::{JobContext, JobHandler, Schedule};
use crate::pagination::{Page, PageRequest, fetch_page};

pub(crate) use http::{routes, tools};

/// Quản lý tên miền riêng của khách mình.
pub const DOMAINS_MANAGE: Permission = Permission::new(
    "kernel.domains.manage",
    "Add the tenant's own domains, verify them with a DNS record and remove them.",
);

pub(crate) const DOMAIN_ADDED: EventInfo = EventInfo::new(
    "kernel.domain_added",
    "A domain was added to the tenant and waits for its DNS record.",
);
pub(crate) const DOMAIN_VERIFIED: EventInfo = EventInfo::new(
    "kernel.domain_verified",
    "A domain of the tenant was verified; requests to it now reach the tenant.",
);
pub(crate) const DOMAIN_REVOKED: EventInfo =
    EventInfo::new("kernel.domain_revoked", "A domain was removed from the tenant.");

/// Loại job (cũng là tên lịch) tra lại tên miền đang chờ.
pub(crate) const VERIFY_DOMAINS: &str = "kernel.verify_domains";

/// Tên bản ghi TXT đứng trước tên miền.
pub const CHALLENGE_PREFIX: &str = "_autoxide-challenge";
/// Giá trị bản ghi TXT đứng trước token.
pub const CHALLENGE_VALUE_PREFIX: &str = "autoxide-verify=";

/// Tên miền `pending` quá lâu chừng này thì `failed`.
const PENDING_DAYS: i64 = 7;
/// Chờ DNS trả lời tối đa chừng này (job và request đều không treo lâu hơn).
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);
/// Số tên miền một lần job tra (lần sau tra tiếp phần còn lại).
const SWEEP_BATCH: u64 = 200;
/// Độ dài tối đa của `check_error` (cột `varchar(200)`).
const CHECK_ERROR_MAX: usize = 200;

/// Tra bản ghi TXT của một tên. Kernel dùng bộ tra DNS của hệ điều hành; module (hay testkit) công
/// bố `dyn DnsTxtLookup` để thay (`context.provide::<dyn DnsTxtLookup>(…)`).
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait DnsTxtLookup: Send + Sync {
    /// Mọi giá trị TXT của `name` (tên đầy đủ, vd `_autoxide-challenge.shop.example`); rỗng khi
    /// tên không có bản ghi TXT hay không tồn tại. Lỗi khi không hỏi được DNS.
    async fn txt(&self, name: &str) -> Result<Vec<String>, Error>;
}

/// Bộ tra DNS dùng cho tên miền: bản module / testkit công bố, hoặc bộ tra của hệ điều hành. Điền
/// một lần sau khi mọi module khởi động (`Modules::start_in`). Clone rẻ.
#[derive(Clone, Default)]
pub(crate) struct DnsSlot(Arc<OnceLock<Arc<dyn DnsTxtLookup>>>);

impl DnsSlot {
    pub(crate) fn fill(&self, lookup: Option<Arc<dyn DnsTxtLookup>>) {
        if let Some(lookup) = lookup {
            let _ = self.0.set(lookup);
        }
    }

    fn get(&self) -> Arc<dyn DnsTxtLookup> {
        self.0.get_or_init(|| Arc::new(dns::SystemDns::default())).clone()
    }
}

/// Tên miền đã chuẩn hoá (chữ thường, không cổng, không dấu chấm cuối), hoặc `None` khi không
/// phải tên miền dùng được: chỉ nhãn ASCII chữ / số / `-` (tên quốc tế viết dạng punycode
/// `xn--…`), mỗi nhãn 1–63 ký tự, cả tên tối đa 253, ít nhất hai nhãn; không IP, không
/// `localhost` hay `*.localhost`.
pub fn normalize_domain(raw: &str) -> Option<String> {
    let mut name = raw.trim().to_ascii_lowercase();
    if let Some((host, port)) = name.rsplit_once(':')
        && !port.is_empty()
        && port.bytes().all(|b| b.is_ascii_digit())
    {
        name = host.to_owned();
    }
    let name = name.strip_suffix('.').unwrap_or(&name);
    if name.is_empty() || name.len() > 253 || !name.contains('.') {
        return None;
    }
    let labels_ok = name.split('.').all(|label| {
        (1..=63).contains(&label.len())
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    });
    // Nhãn cuối toàn số: địa chỉ IPv4 (`10.0.0.1`), không phải tên miền.
    let tld_ok =
        name.rsplit('.').next().is_some_and(|tld| !tld.bytes().all(|b| b.is_ascii_digit()));
    let local = name == "localhost" || name.ends_with(".localhost");
    (labels_ok && tld_ok && !local).then(|| name.to_owned())
}

/// Tên máy của `AUTOXIDE_HTTP_PUBLIC_URL` (`https://API.example.com:8443/x` → `api.example.com`).
pub(crate) fn host_of_url(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    normalize_domain(host)
}

mod entity {
    //! Bảng `kernel_tenant_domains` (mặt điều khiển).
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "kernel_tenant_domains")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub org_id: Option<Uuid>,
        pub domain: String,
        pub status: String,
        pub verification_token: String,
        pub verified_at: Option<DateTimeUtc>,
        pub last_checked_at: Option<DateTimeUtc>,
        pub check_error: Option<String>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

use entity::{Column, Entity, Model};

const PENDING: &str = "pending";
const VERIFIED: &str = "verified";
const FAILED: &str = "failed";
const REVOKED: &str = "revoked";

/// The DNS record that proves the tenant owns the domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = TenantDomainRecord)]
pub struct DomainRecord {
    /// Always `TXT`.
    #[serde(rename = "type")]
    #[schema(example = "TXT")]
    pub record_type: String,
    /// Full name of the record.
    #[schema(example = "_autoxide-challenge.shop.example")]
    pub name: String,
    /// Exact value of the record.
    #[schema(example = "autoxide-verify=6Jx1rYqQe2m3Vb0nC5tWk8sLp4dZa7Hf")]
    pub value: String,
}

/// A domain of the tenant: requests to a verified domain reach this tenant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TenantDomain {
    pub id: Uuid,
    /// Lower case, without port or trailing dot; international names in punycode.
    #[schema(example = "shop.example")]
    pub domain: String,
    /// `pending` (waits for the DNS record), `verified`, or `failed` (not verified within 7
    /// days: fix the record and verify again).
    #[schema(example = "pending")]
    pub status: String,
    /// The DNS record to set at your DNS provider, then call `POST /domains/{id}/verify`.
    pub record: DomainRecord,
    pub verified_at: Option<DateTime<Utc>>,
    /// Last time the DNS record was looked up.
    pub last_checked_at: Option<DateTime<Utc>>,
    /// Why the last lookup did not verify the domain.
    pub check_error: Option<String>,
    pub created_at: DateTime<Utc>,
    /// When the status last changed; a `pending` domain fails 7 days after it.
    pub updated_at: DateTime<Utc>,
}

/// Payload of `kernel.domain_added`, `kernel.domain_verified` and `kernel.domain_revoked`.
#[derive(Debug, Serialize)]
struct DomainChanged {
    id: Uuid,
    domain: String,
    status: String,
}

#[derive(Debug, Serialize)]
#[serde(transparent)]
struct DomainAdded(DomainChanged);

#[derive(Debug, Serialize)]
#[serde(transparent)]
struct DomainVerified(DomainChanged);

#[derive(Debug, Serialize)]
#[serde(transparent)]
struct DomainRevoked(DomainChanged);

impl Event for DomainAdded {
    const NAME: &'static str = "kernel.domain_added";
}

impl Event for DomainVerified {
    const NAME: &'static str = "kernel.domain_verified";
}

impl Event for DomainRevoked {
    const NAME: &'static str = "kernel.domain_revoked";
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

fn describe(row: Model) -> TenantDomain {
    TenantDomain {
        record: DomainRecord {
            record_type: "TXT".to_owned(),
            name: format!("{CHALLENGE_PREFIX}.{}", row.domain),
            value: format!("{CHALLENGE_VALUE_PREFIX}{}", row.verification_token),
        },
        id: row.id,
        domain: row.domain,
        status: row.status,
        verified_at: row.verified_at,
        last_checked_at: row.last_checked_at,
        check_error: row.check_error,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// Kết quả một lần tra bản ghi TXT.
enum Probe {
    Found,
    /// Chưa thấy bản ghi đúng: lý do (tiếng Anh, cho người quản trị).
    Missing(String),
}

/// Dịch vụ tên miền riêng của khách. Module `kernel` công bố (`context.require::<TenantDomains>()`).
pub struct TenantDomains {
    registry: DatabaseRegistry,
    outbox: Outbox,
    dns: DnsSlot,
    /// Tên máy của `AUTOXIDE_HTTP_PUBLIC_URL`: khách không được nhận.
    platform_host: Option<String>,
}

impl std::fmt::Debug for TenantDomains {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TenantDomains").finish_non_exhaustive()
    }
}

impl TenantDomains {
    pub(crate) fn new(
        registry: DatabaseRegistry,
        outbox: Outbox,
        dns: DnsSlot,
        platform_host: Option<String>,
    ) -> Self {
        TenantDomains { registry, outbox, dns, platform_host }
    }

    /// Bản cài chạy chế độ nhiều khách (chỉ khi đó tên miền riêng có tác dụng).
    pub(crate) fn multi_tenant(&self) -> bool {
        self.registry.mode() == TenancyMode::Multi
    }

    fn control(&self) -> &Database {
        self.registry.control()
    }

    /// Tên miền khách gửi, đã chuẩn hoá; không dùng được: `tenant_domain_invalid`.
    fn checked(&self, raw: &str) -> Result<String, Error> {
        let domain = normalize_domain(raw).ok_or_else(|| tenant_domain_invalid(raw))?;
        if self.platform_host.as_deref() == Some(domain.as_str()) {
            return Err(tenant_domain_invalid(raw));
        }
        Ok(domain)
    }

    /// Một trang tên miền của khách `tenant` (trừ tên miền đã thu hồi), mới nhất trước trừ khi
    /// `order=asc`.
    pub async fn list(
        &self,
        tenant: &TenantContext,
        page: &PageRequest,
    ) -> Result<Page<TenantDomain>, Error> {
        let select = Entity::find()
            .filter(Column::TenantId.eq(tenant.id().as_uuid()))
            .filter(Column::Status.ne(REVOKED));
        let rows =
            fetch_page(select, Column::Id, |row| row.id, page, self.control().read()).await?;
        Ok(rows.map(describe))
    }

    /// Thêm tên miền `raw` cho khách `tenant` (trạng thái `pending`, token xác minh mới). Gửi lại
    /// tên miền khách đang có (`pending` / `verified`) trả về chính nó, không đổi gì.
    pub async fn add(
        &self,
        actor: Actor,
        tenant: &TenantContext,
        raw: &str,
    ) -> Result<TenantDomain, Error> {
        if self.registry.mode() == TenancyMode::Single {
            return Err(errors::tenant_single_mode());
        }
        let domain = self.checked(raw)?;
        let token = crate::secret::random_token(24)?;
        let existing = self.find_domain(&domain).await?;
        let row = match existing {
            None => {
                let now = now();
                let row = entity::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    org_id: Set(None),
                    domain: Set(domain.clone()),
                    status: Set(PENDING.to_owned()),
                    verification_token: Set(token),
                    verified_at: Set(None),
                    last_checked_at: Set(None),
                    check_error: Set(None),
                    created_at: Set(now),
                    updated_at: Set(now),
                };
                // Hai khách gửi cùng tên miền cùng lúc: unique index chặn bên sau.
                row.insert(self.control().write()).await.map_err(|error| {
                    if matches!(
                        error.sql_err(),
                        Some(sea_orm::SqlErr::UniqueConstraintViolation(_))
                    ) {
                        tenant_domain_taken(&domain)
                    } else {
                        Error::from(error)
                    }
                })?
            }
            Some(row)
                if row.tenant_id == tenant.id().as_uuid()
                    && (row.status == PENDING || row.status == VERIFIED) =>
            {
                return Ok(describe(row));
            }
            Some(row) if row.status == PENDING || row.status == VERIFIED => {
                return Err(tenant_domain_taken(&domain));
            }
            Some(row) => {
                // `failed` / `revoked` (của khách này hay khách khác): nhận lại từ đầu.
                let now = now();
                let updated = Entity::update_many()
                    .col_expr(Column::TenantId, Expr::value(tenant.id().as_uuid()))
                    .col_expr(Column::OrgId, Expr::value(Option::<Uuid>::None))
                    .col_expr(Column::Status, Expr::value(PENDING))
                    .col_expr(Column::VerificationToken, Expr::value(token))
                    .col_expr(Column::VerifiedAt, Expr::value(Option::<DateTime<Utc>>::None))
                    .col_expr(Column::LastCheckedAt, Expr::value(Option::<DateTime<Utc>>::None))
                    .col_expr(Column::CheckError, Expr::value(Option::<String>::None))
                    .col_expr(Column::CreatedAt, Expr::value(now))
                    .col_expr(Column::UpdatedAt, Expr::value(now))
                    .filter(Column::Id.eq(row.id))
                    .filter(Column::Status.eq(row.status.as_str()))
                    .filter(Column::UpdatedAt.eq(row.updated_at))
                    .exec(self.control().write())
                    .await?;
                if updated.rows_affected == 0 {
                    // Ai đó vừa đổi dòng này (vừa nhận lại, vừa xác minh lại).
                    return Err(tenant_domain_taken(&domain));
                }
                self.find(row.id).await?
            }
        };
        self.registry.invalidate_domain(&domain);
        let event = DomainAdded(changed(&row));
        self.record(tenant, actor, &row, &event).await?;
        tracing::info!(tenant_id = %tenant.id(), domain = %row.domain, "tenant domain added");
        Ok(describe(row))
    }

    /// Tra bản ghi TXT của tên miền `id` của khách `tenant` ngay, trả trạng thái mới. Tên miền
    /// `failed` xác minh lại được; tên miền đã xác minh trả về như cũ.
    pub async fn verify(
        &self,
        actor: Actor,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<TenantDomain, Error> {
        let row = self.owned(tenant, id).await?;
        if row.status == VERIFIED {
            return Ok(describe(row));
        }
        let row = self.check(actor, row).await?;
        Ok(describe(row))
    }

    /// Thu hồi tên miền `id` của khách `tenant` (`revoked`): request tới tên miền đó không còn vào
    /// khách. Thu hồi lại tên miền đã thu hồi không đổi gì.
    pub async fn revoke(
        &self,
        actor: Actor,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<(), Error> {
        let row = self.find_owned(tenant, id).await?;
        if row.status == REVOKED {
            return Ok(());
        }
        let now = now();
        let updated = Entity::update_many()
            .col_expr(Column::Status, Expr::value(REVOKED))
            .col_expr(Column::UpdatedAt, Expr::value(now))
            .filter(Column::Id.eq(row.id))
            .filter(Column::TenantId.eq(tenant.id().as_uuid()))
            .filter(Column::Status.ne(REVOKED))
            .exec(self.control().write())
            .await?;
        self.registry.invalidate_domain(&row.domain);
        if updated.rows_affected > 0 {
            let row = Model { status: REVOKED.to_owned(), updated_at: now, ..row };
            let event = DomainRevoked(changed(&row));
            self.record(tenant, actor, &row, &event).await?;
            tracing::info!(tenant_id = %tenant.id(), domain = %row.domain, "tenant domain revoked");
        }
        Ok(())
    }

    /// Tên miền `raw` đã xác minh và khách của nó đang hoạt động — cho câu hỏi `ask` của Caddy
    /// (cấp chứng chỉ TLS theo yêu cầu). Đi qua cache của sổ database.
    pub async fn allowed(&self, raw: &str) -> Result<bool, Error> {
        let Some(domain) = normalize_domain(raw) else {
            return Ok(false);
        };
        match self.registry.tenant_by_domain(&domain).await {
            Ok(Some(tenant)) => Ok(tenant.status() == TenantStatus::Active),
            Ok(None) => Ok(false),
            // Dòng tên miền trỏ khách đã mất: coi như không có.
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Tra lại các tên miền `pending` trên `database` (job `kernel.verify_domains`): bản ghi đúng
    /// → `verified`; chờ quá 7 ngày → `failed`. Trả số tên miền đã xét.
    pub async fn check_pending(&self, database: &Database) -> Result<usize, Error> {
        let rows = Entity::find()
            .filter(Column::Status.eq(PENDING))
            .order_by_asc(Column::LastCheckedAt)
            .order_by_asc(Column::Id)
            .limit(SWEEP_BATCH)
            .all(database.read())
            .await?;
        let deadline = Utc::now() - chrono::Duration::days(PENDING_DAYS);
        let count = rows.len();
        for row in rows {
            if row.updated_at <= deadline {
                self.expire(database, row).await?;
            } else {
                self.check_on(database, Actor::system(), row).await?;
            }
        }
        Ok(count)
    }

    /// `pending` quá hạn → `failed`.
    async fn expire(&self, database: &Database, row: Model) -> Result<(), Error> {
        let now = now();
        let reason = format!(
            "Not verified within {PENDING_DAYS} days. Set the TXT record, then verify again."
        );
        Entity::update_many()
            .col_expr(Column::Status, Expr::value(FAILED))
            .col_expr(Column::CheckError, Expr::value(reason))
            .col_expr(Column::UpdatedAt, Expr::value(now))
            .filter(Column::Id.eq(row.id))
            .filter(Column::Status.eq(PENDING))
            .exec(database.write())
            .await?;
        tracing::info!(domain = %row.domain, "tenant domain not verified in time; marked failed");
        Ok(())
    }

    /// Tra DNS cho `row` (trên database điều khiển) và ghi kết quả.
    async fn check(&self, actor: Actor, row: Model) -> Result<Model, Error> {
        let database = self.control().clone();
        self.check_on(&database, actor, row).await
    }

    /// Tra DNS cho `row` (dòng của `database`) rồi ghi kết quả; trả dòng sau khi ghi. Chỉ một bên
    /// (request hay job chạy cùng lúc) chuyển được `pending` / `failed` → `verified` và ghi event.
    async fn check_on(
        &self,
        database: &Database,
        actor: Actor,
        row: Model,
    ) -> Result<Model, Error> {
        // Tra DNS NGOÀI mọi transaction: gọi mạng có thể chậm.
        let probe = self.probe(&row.domain, &row.verification_token).await;
        let now = now();
        match probe {
            Probe::Found => {
                let updated = Entity::update_many()
                    .col_expr(Column::Status, Expr::value(VERIFIED))
                    .col_expr(Column::VerifiedAt, Expr::value(now))
                    .col_expr(Column::LastCheckedAt, Expr::value(now))
                    .col_expr(Column::CheckError, Expr::value(Option::<String>::None))
                    .col_expr(Column::UpdatedAt, Expr::value(now))
                    .filter(Column::Id.eq(row.id))
                    .filter(Column::Status.is_in([PENDING, FAILED]))
                    .filter(Column::VerificationToken.eq(row.verification_token.as_str()))
                    .exec(database.write())
                    .await?;
                self.registry.invalidate_domain(&row.domain);
                let after = self.find_in(database, row.id).await?;
                if updated.rows_affected > 0 {
                    let tenant = self.registry.tenant(TenantId::from_uuid(after.tenant_id)).await?;
                    let event = DomainVerified(changed(&after));
                    self.record(&tenant, actor, &after, &event).await?;
                    tracing::info!(
                        tenant_id = %tenant.id(),
                        domain = %after.domain,
                        "tenant domain verified"
                    );
                }
                Ok(after)
            }
            Probe::Missing(reason) => {
                let reason: String = reason.chars().take(CHECK_ERROR_MAX).collect();
                let mut update = Entity::update_many()
                    .col_expr(Column::LastCheckedAt, Expr::value(now))
                    .col_expr(Column::CheckError, Expr::value(reason));
                if row.status == FAILED {
                    // Xác minh lại tên miền đã hỏng: chờ thêm 7 ngày từ bây giờ.
                    update = update
                        .col_expr(Column::Status, Expr::value(PENDING))
                        .col_expr(Column::UpdatedAt, Expr::value(now));
                }
                update
                    .filter(Column::Id.eq(row.id))
                    .filter(Column::Status.eq(row.status.as_str()))
                    .exec(database.write())
                    .await?;
                self.find_in(database, row.id).await
            }
        }
    }

    /// Tra bản ghi TXT của `domain`, so với `token`.
    async fn probe(&self, domain: &str, token: &str) -> Probe {
        let name = format!("{CHALLENGE_PREFIX}.{domain}");
        let expected = format!("{CHALLENGE_VALUE_PREFIX}{token}");
        let lookup = self.dns.get();
        match tokio::time::timeout(LOOKUP_TIMEOUT, lookup.txt(&name)).await {
            Ok(Ok(values)) if values.iter().any(|value| value.trim() == expected) => Probe::Found,
            Ok(Ok(values)) if values.is_empty() => {
                Probe::Missing(format!("No TXT record found at {name}."))
            }
            Ok(Ok(_)) => Probe::Missing(format!(
                "The TXT record at {name} does not hold the expected value."
            )),
            Ok(Err(error)) => {
                tracing::warn!(domain, error = %error.message(), "DNS lookup failed");
                Probe::Missing(format!("DNS lookup failed: {}", error.message()))
            }
            Err(_) => Probe::Missing(format!(
                "DNS lookup timed out after {} seconds.",
                LOOKUP_TIMEOUT.as_secs()
            )),
        }
    }

    async fn find_domain(&self, domain: &str) -> Result<Option<Model>, Error> {
        Ok(Entity::find().filter(Column::Domain.eq(domain)).one(self.control().read()).await?)
    }

    async fn find(&self, id: Uuid) -> Result<Model, Error> {
        self.find_in(self.control(), id).await
    }

    async fn find_in(&self, database: &Database, id: Uuid) -> Result<Model, Error> {
        // Đọc qua kết nối ghi: dòng vừa ghi (read replica có thể chưa kịp có).
        Entity::find_by_id(id).one(database.write()).await?.ok_or_else(tenant_domain_not_found)
    }

    /// Dòng `id` của khách `tenant` (kể cả đã thu hồi); dòng của khách khác là "không có".
    async fn find_owned(&self, tenant: &TenantContext, id: Uuid) -> Result<Model, Error> {
        Entity::find_by_id(id)
            .filter(Column::TenantId.eq(tenant.id().as_uuid()))
            .one(self.control().read())
            .await?
            .ok_or_else(tenant_domain_not_found)
    }

    /// Như [`find_owned`](Self::find_owned), không tính tên miền đã thu hồi.
    async fn owned(&self, tenant: &TenantContext, id: Uuid) -> Result<Model, Error> {
        let row = self.find_owned(tenant, id).await?;
        if row.status == REVOKED { Err(tenant_domain_not_found()) } else { Ok(row) }
    }

    /// Ghi `event` của tên miền `row` vào outbox của `kernel` trong database của khách.
    async fn record<E: Event>(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        row: &Model,
        event: &E,
    ) -> Result<(), Error> {
        let subject = format!("kernel.domain:{}", row.id);
        self.outbox.record(tenant.database().write(), tenant, actor, Some(subject), event).await
    }
}

fn changed(row: &Model) -> DomainChanged {
    DomainChanged { id: row.id, domain: row.domain.clone(), status: row.status.clone() }
}

/// Lịch của job `kernel.verify_domains`: mỗi 10 phút, chỉ trên database điều khiển (nơi duy nhất
/// có tên miền).
pub(crate) fn schedule() -> Schedule {
    Schedule::new(VERIFY_DOMAINS, VERIFY_DOMAINS, Duration::from_secs(600)).on_control_only()
}

/// Bộ chạy job `kernel.verify_domains` (job của cả database điều khiển).
pub(crate) struct VerifyDomains(pub(crate) Arc<TenantDomains>);

#[async_trait::async_trait]
impl JobHandler for VerifyDomains {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let checked = self.0.check_pending(&job.database).await?;
        if checked > 0 {
            tracing::info!(checked, "pending tenant domains checked");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chuan_hoa_ten_mien() {
        assert_eq!(normalize_domain(" Shop.Example. ").as_deref(), Some("shop.example"));
        assert_eq!(normalize_domain("shop.example:8443").as_deref(), Some("shop.example"));
        assert_eq!(
            normalize_domain("xn--d1acufc.xn--p1ai").as_deref(),
            Some("xn--d1acufc.xn--p1ai")
        );
        assert_eq!(normalize_domain("a-1.b.example").as_deref(), Some("a-1.b.example"));
        for bad in [
            "",
            "example",
            "localhost",
            "app.localhost",
            "10.0.0.1",
            "[::1]",
            "::1",
            "shop..example",
            "-shop.example",
            "shop-.example",
            "shop_x.example",
            "https://shop.example",
            "shop.example/path",
            "*.example.com",
            "cửahàng.vn",
        ] {
            assert_eq!(normalize_domain(bad), None, "{bad}");
        }
        let long = format!("{}.example", "a".repeat(64));
        assert_eq!(normalize_domain(&long), None);
        let many = format!("{}example", "abcdefghi.".repeat(25));
        assert!(many.len() > 253);
        assert_eq!(normalize_domain(&many), None);
    }

    #[test]
    fn ten_may_cua_url_cong_khai() {
        assert_eq!(host_of_url("https://API.example.com").as_deref(), Some("api.example.com"));
        assert_eq!(
            host_of_url("http://api.example.com:8443/x").as_deref(),
            Some("api.example.com")
        );
        assert_eq!(host_of_url("http://127.0.0.1:8080"), None);
    }
}
