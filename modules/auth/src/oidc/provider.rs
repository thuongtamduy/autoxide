//! Nhà cung cấp đăng nhập của khách (bảng `auth_oidc_provider`, bước 40 P3): quản trị khách khai
//! Google, Microsoft, GitHub hay một issuer OIDC bất kỳ. `client_secret` mã hoá khi lưu bằng lớp
//! AEAD của kernel (bước 33, D-257) và không bao giờ trả lại.

use autoxide_kernel::time::Timestamp;
use std::collections::BTreeSet;

use autoxide_kernel::access::{Principal, Realm};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::secret::SecretKey;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, Set,
    SqlErr,
};
use serde_json::json;
use uuid::Uuid;

use super::client::{OidcClient, UrlProblem};
use crate::events;

pub(crate) mod entity {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_oidc_provider")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub slug: String,
        pub kind: String,
        pub name: String,
        /// `None` với GitHub (github.com); GitHub Enterprise Server: địa chỉ gốc của máy chủ.
        pub issuer: Option<String>,
        pub client_id: String,
        /// `sealed:v1:<key_id>:…` (mang số phiên bản khoá).
        pub client_secret: String,
        pub scopes: String,
        /// JSON: mảng địa chỉ quay về được phép.
        pub redirect_uris: String,
        /// JSON: mảng tên miền email được phép; `None`: mọi tên miền.
        pub allowed_email_domains: Option<String>,
        pub allow_signup: bool,
        /// `staff` hay `customer` (D-309): loại tài khoản đăng nhập bằng nhà cung cấp này.
        pub realm: String,
        pub default_org_id: Option<Uuid>,
        pub default_role_id: Option<Uuid>,
        pub enabled: bool,
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
        /// Người tạo hay sửa gần nhất (D-20261121-oidc-link-covers); `None`: dòng có từ trước.
        pub configured_by: Option<Uuid>,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}

pub(crate) type Provider = entity::Model;

/// Mục đích của lớp mã hoá cho `client_secret` (dữ liệu đi kèm: khách + ID nhà cung cấp).
const SECRET_PURPOSE: &str = "auth.oidc_client_secret";

pub(crate) const GOOGLE_ISSUER: &str = "https://accounts.google.com";
pub(crate) const MICROSOFT_ISSUER: &str = "https://login.microsoftonline.com/organizations/v2.0";
pub(crate) const DEFAULT_SCOPES: &str = "openid email profile";
const GITHUB_SCOPES: &str = "read:user user:email";
const MAX_REDIRECT_URIS: usize = 20;
const MAX_DOMAINS: usize = 50;

/// Loại nhà cung cấp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Google,
    Microsoft,
    Github,
    Oidc,
}

impl Kind {
    pub(crate) fn parse(raw: &str) -> Option<Kind> {
        match raw {
            "google" => Some(Kind::Google),
            "microsoft" => Some(Kind::Microsoft),
            "github" => Some(Kind::Github),
            "oidc" => Some(Kind::Oidc),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Kind::Google => "google",
            Kind::Microsoft => "microsoft",
            Kind::Github => "github",
            Kind::Oidc => "oidc",
        }
    }
}

impl Provider {
    pub(crate) fn kind(&self) -> Kind {
        Kind::parse(&self.kind).unwrap_or(Kind::Oidc)
    }

    pub(crate) fn realm(&self) -> Realm {
        crate::user::realm_of(&self.realm)
    }

    pub(crate) fn redirect_uris(&self) -> Vec<String> {
        serde_json::from_str(&self.redirect_uris).unwrap_or_default()
    }

    pub(crate) fn allowed_email_domains(&self) -> Option<Vec<String>> {
        self.allowed_email_domains.as_deref().and_then(|raw| serde_json::from_str(raw).ok())
    }

    pub(crate) fn has_secret(&self) -> bool {
        !self.client_secret.is_empty()
    }
}

/// Nhà cung cấp mới.
pub(crate) struct NewProvider {
    pub slug: String,
    pub kind: String,
    pub name: String,
    pub issuer: Option<String>,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub scopes: Option<String>,
    pub redirect_uris: Vec<String>,
    pub allowed_email_domains: Option<Vec<String>>,
    pub allow_signup: bool,
    pub realm: Realm,
    pub default_org_id: Option<Uuid>,
    pub default_role_id: Option<Uuid>,
    pub enabled: bool,
}

