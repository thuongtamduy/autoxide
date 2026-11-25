//! A tenant's own domains (spec 33 "REST API", D-190): a tenant attaches its own domain (e.g.
//! `shop.example`), proves ownership with a TXT record, and then requests to that domain go straight to the
//! tenant (`request.rs`, step 4).
//!
//! - The `kernel_tenant_domains` table belongs to the control plane (only the control database holds data); a
//!   domain belongs to exactly one tenant across the whole platform (unique index).
//! - Status: `pending` (waiting for the TXT record) → `verified`; `pending` for more than 7 days →
//!   `failed`; deleted → `revoked`. A `verified` domain is rechecked daily: 3 consecutive checks that do
//!   not find the right record (`check_failures`) → `failed`, and requests to it no longer reach the
//!   tenant; one successful check resets the count to 0. A DNS that does not answer (error, timeout) does
//!   not count as "not found": a DNS outage does not take away a tenant's domain. A `failed` domain can be
//!   verified again (`POST /domains/{id}/verify`). A `failed` / `revoked` domain of one tenant can be
//!   claimed by another tenant: someone who has not proved ownership does not hold a domain forever (only
//!   `pending` and `verified` block other tenants).
//! - The record to set: TXT `_autoxide-challenge.<domain>` = `autoxide-verify=<verification_token>`;
//!   looked up through the trait [`DnsTxtLookup`] (the real one uses `hickory-resolver`, `dns.rs`; tests use
//!   a fake).
//! - `updated_at` is when the status last changed: a `pending` domain fails 7 days after it.
//! - Event `kernel.domain_added` / `kernel.domain_verified` / `kernel.domain_revoked` /
//!   `kernel.domain_failed` / `kernel.domain_updated` are written to the `kernel` outbox in the tenant's
//!   database (like the events of `TenantAdmin`).
//! - A domain can be attached to a node of the tenant's organization tree (`org_id`, see the wave 2
//!   design): the node must belong to the tenant, asked through the trait [`OrgNodes`] (published by the
//!   step 34 organization-tree module); if nobody publishes it, the domain can only be detached (`null`).
//!   Attaching / detaching does not change the status or the 7-day deadline.
//! - The job `kernel.verify_domains` (every 10 minutes) rechecks `pending` domains, and `verified` domains
//!   more than a day past their last check.
//! - After verification / revocation the domain is removed from the database registry's cache at once;
//!   other servers see it after at most `AUTOXIDE_TENANCY_CACHE_SECONDS`.

mod dns;
mod http;

use crate::time::Timestamp;
use jiff::SignedDuration;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use utoipa::ToSchema;
use uuid::Uuid;

use super::errors::{
    self, tenant_domain_dns_failed, tenant_domain_invalid, tenant_domain_not_found,
    tenant_domain_revoked, tenant_domain_taken,
};
use super::{
    DatabaseRegistry, OrgId, OrgNodes, TenancyMode, TenantContext, TenantId, TenantStatus,
};
use crate::access::Permission;
use crate::database::Database;
use crate::error::{Error, ErrorKind, FieldError};
use crate::event::{Actor, Event, EventInfo, Outbox};
use crate::job::{JobContext, JobHandler, Schedule};
use crate::migration::prelude::{
    DbErr, MigrationName, MigrationTrait, SchemaManager, Table, schema,
};
use crate::pagination::{Page, PageRequest, fetch_page};

pub(crate) use http::{routes, tools};

/// Manages the caller's own tenant's domains.
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
pub(crate) const DOMAIN_FAILED: EventInfo = EventInfo::new(
    "kernel.domain_failed",
    "A domain of the tenant became `failed`: not verified within 7 days, or its DNS record was \
     missing at 3 daily checks in a row; requests to it no longer reach the tenant.",
);

pub(crate) const DOMAIN_UPDATED: EventInfo = EventInfo::new(
    "kernel.domain_updated",
    "A domain of the tenant was attached to another node of the tenant's organization tree, or \
     detached from it.",
);

/// Job kind (also the schedule name) that rechecks pending domains.
pub(crate) const VERIFY_DOMAINS: &str = "kernel.verify_domains";

/// Name of the TXT record placed before the domain.
pub const CHALLENGE_PREFIX: &str = "_autoxide-challenge";
/// Value of the TXT record placed before the token.
pub const CHALLENGE_VALUE_PREFIX: &str = "autoxide-verify=";

