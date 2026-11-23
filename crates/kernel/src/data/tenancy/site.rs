//! The website of a public request (step 34, D-208): each node of the organization tree can have its own
//! website (a store, a seller on a marketplace). A request to a verified domain attached to node N → N's
//! website; not attached to a node → the whole tenant's website (the root node). A client that does not
//! come through its own domain (e.g. a prebuilt UI, another server) names the website with the parameter
//! `?site=<node id>`.
//!
//! A public-read module takes [`Site`] as an extractor then filters by [`Site::lineage`]: the data of the
//! node itself and of every ancestor node shows on that website (a headquarters post shows in every store;
//! a post of store A does not show in store B).

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use uuid::Uuid;

use super::{OrgId, OrgTree, TenantContext, TenantHint, root_of};
use crate::error::Error;

/// Name of the query parameter that names the website.
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

/// The request's website: a node of the organization tree (root = the whole tenant).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Site {
    org: OrgId,
    /// The node and all its ancestors, nearest first (including the root).
    lineage: Vec<OrgId>,
}

impl Site {
    /// The website of the whole tenant `tenant`.
    pub fn whole(tenant: &TenantContext) -> Self {
        let root = root_of(tenant.id());
        Site { org: root, lineage: vec![root] }
    }

    /// The website of node `org` of tenant `tenant`; a node not in the tree → `site_not_found`.
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

    /// The website's node.
    pub fn org(&self) -> OrgId {
        self.org
    }

    /// The website of the whole tenant (the root node).
    pub fn is_whole(&self) -> bool {
        self.lineage.len() <= 1
    }

    /// The node and all its ancestors, nearest first: the data of these nodes shows on the website.
    pub fn lineage(&self) -> &[OrgId] {
        &self.lineage
    }

    /// The UUIDs of [`lineage`](Self::lineage), for the condition `org_id IN (…)`.
    pub fn lineage_ids(&self) -> Vec<Uuid> {
        self.lineage.iter().map(OrgId::as_uuid).collect()
    }
}

/// `site_not_found` (404): the tenant has no node `org` (modules return the same code when the node is
/// unusable).
pub fn site_not_found(org: OrgId) -> Error {
    Error::not_found(
        "site_not_found",
        "There is no website (organization unit) `{site}` in this tenant. Leave `site` out for the \
         website of the whole tenant.",
    )
    .with_param("site", org.as_uuid())
}

/// `site` in the query string, if present. A value that is not a UUID → 400.
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