/// Thay đổi của nhà cung cấp; `None`: giữ nguyên. `Some(None)`: xoá giá trị.
#[derive(Default)]
pub(crate) struct ProviderChanges {
    pub name: Option<String>,
    pub issuer: Option<Option<String>>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub scopes: Option<String>,
    pub redirect_uris: Option<Vec<String>>,
    pub allowed_email_domains: Option<Option<Vec<String>>>,
    pub allow_signup: Option<bool>,
    pub default_org_id: Option<Option<Uuid>>,
    pub default_role_id: Option<Option<Uuid>>,
    pub enabled: Option<bool>,
}

pub(crate) struct ProviderService {
    pub secrets: SecretKey,
    pub outbox: Outbox,
}

fn now() -> Timestamp {
    Timestamp::now()
}

impl ProviderService {
    /// Mọi nhà cung cấp của khách, theo thứ tự tạo.
    pub(crate) async fn list(&self, tenant: &TenantContext) -> Result<Vec<Provider>, Error> {
        Ok(entity::Entity::find()
            .scoped(tenant)
            .order_by_asc(entity::Column::Id)
            .all(tenant.database().read())
            .await?)
    }

    /// Một trang nhà cung cấp của khách (`enabled_only`: chỉ cái đang bật), mới nhất trước trừ `order=asc`; keyset trên khoá chính (tenant_id, id) / index (tenant_id, enabled, id).
    pub(crate) async fn page(
        &self,
        tenant: &TenantContext,
        enabled_only: bool,
        page: &autoxide_kernel::pagination::PageRequest,
    ) -> Result<autoxide_kernel::pagination::Page<Provider>, Error> {
        let mut select = entity::Entity::find().scoped(tenant);
        if enabled_only {
            select = select.filter(entity::Column::Enabled.eq(true));
        }
        autoxide_kernel::pagination::fetch_page(
            select,
            entity::Column::Id,
            |row| row.id,
            page,
            tenant.database().read(),
        )
        .await
    }