/// A `pending` domain this old becomes `failed`.
const PENDING_DAYS: i64 = 7;
/// Waits at most this long for DNS to answer (neither the job nor a request hangs longer).
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);
/// Number of domains the job checks per run (the next run checks the rest).
const SWEEP_BATCH: u64 = 200;
/// Maximum length of `check_error` (the `varchar(200)` column).
const CHECK_ERROR_MAX: usize = 200;
/// A `verified` domain is rechecked this many hours after its previous check.
const RECHECK_HOURS: i64 = 24;
/// Number of consecutive rechecks that do not find the record before a `verified` domain becomes `failed`.
const MAX_CHECK_FAILURES: i32 = 3;

/// Looks up the TXT records of a name. The kernel uses the operating system's DNS resolver; a module (or
/// testkit) publishes `dyn DnsTxtLookup` to replace it (`context.provide::<dyn DnsTxtLookup>(…)`).
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait DnsTxtLookup: Send + Sync {
    /// Every TXT value of `name` (the full name, e.g. `_autoxide-challenge.shop.example`); empty when
    /// the name has no TXT record or does not exist. An error when DNS cannot be queried.
    async fn txt(&self, name: &str) -> Result<Vec<String>, Error>;
}

/// The DNS resolver used for domains: the one a module / testkit publishes, or the operating system's. Filled
/// once after every module has started (`Modules::start_in`). Cheap to clone.
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

/// The organization tree a module publishes (step 34), or none. Filled once after every module has
/// started (`Modules::start_in`). Cheap to clone.
#[derive(Clone, Default)]
pub(crate) struct OrgSlot(Arc<OnceLock<Arc<dyn OrgNodes>>>);

impl OrgSlot {
    pub(crate) fn fill(&self, nodes: Option<Arc<dyn OrgNodes>>) {
        if let Some(nodes) = nodes {
            let _ = self.0.set(nodes);
        }
    }

    fn get(&self) -> Option<Arc<dyn OrgNodes>> {
        self.0.get().cloned()
    }
}

/// The normalized domain (lowercase, no port, no trailing dot), or `None` when it is not
/// a usable domain: only ASCII labels of letters / digits / `-` (international names in punycode form
/// `xn--…`), each label 1 to 63 characters, the whole name at most 253, at least two labels; no IP, no
/// `localhost` or `*.localhost`.
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

/// Host name of `AUTOXIDE_HTTP_PUBLIC_URL` (`https://API.example.com:8443/x` → `api.example.com`).
pub(crate) fn host_of_url(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    normalize_domain(host)
}

pub(super) mod entity {
    //! The `kernel_tenant_domains` table (control plane).
    use crate::time::Timestamp;
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
        pub verified_at: Option<Timestamp>,
        pub last_checked_at: Option<Timestamp>,
        pub check_error: Option<String>,
        /// Number of consecutive rechecks that did not find the record (`verified` domain).
        pub check_failures: i32,
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
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
    /// days, or a verified domain whose record was missing at 3 daily checks in a row: fix the
    /// record and verify again).
    #[schema(example = "pending")]
    pub status: String,
    /// The DNS record to set at your DNS provider, then call `POST /domains/{id}/verify`.
    pub record: DomainRecord,
    /// Node of the tenant's organization tree the domain belongs to (e.g. one store's own
    /// site); `null`: the tenant as a whole. Set it with `PATCH /domains/{id}`.
    pub org_id: Option<Uuid>,
    pub verified_at: Option<Timestamp>,
    /// Last time the DNS record was looked up.
    pub last_checked_at: Option<Timestamp>,
    /// Why the last lookup did not verify the domain.
    pub check_error: Option<String>,
    /// Daily checks in a row of the verified domain that did not find the DNS record; at 3 the
    /// domain becomes `failed`. Keep the record in place: one successful check resets it to 0.
    pub failed_checks: u32,
    pub created_at: Timestamp,
    /// When the status last changed; a `pending` domain fails 7 days after it.
    pub updated_at: Timestamp,
}

/// Payload of `kernel.domain_added`, `kernel.domain_verified`, `kernel.domain_revoked` and
/// `kernel.domain_failed`.
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

