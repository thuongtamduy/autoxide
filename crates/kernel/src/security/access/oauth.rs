//! OAuth 2.1 on the protected-resource side (step 71 G4, D-20261126-mcp-oauth): what a caller that arrived with an
//! OAuth access token carries ([`OAuthAccess`]), how a grant's scopes narrow its permissions, and the trait an
//! authorization-server module publishes so the kernel can serve the discovery documents ([`AuthorizationServer`]).
//!
//! A scope is a permission name (`notes.write`), a prefix ending in `.*` (`notes.*`: every permission whose name
//! starts with `notes.`), or `*` (every permission the user has).

use std::collections::BTreeMap;
use std::sync::Arc;

use uuid::Uuid;

use super::{Grants, PermissionCatalog, Principal};
use crate::error::Error;
use crate::tenancy::root_of;

/// Reason of `permission_denied`: the user has the permission, but the OAuth grant the client holds does not
/// include it. Asking the user to approve a wider grant (step-up authorization) fixes it.
pub(super) const OUTSIDE_GRANT_SCOPE: &str = "outside_grant_scope";

/// Metadata of the authorization server, published by the module that issues OAuth access tokens (usually
/// `auth`) with `context.provide::<dyn AuthorizationServer>(…)`. The kernel serves it at
/// `/.well-known/oauth-authorization-server` (RFC 8414) and lists the issuer in the protected resource metadata
/// (RFC 9728). When no module publishes it, those documents do not exist.
pub trait AuthorizationServer: Send + Sync {
    /// The RFC 8414 metadata document for `issuer` (an origin such as `https://api.example.com`, no trailing
    /// slash). The kernel sets the `issuer` member itself, so it always equals the address the document is
    /// served from.
    fn metadata(&self, issuer: &str) -> serde_json::Map<String, serde_json::Value>;
}

/// What a caller holding an OAuth access token carries besides its permissions: the grant the token belongs to,
/// the client that holds it and the resource (audience) it was issued for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthAccess {
    grant_id: Uuid,
    client_id: String,
    audience: String,
}

impl OAuthAccess {
    /// A token of grant `grant_id`, held by client `client_id`, issued for resource `audience` (an absolute URL
    /// such as `https://api.example.com/mcp`).
    pub fn new(grant_id: Uuid, client_id: impl Into<String>, audience: impl Into<String>) -> Self {
        OAuthAccess { grant_id, client_id: client_id.into(), audience: audience.into() }
    }

    /// ID of the grant (the user's approval) the token belongs to.
    pub fn grant_id(&self) -> Uuid {
        self.grant_id
    }

    /// The OAuth client: a URL for clients identified by a metadata document, otherwise the registered ID.
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// The resource the token was issued for.
    pub fn audience(&self) -> &str {
        &self.audience
    }

    /// Whether the token may be used at `resource`: the audience is the resource itself or a resource above it
    /// (`https://a.example/mcp` covers `https://a.example/mcp/notes`; a broader resource implies the narrower
    /// ones, never the other way round).
    pub fn covers(&self, resource: &str) -> bool {
        resource == self.audience
            || resource
                .strip_prefix(self.audience.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
    }
}

/// Whether the scope list `scopes` (space-separated tokens, as in OAuth) covers the permission `name`.
pub fn scope_allows<'a>(scopes: impl IntoIterator<Item = &'a str>, name: &str) -> bool {
    scopes.into_iter().any(|scope| {
        scope == "*"
            || scope == name
            || scope.strip_suffix(".*").is_some_and(|prefix| {
                name.strip_prefix(prefix).is_some_and(|rest| rest.starts_with('.'))
            })
    })
}

impl Principal {
    /// Marks the caller as an OAuth client acting for the user (see [`OAuthAccess`]).
    #[must_use]
    pub fn with_oauth(mut self, access: OAuthAccess) -> Self {
        self.oauth = Some(Arc::new(access));
        self
    }

    /// The OAuth grant, when the request arrived with an OAuth access token.
    pub fn oauth(&self) -> Option<&OAuthAccess> {
        self.oauth.as_deref()
    }

    /// Keeps only the permissions the grant's `scopes` cover. Permissions the user has but the scopes leave out
    /// are remembered: refusing one says `outside_grant_scope` and carries an `insufficient_scope` challenge, so
    /// the client can ask the user to approve more (permissions the user lacks keep the reason `missing` —
    /// re-approving would not help). `catalog` expands "every permission" (role `admin`) into names.
    #[must_use]
    pub fn within_scope(mut self, scopes: &[String], catalog: &PermissionCatalog) -> Self {
        let allows = |name: &str| scope_allows(scopes.iter().map(String::as_str), name);
        let root = root_of(self.tenant);
        let held: BTreeMap<String, Vec<super::Grant>> = match &self.grants {
            Grants::All => catalog
                .iter()
                .map(|permission| {
                    (
                        permission.name.to_owned(),
                        vec![super::Grant::new(root, super::Scope::Tenant)],
                    )
                })
                .collect(),
            Grants::Only(map) => (**map).clone(),
        };
        let (kept, withheld): (BTreeMap<_, _>, BTreeMap<_, _>) =
            held.into_iter().partition(|(name, _)| allows(name));
        if withheld.is_empty() && matches!(self.grants, Grants::Only(_)) {
            return self;
        }
        self.grants = Grants::Only(Arc::new(kept));
        let mut denied = (*self.denied).clone();
        for name in withheld.into_keys() {
            denied.entry(name).or_insert(OUTSIDE_GRANT_SCOPE);
        }
        self.denied = Arc::new(denied);
        self
    }
}