    pub(crate) async fn get(&self, tenant: &TenantContext, id: Uuid) -> Result<Provider, Error> {
        entity::Entity::find_by_id((tenant.id().as_uuid(), id))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| not_found_id(id))
    }

    /// Nhà cung cấp đang bật mang `slug` của khách; không có (kể cả của khách khác) thì
    /// `auth.oidc_provider_not_found`.
    pub(crate) async fn enabled_by_slug(
        &self,
        tenant: &TenantContext,
        slug: &str,
    ) -> Result<Provider, Error> {
        entity::Entity::find()
            .filter(entity::Column::Slug.eq(slug))
            .filter(entity::Column::Enabled.eq(true))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| not_found_slug(slug))
    }

    /// `client_secret` đã giải mã.
    pub(crate) async fn secret(
        &self,
        tenant: &TenantContext,
        provider: &Provider,
    ) -> Result<String, Error> {
        if !provider.has_secret() {
            return Ok(String::new());
        }
        let sealer = self.secrets.sealer().await?;
        let opened = sealer.open(
            tenant.id(),
            SECRET_PURPOSE,
            &provider.id.to_string(),
            &provider.client_secret,
        )?;
        String::from_utf8(opened.to_vec()).map_err(Error::internal)
    }

    async fn seal(&self, tenant: &TenantContext, id: Uuid, secret: &str) -> Result<String, Error> {
        if secret.is_empty() {
            return Ok(String::new());
        }
        let sealer = self.secrets.sealer().await?;
        sealer.seal(tenant.id(), SECRET_PURPOSE, &id.to_string(), secret.as_bytes())
    }

    pub(crate) async fn create(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        client: &OidcClient,
        input: NewProvider,
    ) -> Result<Provider, Error> {
        // Nhà cung cấp tự liên kết tài khoản theo quyền của người cấu hình (D-20261121-oidc-link-covers),
        // không theo phạm vi key: API key không được thêm hay sửa nó (pentest 2026-11).
        crate::access::require_session(principal)?;
        let mut problems = Vec::new();
        let slug = input.slug.trim().to_ascii_lowercase();
        problems.extend(check_slug(&slug));
        let kind = Kind::parse(&input.kind);
        if kind.is_none() {
            problems.push(FieldError::new(
                "kind",
                "invalid",
                "must be `google`, `microsoft`, `github` or `oidc`",
            ));
        }
        let kind = kind.unwrap_or(Kind::Oidc);
        let id = Uuid::now_v7();
        let now = now();
        let mut row = Provider {
            tenant_id: tenant.id().as_uuid(),
            id,
            slug: slug.clone(),
            kind: kind.as_str().to_owned(),
            name: input.name.trim().to_owned(),
            issuer: input.issuer.map(|issuer| issuer.trim().trim_end_matches('/').to_owned()),
            client_id: input.client_id.trim().to_owned(),
            client_secret: String::new(),
            scopes: input
                .scopes
                .map_or_else(|| default_scopes(kind).to_owned(), |s| normalize_scopes(&s)),
            redirect_uris: json!(input.redirect_uris).to_string(),
            allowed_email_domains: input
                .allowed_email_domains
                .as_ref()
                .map(|list| json!(domains(list)).to_string()),
            allow_signup: input.allow_signup,
            realm: crate::user::entity::realm_str(input.realm).to_owned(),
            default_org_id: input.default_org_id,
            default_role_id: input.default_role_id,
            enabled: input.enabled,
            created_at: now,
            updated_at: now,
            configured_by: Some(principal.user_id()),
        };
        if kind == Kind::Google && row.issuer.is_none() {
            row.issuer = Some(GOOGLE_ISSUER.to_owned());
        }
        if kind == Kind::Microsoft && row.issuer.is_none() {
            row.issuer = Some(MICROSOFT_ISSUER.to_owned());
        }
        let secret = input.client_secret.unwrap_or_default();
        problems.extend(check(
            &row,
            client,
            &input.redirect_uris,
            input.allowed_email_domains.as_deref(),
        ));
        if kind != Kind::Oidc && secret.trim().is_empty() {
            problems.push(FieldError::new(
                "client_secret",
                "required",
                "is required for Google, Microsoft and GitHub",
            ));
        }
        problems.extend(self.check_defaults(tenant, principal, &row).await?);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        row.client_secret = self.seal(tenant, id, secret.trim()).await?;
        let transaction = tenant.database().begin_write().await?;
        let row =
            row.into_active_model().reset_all().insert(&transaction).await.map_err(|error| {
                match error.sql_err() {
                    Some(SqlErr::UniqueConstraintViolation(_)) => slug_taken(&slug),
                    _ => Error::from(error),
                }
            })?;
        let event = events::OidcProviderCreated { before: None, after: Some(describe(&row)) };
        self.outbox
            .record(&transaction, tenant, Actor::from(principal), subject(row.id), &event)
            .await?;
        transaction.commit().await?;
        tracing::info!(provider_id = %row.id, kind = row.kind.as_str(), "sign-in provider added");
        Ok(row)
    }

    pub(crate) async fn update(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        client: &OidcClient,
        id: Uuid,
        changes: ProviderChanges,
    ) -> Result<Provider, Error> {
        crate::access::require_session(principal)?;
        let before = self.get(tenant, id).await?;
        let mut row = before.clone();
        if let Some(name) = changes.name {
            row.name = name.trim().to_owned();
        }
        if let Some(issuer) = changes.issuer {
            row.issuer = issuer.map(|issuer| issuer.trim().trim_end_matches('/').to_owned());
            match row.kind() {
                Kind::Google if row.issuer.is_none() => row.issuer = Some(GOOGLE_ISSUER.to_owned()),
                Kind::Microsoft if row.issuer.is_none() => {
                    row.issuer = Some(MICROSOFT_ISSUER.to_owned());
                }
                _ => {}
            }
        }
        if let Some(client_id) = changes.client_id {
            row.client_id = client_id.trim().to_owned();
        }
        if let Some(scopes) = changes.scopes {
            row.scopes = normalize_scopes(&scopes);
        }
        let redirect_uris = changes.redirect_uris.unwrap_or_else(|| before.redirect_uris());
        row.redirect_uris = json!(redirect_uris).to_string();
        let allowed = match changes.allowed_email_domains {
            Some(list) => list,
            None => before.allowed_email_domains(),
        };
        row.allowed_email_domains = allowed.as_ref().map(|list| json!(domains(list)).to_string());
        if let Some(allow) = changes.allow_signup {
            row.allow_signup = allow;
        }
        if let Some(org) = changes.default_org_id {
            row.default_org_id = org;
        }
        if let Some(role) = changes.default_role_id {
            row.default_role_id = role;
        }
        if let Some(enabled) = changes.enabled {
            row.enabled = enabled;
        }
        let mut problems = check(&row, client, &redirect_uris, allowed.as_deref());
        // Mọi lần sửa, không chỉ khi đổi trường mặc định: người sửa nhà cung cấp còn role mặc định
        // (issuer, client, tự đăng ký, tên miền…) quyết định ai nhận role đó, nên phải tự trao được nó
        // (D-20261125-durable-grants).
        problems.extend(self.check_defaults(tenant, principal, &row).await?);
        let secret_changed = changes.client_secret.is_some();
        if let Some(secret) = changes.client_secret {
            if row.kind() != Kind::Oidc && secret.trim().is_empty() {
                problems.push(FieldError::new(
                    "client_secret",
                    "required",
                    "is required for Google, Microsoft and GitHub",
                ));
            }
            row.client_secret = self.seal(tenant, id, secret.trim()).await?;
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        row.updated_at = now();
        let transaction = tenant.database().begin_write().await?;
        // Chỉ các cột đổi được; khoá chính, `slug`, `kind`, `realm`, `created_at` giữ nguyên.
        let mut active = before.clone().into_active_model();
        active.name = Set(row.name.clone());
        active.issuer = Set(row.issuer.clone());
        active.client_id = Set(row.client_id.clone());
        active.client_secret = Set(row.client_secret.clone());
        active.scopes = Set(row.scopes.clone());
        active.redirect_uris = Set(row.redirect_uris.clone());
        active.allowed_email_domains = Set(row.allowed_email_domains.clone());
        active.allow_signup = Set(row.allow_signup);
        active.default_org_id = Set(row.default_org_id);
        active.default_role_id = Set(row.default_role_id);
        active.enabled = Set(row.enabled);
        active.updated_at = Set(row.updated_at);
        active.configured_by = Set(Some(principal.user_id()));
        let row = active.update(&transaction).await?;
        let mut after = describe(&row);
        after["secret_changed"] = json!(secret_changed);
        let event =
            events::OidcProviderUpdated { before: Some(describe(&before)), after: Some(after) };
        self.outbox
            .record(&transaction, tenant, Actor::from(principal), subject(id), &event)
            .await?;
        transaction.commit().await?;
        tracing::info!(provider_id = %id, "sign-in provider changed");
        Ok(row)
    }

    /// Xoá nhà cung cấp `id` cùng mọi liên kết tài khoản tới nó (người chỉ đăng nhập bằng nó phải
    /// đặt lại mật khẩu bằng email).
    pub(crate) async fn delete(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        let before = self.get(tenant, id).await?;
        let transaction = tenant.database().begin_write().await?;
        super::identity::delete_for_provider(&transaction, tenant, id).await?;
        let deleted = entity::Entity::delete_by_id((tenant.id().as_uuid(), id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if deleted.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(not_found_id(id));
        }
        let event = events::OidcProviderDeleted { before: Some(describe(&before)), after: None };
        self.outbox
            .record(&transaction, tenant, Actor::from(principal), subject(id), &event)
            .await?;
        transaction.commit().await?;
        tracing::info!(provider_id = %id, "sign-in provider removed");
        Ok(())
    }

    /// Nút và role mặc định cho tài khoản mới: role phải có, nút trong khách, người khai phải có
    /// mọi permission của role (không tự nâng quyền qua người đăng ký); người mua không có role.
    async fn check_defaults(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        row: &Provider,
    ) -> Result<Vec<FieldError>, Error> {
        let Some(role) = row.default_role_id else {
            if row.default_org_id.is_some() {
                return Ok(vec![FieldError::new(
                    "default_role_id",
                    "required",
                    "is required together with `default_org_id`",
                )]);
            }
            return Ok(Vec::new());
        };
        if row.realm() == Realm::Customer {
            return Ok(vec![FieldError::new(
                "default_role_id",
                "invalid",
                "customer accounts have no roles; leave it out for `realm` `customer`",
            )]);
        }
        let org = row
            .default_org_id
            .unwrap_or_else(|| autoxide_kernel::tenancy::root_of(tenant.id()).as_uuid());
        let roles = BTreeSet::from([role]);
        let read = tenant.database().read();
        let problems =
            crate::role::grant_problems(read, tenant, principal, "default_", org, &roles).await?;
        // Câu chung của `grant_problems` không nói cách làm tiếp với nhà cung cấp: thay bằng câu
        // chỉ đường. Tên ô (`default_role_ids`) giữ nguyên — đã có trong bản phát hành.
        let mut checked = Vec::with_capacity(problems.len());
        for problem in problems {
            if problem.code != "more_than_you_have" {
                checked.push(problem);
                continue;
            }
            // Tên role đã có trong tham số của lỗi gốc (`grant_problems`), không đọc lại.
            let name = problem
                .params()
                .find(|(key, _)| *key == "role")
                .map_or_else(|| role.to_string(), |(_, value)| value.to_owned());
            checked.push(
                FieldError::new(
                    problem.field,
                    "more_than_you_have",
                    "role `{role}` has permissions you do not have, or has them more widely than \
                     you do; ask someone who can give this role to save the provider, or remove \
                     the default role",
                )
                .with_param("role", name),
            );
        }
        Ok(checked)
    }
}

/// Issuer của Entra cho mọi tổ chức (`…/organizations/v2.0`, `…/common/v2.0`), không gắn một
/// directory.
fn is_multi_tenant_entra(issuer: &str) -> bool {
    let path = issuer.trim_end_matches('/');
    ["/organizations/v2.0", "/common/v2.0", "/organizations", "/common"]
        .iter()
        .any(|tail| path.ends_with(tail))
}

/// Lỗi của các trường không cần database.
fn check(
    row: &Provider,
    client: &OidcClient,
    redirect_uris: &[String],
    domains: Option<&[String]>,
) -> Vec<FieldError> {
    let mut problems = Vec::new();
    if row.name.is_empty() || row.name.chars().count() > 100 {
        problems.push(FieldError::new("name", "length", "must be 1 to 100 characters"));
    }
    if row.client_id.is_empty() || row.client_id.len() > 255 {
        problems.push(FieldError::new("client_id", "length", "must be 1 to 255 characters"));
    }
    match (&row.issuer, row.kind()) {
        (None, Kind::Oidc) => problems.push(FieldError::new(
            "issuer",
            "required",
            "is required for `oidc`: the issuer address, e.g. `https://id.example.com`",
        )),
        (Some(issuer), Kind::Google) if issuer != GOOGLE_ISSUER => problems.push(FieldError::new(
            "issuer",
            "invalid",
            "must be left out for Google (it is always `https://accounts.google.com`)",
        )),
        (Some(issuer), _) => {
            if issuer.len() > 255 {
                problems.push(FieldError::new(
                    "issuer",
                    "length",
                    "must be at most 255 characters",
                ));
            } else if let Err(problem) = client.check_url(issuer) {
                problems.push(issuer_problem(problem));
            } else if issuer.contains(['?', '#', '{', '}']) {
                problems.push(FieldError::new(
                    "issuer",
                    "invalid",
                    "must not have a query, a fragment or braces",
                ));
            }
        }
        (None, _) => {}
    }
    if row.scopes.is_empty() || row.scopes.len() > 255 {
        problems.push(FieldError::new("scopes", "length", "must be 1 to 255 characters"));
    } else if row.kind() != Kind::Github && !row.scopes.split(' ').any(|scope| scope == "openid") {
        problems.push(FieldError::new("scopes", "invalid", "must include `openid`"));
    }
    if redirect_uris.is_empty() || redirect_uris.len() > MAX_REDIRECT_URIS {
        problems.push(
            FieldError::new("redirect_uris", "length", "must list 1 to {max} addresses")
                .with_param("max", MAX_REDIRECT_URIS),
        );
    }
    for (index, uri) in redirect_uris.iter().enumerate() {
        let valid = reqwest::Url::parse(uri).is_ok_and(|url| {
            let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
            (url.scheme() == "https" || (url.scheme() == "http" && local))
                && url.fragment().is_none()
                && uri.len() <= 500
        });
        if !valid {
            problems.push(FieldError::new(
                format!("redirect_uris[{index}]"),
                "invalid",
                "must be an `https://` address of the interface without `#` (plain `http://` only \
                 for localhost), at most 500 characters",
            ));
        }
    }
    // Entra nhiều tổ chức (`organizations`, `common`): bất kỳ công ty nào dùng Microsoft cũng đăng
    // nhập được — cho tự tạo tài khoản thì phải nêu tên miền email được phép.
    if row.kind() == Kind::Microsoft
        && row.allow_signup
        && row.issuer.as_deref().is_some_and(is_multi_tenant_entra)
        && domains.is_none_or(<[String]>::is_empty)
    {
        problems.push(FieldError::new(
            "allowed_email_domains",
            "required",
            "is required when `allow_signup` is on with a multi-organization Microsoft issuer \
             (`organizations` or `common`); list your company's email domains, or use your \
             directory's own issuer",
        ));
    }
    if let Some(list) = domains {
        let bad = list.len() > MAX_DOMAINS
            || list.iter().any(|domain| {
                let domain = domain.trim();
                domain.is_empty()
                    || domain.contains('@')
                    || domain.contains(' ')
                    || !domain.contains('.')
            });
        if bad {
            problems.push(
                FieldError::new(
                    "allowed_email_domains",
                    "invalid",
                    "must list at most {max} domain names like `example.com`",
                )
                .with_param("max", MAX_DOMAINS),
            );
        }
    }
    problems
}

fn issuer_problem(problem: UrlProblem) -> FieldError {
    match problem {
        UrlProblem::NotUrl => FieldError::new(
            "issuer",
            "invalid",
            "must be an absolute URL like `https://id.example.com`",
        ),
        UrlProblem::Credentials => {
            FieldError::new("issuer", "invalid", "must not contain a user name or password")
        }
        UrlProblem::Scheme => FieldError::new("issuer", "invalid", "must start with `https://`"),
        UrlProblem::Private => FieldError::new(
            "issuer",
            "invalid",
            "points to a private or local network address; sign-in providers must be on public \
             addresses unless the server sets `AUTOXIDE_AUTH_OIDC_ALLOW_PRIVATE_NETWORKS=true`",
        ),
    }
}

fn check_slug(slug: &str) -> Option<FieldError> {
    let valid = !slug.is_empty()
        && slug.len() <= 64
        && slug.starts_with(|c: char| c.is_ascii_lowercase())
        && slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    (!valid).then(|| {
        FieldError::new(
            "slug",
            "invalid",
            "must be 1 to 64 lowercase letters, digits and `-`, starting with a letter",
        )
    })
}

fn default_scopes(kind: Kind) -> &'static str {
    if kind == Kind::Github { GITHUB_SCOPES } else { DEFAULT_SCOPES }
}