#[derive(Debug, Serialize)]
#[serde(transparent)]
struct DomainFailed(DomainChanged);

impl Event for DomainFailed {
    const NAME: &'static str = "kernel.domain_failed";
}

/// Payload of `kernel.domain_updated`.
#[derive(Debug, Serialize)]
struct DomainUpdated {
    id: Uuid,
    domain: String,
    status: String,
    /// Node the domain belongs to now; `null`: the tenant as a whole.
    org_id: Option<Uuid>,
    /// Node it belonged to before.
    previous_org_id: Option<Uuid>,
}

impl Event for DomainUpdated {
    const NAME: &'static str = "kernel.domain_updated";
}

fn now() -> Timestamp {
    Timestamp::now()
}

fn describe(row: Model) -> TenantDomain {
    TenantDomain {
        record: DomainRecord {
            record_type: "TXT".to_owned(),
            name: format!("{CHALLENGE_PREFIX}.{}", row.domain),
            value: format!("{CHALLENGE_VALUE_PREFIX}{}", row.verification_token),
        },
        id: row.id,
        org_id: row.org_id,
        domain: row.domain,
        status: row.status,
        verified_at: row.verified_at,
        last_checked_at: row.last_checked_at,
        check_error: row.check_error,
        failed_checks: u32::try_from(row.check_failures).unwrap_or(0),
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// Result of one TXT lookup.
enum Probe {
    Found,
    /// DNS answered but without the right record: the reason (English, for the administrator).
    Missing(String),
    /// DNS could not be queried (error, timeout): unknown whether the record still exists.
    Unreachable(String),
}

/// The service for tenants' own domains. Published by the `kernel` module
/// (`context.require::<TenantDomains>()`).
pub struct TenantDomains {
    registry: DatabaseRegistry,
    outbox: Outbox,
    dns: DnsSlot,
    orgs: OrgSlot,
    /// Host name of `AUTOXIDE_HTTP_PUBLIC_URL`: a tenant may not claim it.
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
        orgs: OrgSlot,
        platform_host: Option<String>,
    ) -> Self {
        TenantDomains { registry, outbox, dns, orgs, platform_host }
    }

    /// The installation runs in multi-tenant mode (the only case where own domains take effect).
    pub(crate) fn multi_tenant(&self) -> bool {
        self.registry.mode() == TenancyMode::Multi
    }

    fn control(&self) -> &Database {
        self.registry.control()
    }

    /// The domain the tenant submitted, normalized; if not usable: `tenant_domain_invalid`.
    fn checked(&self, raw: &str) -> Result<String, Error> {
        let domain = normalize_domain(raw).ok_or_else(|| tenant_domain_invalid(raw))?;
        if self.platform_host.as_deref() == Some(domain.as_str()) {
            return Err(tenant_domain_invalid(raw));
        }
        Ok(domain)
    }

    /// One page of the domains of tenant `tenant` (excluding revoked domains), newest first unless
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

    /// Host names of tenant `tenant`'s verified domains (e.g. `shop.example`), at most 1,000 - for checking
    /// addresses a buyer's browser is sent back to (`return_url` of a payment). Always empty in
    /// single-tenant mode, where a tenant cannot add domains.
    pub async fn verified_hosts(&self, tenant: &TenantContext) -> Result<Vec<String>, Error> {
        let rows = Entity::find()
            .filter(Column::TenantId.eq(tenant.id().as_uuid()))
            .filter(Column::Status.eq(VERIFIED))
            .order_by_asc(Column::Id)
            .limit(1000)
            .all(self.control().read())
            .await?;
        Ok(rows.into_iter().map(|row| row.domain).collect())
    }

    /// Adds domain `raw` for tenant `tenant` (status `pending`, new verification token). Resubmitting
    /// a domain the tenant already has (`pending` / `verified`) returns it unchanged.
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
                    check_failures: Set(0),
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
                    .col_expr(Column::VerifiedAt, Expr::value(Option::<Timestamp>::None))
                    .col_expr(Column::LastCheckedAt, Expr::value(Option::<Timestamp>::None))
                    .col_expr(Column::CheckError, Expr::value(Option::<String>::None))
                    .col_expr(Column::CheckFailures, Expr::value(0))
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

    /// Looks up the TXT record of domain `id` of tenant `tenant` right now and returns the new status. A
    /// `failed` domain can be verified again; an already verified domain is returned as it was, unless the
    /// most recent daily recheck did not find the record: then it is rechecked at once (after fixing the
    /// record, the count restarts from 0).
    pub async fn verify(
        &self,
        actor: Actor,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<TenantDomain, Error> {
        let row = self.owned(tenant, id).await?;
        if row.status == VERIFIED {
            if row.check_failures == 0 {
                return Ok(describe(row));
            }
            let database = self.control().clone();
            return self.recheck(&database, row).await.map(describe);
        }
        let row = self.check(actor, row).await?;
        Ok(describe(row))
    }

    /// Revokes domain `id` of tenant `tenant` (`revoked`): requests to that domain no longer reach
    /// the tenant. Revoking an already revoked domain changes nothing.
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

    /// Attaches domain `id` of tenant `tenant` to node `org` of the tenant's organization tree (`None`:
    /// detach, the domain belongs to the whole tenant). The node must belong to the tenant, asked of
    /// [`OrgNodes`]; if no module publishes an organization tree, only `None` is accepted. A revoked
    /// domain: `tenant_domain_revoked`. Attaching again to the node it is already attached to changes
    /// nothing and writes no event. Does not change the status or `updated_at` (the 7-day deadline of a
    /// `pending` domain counts from it). Two concurrent attach commands: the later write wins.
    pub async fn assign_org(
        &self,
        actor: Actor,
        tenant: &TenantContext,
        id: Uuid,
        org: Option<OrgId>,
    ) -> Result<TenantDomain, Error> {
        let row = self.find_owned(tenant, id).await?;
        if row.status == REVOKED {
            return Err(tenant_domain_revoked());
        }
        let org_id = org.map(|org| org.as_uuid());
        if row.org_id == org_id {
            return Ok(describe(row));
        }
        if let Some(org) = org {
            // Hỏi cây tổ chức NGOÀI mọi lệnh ghi: module khác có thể đọc database của nó.
            let problem = match self.orgs.get() {
                Some(nodes) if nodes.has_node(tenant, org).await? => None,
                Some(_) => Some(FieldError::new(
                    "org_id",
                    "not_found",
                    "is not a node of this tenant's organization tree; send `null` to attach the \
                     domain to the tenant as a whole",
                )),
                None => Some(FieldError::new(
                    "org_id",
                    "unsupported",
                    "cannot name a node: no installed module keeps an organization tree; send \
                     `null`",
                )),
            };
            if let Some(problem) = problem {
                return Err(Error::validation(vec![problem]));
            }
        }
        let updated = Entity::update_many()
            .col_expr(Column::OrgId, Expr::value(org_id))
            .filter(Column::Id.eq(row.id))
            .filter(Column::TenantId.eq(tenant.id().as_uuid()))
            .filter(Column::Status.ne(REVOKED))
            .exec(self.control().write())
            .await?;
        if updated.rows_affected == 0 {
            // Vừa bị thu hồi (hay nhận lại bởi khách khác) giữa lúc đọc và lúc ghi.
            return Err(tenant_domain_revoked());
        }
        // Cache của sổ database giữ tên miền → (khách, nút) (`domain_site`): quên dòng này để website
        // thấy nút mới ngay ở máy này (máy khác thấy khi cache hết hạn).
        self.registry.invalidate_domain(&row.domain);
        let after = self.find(row.id).await?;
        let event = DomainUpdated {
            id: after.id,
            domain: after.domain.clone(),
            status: after.status.clone(),
            org_id: after.org_id,
            previous_org_id: row.org_id,
        };
        self.record(tenant, actor, &after, &event).await?;
        tracing::info!(
            tenant_id = %tenant.id(),
            domain = %after.domain,
            org_id = ?after.org_id,
            "tenant domain attached to an organization node"
        );
        Ok(describe(after))
    }

    /// Whether domain `raw` is verified and its tenant is active: for Caddy's `ask` query
    /// (on-demand TLS certificate issuance). Goes through the database registry's cache.
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

    /// Rechecks the `pending` domains on `database` (job `kernel.verify_domains`): the right record
    /// → `verified`; waiting more than 7 days → `failed`. Returns the number of domains examined.
    pub async fn check_pending(&self, database: &Database) -> Result<usize, Error> {
        let rows = Entity::find()
            .filter(Column::Status.eq(PENDING))
            .order_by_asc(Column::LastCheckedAt)
            .order_by_asc(Column::Id)
            .limit(SWEEP_BATCH)
            .all(database.read())
            .await?;
        let deadline = Timestamp::now() - SignedDuration::from_hours(24 * PENDING_DAYS);
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

    /// `pending` past its deadline → `failed`.
    async fn expire(&self, database: &Database, row: Model) -> Result<(), Error> {
        let now = now();
        let reason = format!(
            "Not verified within {PENDING_DAYS} days. Set the TXT record, then verify again."
        );
        let updated = Entity::update_many()
            .col_expr(Column::Status, Expr::value(FAILED))
            .col_expr(Column::CheckError, Expr::value(reason.clone()))
            .col_expr(Column::UpdatedAt, Expr::value(now))
            .filter(Column::Id.eq(row.id))
            .filter(Column::Status.eq(PENDING))
            .exec(database.write())
            .await?;
        if updated.rows_affected > 0 {
            let after = Model {
                status: FAILED.to_owned(),
                check_error: Some(reason),
                updated_at: now,
                ..row
            };
            self.record_failed(&after).await?;
            tracing::info!(domain = %after.domain, "tenant domain not verified in time; marked failed");
        }
        Ok(())
    }

    /// Rechecks the `verified` domains on `database` more than [`RECHECK_HOURS`] hours past their previous
    /// check (job `kernel.verify_domains`): record found → count restarts from 0; not found for the
    /// [`MAX_CHECK_FAILURES`]-th consecutive time → `failed`. Returns the number of domains examined.
    pub async fn recheck_verified(&self, database: &Database) -> Result<usize, Error> {
        let due = Timestamp::now() - SignedDuration::from_hours(RECHECK_HOURS);
        let rows = Entity::find()
            .filter(Column::Status.eq(VERIFIED))
            .filter(
                sea_orm::Condition::any()
                    .add(Column::LastCheckedAt.is_null())
                    .add(Column::LastCheckedAt.lt(due)),
            )
            .order_by_asc(Column::LastCheckedAt)
            .order_by_asc(Column::Id)
            .limit(SWEEP_BATCH)
            .all(database.read())
            .await?;
        let count = rows.len();
        for row in rows {
            self.recheck(database, row).await?;
        }
        Ok(count)
    }

    /// Looks up DNS for the `verified` domain `row` then writes the result; returns the row after writing.
    /// Every write only writes while the failure count has not been changed by another party: two
    /// concurrent checks count only once and only one writes the event.
    async fn recheck(&self, database: &Database, row: Model) -> Result<Model, Error> {
        // Tra DNS NGOÀI mọi transaction: gọi mạng có thể chậm.
        let probe = self.probe(&row.domain, &row.verification_token).await;
        let now = now();
        let unchanged = |update: sea_orm::UpdateMany<Entity>| {
            update
                .filter(Column::Id.eq(row.id))
                .filter(Column::Status.eq(VERIFIED))
                .filter(Column::CheckFailures.eq(row.check_failures))
        };
        match probe {
            Probe::Found => {
                unchanged(Entity::update_many())
                    .col_expr(Column::CheckFailures, Expr::value(0))
                    .col_expr(Column::LastCheckedAt, Expr::value(now))
                    .col_expr(Column::CheckError, Expr::value(Option::<String>::None))
                    .exec(database.write())
                    .await?;
            }
            Probe::Unreachable(reason) => {
                // Không biết bản ghi còn hay không: không tính là trượt, kiểm lại sau một ngày.
                let reason: String = reason.chars().take(CHECK_ERROR_MAX).collect();
                unchanged(Entity::update_many())
                    .col_expr(Column::LastCheckedAt, Expr::value(now))
                    .col_expr(Column::CheckError, Expr::value(reason))
                    .exec(database.write())
                    .await?;
            }
            Probe::Missing(reason) => {
                let failures = row.check_failures.saturating_add(1);
                let reason: String = reason.chars().take(CHECK_ERROR_MAX).collect();
                let mut update = unchanged(Entity::update_many())
                    .col_expr(Column::CheckFailures, Expr::value(failures))
                    .col_expr(Column::LastCheckedAt, Expr::value(now))
                    .col_expr(Column::CheckError, Expr::value(reason));
                let failed = failures >= MAX_CHECK_FAILURES;
                if failed {
                    update = update
                        .col_expr(Column::Status, Expr::value(FAILED))
                        .col_expr(Column::UpdatedAt, Expr::value(now));
                }
                let updated = update.exec(database.write()).await?;
                if failed && updated.rows_affected > 0 {
                    self.registry.invalidate_domain(&row.domain);
                    let after = self.find_in(database, row.id).await?;
                    self.record_failed(&after).await?;
                    tracing::warn!(
                        tenant_id = %after.tenant_id,
                        domain = %after.domain,
                        failures,
                        "tenant domain lost its DNS record; marked failed"
                    );
                    return Ok(after);
                }
            }
        }
        self.find_in(database, row.id).await
    }

    /// Writes `kernel.domain_failed` for a domain that has just become `failed` (skipped if the tenant no
    /// longer exists).
    async fn record_failed(&self, row: &Model) -> Result<(), Error> {
        match self.registry.tenant(TenantId::from_uuid(row.tenant_id)).await {
            Ok(tenant) => {
                let event = DomainFailed(changed(row));
                self.record(&tenant, Actor::system(), row, &event).await
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Looks up DNS for `row` (on the control database) and writes the result.
    async fn check(&self, actor: Actor, row: Model) -> Result<Model, Error> {
        let database = self.control().clone();
        self.check_on(&database, actor, row).await
    }

    /// Looks up DNS for `row` (a row of `database`) then writes the result; returns the row after writing.
    /// Only one party (a request or a job running at the same time) can move `pending` / `failed` →
    /// `verified` and write the event.
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
                    .col_expr(Column::CheckFailures, Expr::value(0))
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
            Probe::Missing(reason) | Probe::Unreachable(reason) => {
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

    /// Looks up the TXT record of `domain`, compared with `token`.
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
                Probe::Unreachable(format!("DNS lookup failed: {}", error.message()))
            }
            Err(_) => Probe::Unreachable(format!(
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

    /// Row `id` of tenant `tenant` (including revoked ones); a row of another tenant counts as "not found".
    async fn find_owned(&self, tenant: &TenantContext, id: Uuid) -> Result<Model, Error> {
        Entity::find_by_id(id)
            .filter(Column::TenantId.eq(tenant.id().as_uuid()))
            .one(self.control().read())
            .await?
            .ok_or_else(tenant_domain_not_found)
    }

    /// Like [`find_owned`](Self::find_owned), not counting revoked domains.
    async fn owned(&self, tenant: &TenantContext, id: Uuid) -> Result<Model, Error> {
        let row = self.find_owned(tenant, id).await?;
        if row.status == REVOKED { Err(tenant_domain_not_found()) } else { Ok(row) }
    }

    /// Writes `event` of domain `row` to the `kernel` outbox in the tenant's database.
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

/// Schedule of the job `kernel.verify_domains`: every 10 minutes, only on the control database (the only place
/// that has domains).
pub(crate) fn schedule() -> Schedule {
    Schedule::new(VERIFY_DOMAINS, VERIFY_DOMAINS, Duration::from_secs(600)).on_control_only()
}

/// Runner of the job `kernel.verify_domains` (a job of the whole control database).
pub(crate) struct VerifyDomains(pub(crate) Arc<TenantDomains>);

#[async_trait::async_trait]
impl JobHandler for VerifyDomains {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let checked = self.0.check_pending(&job.database).await?;
        if checked > 0 {
            tracing::info!(checked, "pending tenant domains checked");
        }
        let rechecked = self.0.recheck_verified(&job.database).await?;
        if rechecked > 0 {
            tracing::info!(rechecked, "verified tenant domains checked again");
        }
        Ok(())
    }
}

/// The `check_failures` column of `kernel_tenant_domains`: the number of consecutive daily rechecks that
/// did not find the TXT record of a verified domain.
pub(crate) struct AddCheckFailures;

impl MigrationName for AddCheckFailures {
    fn name(&self) -> &str {
        "m0012_domain_check_failures"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for AddCheckFailures {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("kernel_tenant_domains")
                    .add_column(schema::integer("check_failures").default(0))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("kernel_tenant_domains")
                    .drop_column("check_failures")
                    .to_owned(),
            )
            .await
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