/// Adds the RFC 6750 `insufficient_scope` challenge to a `permission_denied` error when the refused permission
/// is outside the grant's scope (`reason`); the kernel's HTTP layer adds `resource_metadata` to it.
pub(super) fn challenge_scope(error: Error, reason: &str, permission: &str) -> Error {
    if reason != OUTSIDE_GRANT_SCOPE {
        return error;
    }
    error.with_challenge(format!(
        "Bearer error=\"insufficient_scope\", scope=\"{permission}\", error_description=\"The \
         authorization does not include this permission\""
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::Permission;

    const READ: Permission = Permission::new("notes.read", "Read notes.");
    const WRITE: Permission = Permission::new("notes.write", "Write notes.");
    const OTHER: Permission = Permission::new("media.read", "Read media.");

    fn scopes(list: &[&str]) -> Vec<String> {
        list.iter().map(|scope| (*scope).to_owned()).collect()
    }

    #[test]
    fn scope_khop_ten_tien_to_va_sao() {
        assert!(scope_allows(["notes.read"], "notes.read"));
        assert!(!scope_allows(["notes.read"], "notes.write"));
        assert!(scope_allows(["notes.*"], "notes.write"));
        // Tiền tố phải dừng ở dấu chấm: `note.*` không phủ `notes.read`.
        assert!(!scope_allows(["note.*"], "notes.read"));
        assert!(scope_allows(["kernel.tenants.*"], "kernel.tenants.operate"));
        assert!(scope_allows(["*"], "media.read"));
        assert!(!scope_allows(Vec::<&str>::new(), "media.read"));
    }

    #[test]
    fn audience_rong_phu_resource_hep_khong_nguoc_lai() {
        let mcp = OAuthAccess::new(Uuid::nil(), "c", "https://a.example/mcp");
        assert!(mcp.covers("https://a.example/mcp"));
        assert!(mcp.covers("https://a.example/mcp/notes"));
        assert!(!mcp.covers("https://a.example/mcpx"));
        assert!(!mcp.covers("https://a.example/api/v1"));
        let notes = OAuthAccess::new(Uuid::nil(), "c", "https://a.example/mcp/notes");
        assert!(!notes.covers("https://a.example/mcp"));
        let all = OAuthAccess::new(Uuid::nil(), "c", "https://a.example");
        assert!(all.covers("https://a.example/api/v1"));
        assert!(!all.covers("https://a.example.evil/api/v1"));
    }

    #[test]
    fn quyen_ngoai_scope_doi_cap_them_quyen_ngoai_tai_khoan_thi_khong() {
        let catalog =
            PermissionCatalog::new(vec![("notes", vec![READ, WRITE]), ("media", vec![OTHER])])
                .unwrap();
        let principal =
            Principal::new(Uuid::now_v7(), ["notes.read".to_owned(), "notes.write".to_owned()])
                .within_scope(&scopes(&["notes.read", "media.read"]), &catalog);
        assert!(principal.require(&READ).is_ok());
        let error = principal.require(&WRITE).unwrap_err();
        assert_eq!(error.reason(), Some(OUTSIDE_GRANT_SCOPE));
        assert!(error.challenge().is_some_and(|c| c.contains("scope=\"notes.write\"")));
        // Người dùng không có `media.read`: scope có cũng không cấp, và không đòi cấp lại.
        let error = principal.require(&OTHER).unwrap_err();
        assert_eq!(error.reason(), Some("missing"));
        assert!(error.challenge().is_none());
    }

    #[test]
    fn toan_quyen_bi_thu_hep_theo_scope() {
        let catalog =
            PermissionCatalog::new(vec![("notes", vec![READ, WRITE]), ("media", vec![OTHER])])
                .unwrap();
        let principal = Principal::with_all_permissions(Uuid::now_v7())
            .within_scope(&scopes(&["notes.*"]), &catalog);
        assert!(principal.require(&WRITE).is_ok());
        assert_eq!(principal.require(&OTHER).unwrap_err().reason(), Some(OUTSIDE_GRANT_SCOPE));
        let principal =
            Principal::with_all_permissions(Uuid::now_v7()).within_scope(&scopes(&["*"]), &catalog);
        assert!(principal.require(&OTHER).is_ok());
    }
}