fn normalize_scopes(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn domains(list: &[String]) -> Vec<String> {
    list.iter().map(|domain| domain.trim().to_ascii_lowercase()).collect()
}

/// Bỏ "người cấu hình" (`configured_by = NULL`) của mọi nhà cung cấp do người `user_id` cấu hình,
/// trong `connection` (transaction khoá tài khoản), mỗi nhà cung cấp một event
/// `auth.oidc_provider_updated`. Sức mạnh tự liên kết danh tính ngoài vào tài khoản có sẵn đo bằng
/// quyền HIỆN TẠI của người cấu hình: tài khoản bị khoá (hay được trao quyền cao sau này) mà vẫn là
/// người cấu hình thì nhà cung cấp kẻ tấn công dựng sẵn gộp được vào tài khoản quyền cao. Liên kết sẵn
/// có vẫn đăng nhập được; chỉ ngừng tự liên kết cho tới khi người quản lý lưu lại nhà cung cấp
/// (D-20261125-durable-grants).
pub(crate) async fn forget_configurer_in<C: sea_orm::ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    outbox: &Outbox,
    actor: Actor,
    user_id: Uuid,
) -> Result<(), Error> {
    let rows = entity::Entity::find()
        .filter(entity::Column::ConfiguredBy.eq(user_id))
        .scoped(tenant)
        .all(connection)
        .await?;
    for row in rows {
        let before = describe(&row);
        let mut active = row.clone().into_active_model();
        active.configured_by = Set(None);
        active.update(connection).await?;
        let mut after = before.clone();
        after["configured_by"] = serde_json::Value::Null;
        after["secret_changed"] = json!(false);
        let event = events::OidcProviderUpdated { before: Some(before), after: Some(after) };
        outbox.record(connection, tenant, actor, subject(row.id), &event).await?;
        tracing::info!(provider_id = %row.id, %user_id, "sign-in provider lost its configurer");
    }
    Ok(())
}

