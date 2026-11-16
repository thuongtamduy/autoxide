//! Website của một request công khai (bước 34, D-208): mỗi nút của cây tổ chức có thể có website
//! riêng (cửa hàng, người bán trên sàn). Request tới tên miền đã xác minh gắn với nút N → website
//! của N; không gắn nút → website của cả khách (nút gốc). Client không gọi qua tên miền riêng (vd
//! giao diện dựng sẵn, máy chủ khác) nêu website bằng tham số `?site=<id của nút>`.
//!
//! Module đọc công khai lấy [`Site`] làm extractor rồi lọc theo [`Site::lineage`]: dữ liệu của
//! chính nút và của mọi nút cấp trên hiện trên website đó (bài của trụ sở hiện ở mọi cửa hàng; bài
//! của cửa hàng A không hiện ở cửa hàng B).

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use uuid::Uuid;

use super::{OrgId, OrgTree, TenantContext, TenantHint, root_of};
use crate::error::Error;

/// Tên tham số query nêu website.
pub const SITE_PARAM: &str = "site";

// Chỉ để mô tả tham số trong OpenAPI (`params(…, SiteQuery)`): giá trị do extractor `Site` đọc.
/// The website asking.
#[derive(Debug, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SiteQuery {
    /// The website (organization unit) asking, when it is not called on its own domain; a domain
    /// bound to a unit chooses it already. Content of that unit and of the units above it is
    /// shown. Omitted: the website of the whole tenant.
    pub site: Option<Uuid>,
}

/// Website của request: nút của cây tổ chức (gốc = cả khách).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Site {
    org: OrgId,
    /// Nút và mọi nút cấp trên, gần nhất trước (gồm gốc).
    lineage: Vec<OrgId>,
}

impl Site {
    /// Website của cả khách `tenant`.
    pub fn whole(tenant: &TenantContext) -> Self {
        let root = root_of(tenant.id());
        Site { org: root, lineage: vec![root] }
    }

    /// Website của nút `org` của khách `tenant`; nút không có trong cây → `site_not_found`.
    pub async fn of(tenant: &TenantContext, org: OrgId) -> Result<Self, Error> {
        let root = root_of(tenant.id());
        if org == root {
            return Ok(Site::whole(tenant));
        }
        let lineage = OrgTree::ancestors(tenant.database().read(), tenant.id(), org).await?;
        if lineage.is_empty() {
            return Err(site_not_found(org));
        }
        Ok(Site { org, lineage })
    }

    /// Nút của website.
    pub fn org(&self) -> OrgId {
        self.org
    }

    /// Website của cả khách (nút gốc).
    pub fn is_whole(&self) -> bool {
        self.lineage.len() <= 1
    }

    /// Nút và mọi nút cấp trên, gần nhất trước: dữ liệu của các nút này hiện trên website.
    pub fn lineage(&self) -> &[OrgId] {
        &self.lineage
    }

    /// UUID của [`lineage`](Self::lineage), cho điều kiện `org_id IN (…)`.
    pub fn lineage_ids(&self) -> Vec<Uuid> {
        self.lineage.iter().map(OrgId::as_uuid).collect()
    }
}

fn site_not_found(org: OrgId) -> Error {
    Error::not_found(
        "site_not_found",
        "There is no website (organization unit) `{site}` in this tenant. Leave `site` out for the \
         website of the whole tenant.",
    )
    .with_param("site", org.as_uuid())
}

/// `site` trong query string, nếu có. Giá trị không phải UUID → 400.
fn site_param(parts: &Parts) -> Result<Option<OrgId>, Error> {
    let Some(query) = parts.uri.query() else { return Ok(None) };
    for pair in query.split('&') {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        if name == SITE_PARAM {
            return value.parse::<Uuid>().map(|id| Some(OrgId::from_uuid(id))).map_err(|_| {
                Error::bad_request(
                    "invalid_site",
                    "`site` must be the id (UUID) of an organization unit.",
                )
            });
        }
    }
    Ok(None)
}

// Handler viết `site: Site`: giải khách của request, rồi nút của website (tham số `site` thắng tên
// miền). Kết quả giữ trong extensions để handler gọi lại không tra lần hai.
impl<S: Send + Sync> FromRequestParts<S> for Site {
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        if let Some(site) = parts.extensions.get::<Site>() {
            return Ok(site.clone());
        }
        let tenant =
            <TenantContext as FromRequestParts<S>>::from_request_parts(parts, state).await?;
        let site = match site_param(parts)? {
            Some(org) => Site::of(&tenant, org).await?,
            None => {
                let hint = TenantHint::of_request(&parts.headers, &parts.extensions);
                let registry = parts.extensions.get::<super::DatabaseRegistry>().cloned();
                let org = match (registry, hint.host()) {
                    (Some(registry), Some(host)) => match registry.domain_site(host).await? {
                        Some((owner, Some(org))) if owner == tenant.id() => Some(org),
                        _ => None,
                    },
                    _ => None,
                };
                match org {
                    Some(org) => Site::of(&tenant, org).await?,
                    None => Site::whole(&tenant),
                }
            }
        };
        parts.extensions.insert(site.clone());
        Ok(site)
    }
}