fn subject(id: Uuid) -> Option<String> {
    Some(format!("auth.oidc_provider:{id}"))
}

/// Bản mô tả cho event — không bao giờ có `client_secret`.
fn describe(row: &Provider) -> serde_json::Value {
    json!({
        "slug": row.slug,
        "kind": row.kind,
        "name": row.name,
        "issuer": row.issuer,
        "client_id": row.client_id,
        "scopes": row.scopes,
        "redirect_uris": row.redirect_uris(),
        "allowed_email_domains": row.allowed_email_domains(),
        "allow_signup": row.allow_signup,
        "realm": row.realm,
        "default_org_id": row.default_org_id,
        "default_role_id": row.default_role_id,
        "enabled": row.enabled,
    })
}

fn not_found_id(id: Uuid) -> Error {
    Error::not_found(
        "auth.oidc_provider_not_found",
        "There is no sign-in provider `{slug}`. See the list at `GET /api/v1/auth/oidc/providers`.",
    )
    .with_param("slug", id)
}

pub(crate) fn not_found_slug(slug: &str) -> Error {
    Error::not_found(
        "auth.oidc_provider_not_found",
        "There is no sign-in provider `{slug}`. See the list at `GET /api/v1/auth/oidc/providers`.",
    )
    .with_param("slug", slug)
}

fn slug_taken(slug: &str) -> Error {
    Error::conflict("auth.oidc_slug_taken", "There is already a sign-in provider named `{slug}`.")
        .with_param("slug", slug)
}

#[cfg(test)]
mod check_tests {
    use super::*;

    fn entra(issuer: &str, allow_signup: bool) -> Provider {
        let now = Timestamp::now();
        Provider {
            tenant_id: Uuid::now_v7(),
            id: Uuid::now_v7(),
            slug: "entra".to_owned(),
            kind: Kind::Microsoft.as_str().to_owned(),
            name: "Entra".to_owned(),
            issuer: Some(issuer.to_owned()),
            client_id: "client".to_owned(),
            client_secret: String::new(),
            scopes: DEFAULT_SCOPES.to_owned(),
            redirect_uris: "[]".to_owned(),
            allowed_email_domains: None,
            allow_signup,
            realm: "staff".to_owned(),
            default_org_id: None,
            default_role_id: None,
            enabled: true,
            created_at: now,
            updated_at: now,
            configured_by: None,
        }
    }

    /// Entra nhiều tổ chức cho tự tạo tài khoản thì phải nêu tên miền email.
    #[test]
    fn entra_nhieu_to_chuc_cho_dang_ky_phai_neu_ten_mien() {
        let client = OidcClient::new(false).unwrap();
        let uris = vec!["https://app.example.com/callback".to_owned()];
        let codes = |row: &Provider, domains: Option<&[String]>| {
            check(row, &client, &uris, domains)
                .into_iter()
                .filter(|problem| problem.field == "allowed_email_domains")
                .map(|problem| problem.code)
                .collect::<Vec<_>>()
        };
        let common = "https://login.microsoftonline.com/common/v2.0";
        assert_eq!(codes(&entra(MICROSOFT_ISSUER, true), None), ["required"]);
        assert_eq!(codes(&entra(common, true), Some(&[])), ["required"]);
        let listed = ["example.com".to_owned()];
        assert!(codes(&entra(MICROSOFT_ISSUER, true), Some(&listed)).is_empty());
        assert!(codes(&entra(MICROSOFT_ISSUER, false), None).is_empty(), "không cho đăng ký");
        let own = "https://login.microsoftonline.com/0d4c7a9e-0000-4000-8000-000000000000/v2.0";
        assert!(codes(&entra(own, true), None).is_empty(), "directory riêng");
    }
}
