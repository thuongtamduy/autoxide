//! Tool MCP của module `auth` (D-068): thông tin người gọi, danh mục permission, và
//! quản lý người dùng. Mỗi tool gọi đúng service mà endpoint REST tương ứng gọi, với
//! cùng permission — tool không làm được gì hơn REST.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::mcp::{NoInput, PageInput, Tool};
use autoxide_kernel::pagination::Page;
use autoxide_kernel::tenancy::TenantContext;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::http::HttpState;
use crate::http::api_keys::{ApiKeyResponse, CreateApiKeyRequest, CreatedApiKeyResponse};
use crate::http::roles::{PermissionResponse, RoleRequest, RoleResponse};
use crate::http::session::MeResponse;
use crate::http::users::{UserResponse, describe, describe_page};
use crate::permissions::{ROLES_MANAGE, ROLES_READ, USERS_MANAGE, USERS_READ};
use crate::role::RoleInput;
use crate::user::UserChanges;

/// Changes to a user account; omitted fields stay as they are.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateUserInput {
    /// Id of the user.
    id: Uuid,
    /// New display name, 1 to 200 characters.
    name: Option<String>,
    /// `true` disables the account and signs it out everywhere; `false` enables it.
    disabled: Option<bool>,
}

/// Which users to list; paging as for every list.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListUsersInput {
    /// Only users whose email or name contains this text, in any case.
    #[validate(length(max = 200))]
    search: Option<String>,
    /// Maximum number of users to return, 1 to 200. Defaults to 50.
    #[schema(minimum = 1, maximum = 200)]
    limit: Option<u32>,
    /// Where to continue: the `next_cursor` of the previous page. Omit for the first page.
    cursor: Option<String>,
    /// `desc` (newest first, the default) or `asc` (oldest first).
    order: Option<String>,
}

/// A role or user to act on.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct IdInput {
    /// The id.
    id: Uuid,
}

/// The full new content of a role.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplaceRoleInput {
    /// Id of the role.
    id: Uuid,
    /// Unique. Lowercase letters, digits, `_` and `-`, starting with a letter.
    name: String,
    /// What the role is for.
    #[serde(default)]
    description: String,
    /// Every permission the role grants for the whole tenant (replaces the old list); see
    /// `list_permissions`.
    #[serde(default)]
    permissions: Vec<String>,
    /// Permissions granted with a scope in the organization tree; each permission is listed once.
    #[serde(default)]
    #[schema(inline)]
    grants: Vec<crate::http::roles::RoleGrant>,
    /// `true`: the role's permissions work only for accounts with two-step sign-in.
    #[serde(default)]
    requires_mfa: bool,
}

/// The complete list of roles a user should have.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetUserRolesInput {
    /// Id of the user.
    user_id: Uuid,
    /// Ids of the roles. Replaces every role the user had; `[]` removes all.
    #[validate(length(max = 100))]
    role_ids: Vec<Uuid>,
}

/// Changes to a sign-in provider; fields left out stay as they are, `null` clears.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateOidcProviderInput {
    /// Id of the provider.
    id: Uuid,
    name: Option<String>,
    /// Issuer address; `null` resets it to the default of the kind.
    #[serde(default, deserialize_with = "crate::http::oidc::present")]
    #[schema(value_type = Option<String>, nullable)]
    issuer: Option<Option<String>>,
    client_id: Option<String>,
    /// A new client secret (write-only).
    client_secret: Option<String>,
    /// Space-separated scopes.
    scopes: Option<String>,
    /// Pages of the interface the provider may send the browser back to (replaces the list).
    redirect_uris: Option<Vec<String>>,
    /// Only emails at these domains may sign in; `null`: any.
    #[serde(default, deserialize_with = "crate::http::oidc::present")]
    #[schema(value_type = Option<Vec<String>>, nullable)]
    allowed_email_domains: Option<Option<Vec<String>>>,
    allow_signup: Option<bool>,
    #[serde(default, deserialize_with = "crate::http::oidc::present")]
    #[schema(value_type = Option<Uuid>, nullable)]
    default_org_id: Option<Option<Uuid>>,
    #[serde(default, deserialize_with = "crate::http::oidc::present")]
    #[schema(value_type = Option<Uuid>, nullable)]
    default_role_id: Option<Option<Uuid>>,
    enabled: Option<bool>,
}

pub(crate) fn all(state: &Arc<HttpState>) -> Vec<Tool> {
    let mut tools = users(state);
    tools.extend(oidc(state));
    tools.extend(memberships(state));
    tools.extend(roles(state));
    tools.extend(api_keys(state));
    tools.extend(invitations(state));
    tools
}

/// Tool về role: cùng service và permission với `/api/v1/auth/roles` và
/// `/api/v1/auth/users/{id}/roles`.
fn roles(state: &Arc<HttpState>) -> Vec<Tool> {
    let (list, get, create, replace, delete, of_user, set) = (
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
    );
    let count = state.clone();
    vec![
        Tool::new(
            "count_role_members",
            "Count the people holding each role (at any unit, each person once).",
            move |tenant: TenantContext, _: Principal, _: autoxide_kernel::mcp::NoInput| {
                let state = count.clone();
                async move {
                    let counts = state.roles.role_member_counts(&tenant).await?;
                    let data: Vec<crate::http::roles::RoleCountResponse> =
                        counts.into_iter().map(Into::into).collect();
                    Ok::<_, Error>(autoxide_kernel::response::Data { data })
                }
            },
        )
        .requires(ROLES_READ)
        .read_only(),
        Tool::new(
            "list_roles",
            "List roles with the permissions they grant; follow `pagination.next_cursor` for more.",
            move |tenant: TenantContext, _: Principal, input: PageInput| {
                let state = list.clone();
                async move {
                    let page = state.roles.list(&tenant, &input.page()?).await?;
                    Ok::<Page<RoleResponse>, Error>(page.map(RoleResponse::from))
                }
            },
        )
        .requires(ROLES_READ)
        .read_only(),
        Tool::new(
            "get_role",
            "Get one role.",
            move |tenant: TenantContext, _: Principal, input: IdInput| {
                let state = get.clone();
                async move {
                    Ok::<_, Error>(RoleResponse::from(state.roles.get(&tenant, input.id).await?))
                }
            },
        )
        .requires(ROLES_READ)
        .read_only(),
        Tool::new(
            "create_role",
            "Create a role that grants the listed permissions (see `list_permissions`).",
            move |tenant: TenantContext, principal: Principal, input: RoleRequest| {
                let state = create.clone();
                async move {
                    let role =
                        state.roles.create(&tenant, Actor::from(&principal), input.into()).await?;
                    Ok::<_, Error>(RoleResponse::from(role))
                }
            },
        )
        .requires(ROLES_MANAGE),
        Tool::new(
            "replace_role",
            "Replace the name, description and permissions of a role. The built-in `admin` \
             role cannot be changed.",
            move |tenant: TenantContext, principal: Principal, input: ReplaceRoleInput| {
                let state = replace.clone();
                async move {
                    let role = RoleInput {
                        requires_mfa: input.requires_mfa,
                        name: input.name,
                        description: input.description,
                        permissions: input.permissions,
                        grants: input
                            .grants
                            .into_iter()
                            .map(|grant| (grant.permission, grant.scope))
                            .collect(),
                    };
                    let role = state
                        .roles
                        .update(&tenant, Actor::from(&principal), input.id, role, None)
                        .await?;
                    Ok::<_, Error>(RoleResponse::from(role))
                }
            },
        )
        .requires(ROLES_MANAGE),
        Tool::new(
            "delete_role",
            "Delete a role; users who had it lose its permissions. The built-in `admin` role \
             cannot be deleted.",
            move |tenant: TenantContext, principal: Principal, input: IdInput| {
                let state = delete.clone();
                async move {
                    state.roles.delete(&tenant, Actor::from(&principal), input.id).await?;
                    Ok::<_, Error>(serde_json::json!({ "deleted": input.id }))
                }
            },
        )
        .requires(ROLES_MANAGE)
        .destructive(),
        Tool::new(
            "get_user_roles",
            "List the roles of a user (`id` is the user's id).",
            move |tenant: TenantContext, _: Principal, input: IdInput| {
                let state = of_user.clone();
                async move {
                    Ok::<_, Error>(roles_response(state.roles.user_roles(&tenant, input.id).await?))
                }
            },
        )
        .requires(USERS_READ)
        .read_only(),
        Tool::new(
            "set_user_roles",
            "Replace every role of a user with `role_ids`. The last administrator cannot lose \
             the `admin` role.",
            move |tenant: TenantContext, principal: Principal, input: SetUserRolesInput| {
                let state = set.clone();
                async move {
                    let actor = Actor::from(&principal);
                    let roles =
                        state.roles.set_user_roles(&tenant, actor, input.user_id, input.role_ids);
                    Ok::<_, Error>(roles_response(roles.await?))
                }
            },
        )
        .requires(ROLES_MANAGE),
    ]
}

/// Tool về API key của chính người gọi: cùng service với `/api/v1/auth/api-keys`.
fn api_keys(state: &Arc<HttpState>) -> Vec<Tool> {
    let (list, create, revoke) = (state.clone(), state.clone(), state.clone());
    vec![
        Tool::new(
            "list_api_keys",
            "List your API keys (never their secrets); follow `pagination.next_cursor` for more.",
            move |tenant: TenantContext, principal: Principal, input: PageInput| {
                let state = list.clone();
                async move {
                    let page =
                        state.api_keys.list(&tenant, principal.user_id(), &input.page()?).await?;
                    Ok::<Page<ApiKeyResponse>, Error>(page.map(ApiKeyResponse::from))
                }
            },
        )
        .for_any_signed_in()
        .read_only(),
        Tool::new(
            "create_api_key",
            "Create an API key that acts as you, limited to the listed permissions. Needs your \
             current password and a password session: an API key cannot create keys. The \
             result carries the key, shown only once.",
            move |tenant: TenantContext, principal: Principal, input: CreateApiKeyRequest| {
                let state = create.clone();
                async move {
                    let created = state.api_keys.create(&tenant, &principal, input.into()).await?;
                    Ok::<_, Error>(CreatedApiKeyResponse::from(created))
                }
            },
        )
        .for_any_signed_in()
        .returns_secret(),
        Tool::new(
            "revoke_api_key",
            "Revoke one of your API keys; it stops working at once.",
            move |tenant: TenantContext, principal: Principal, input: IdInput| {
                let state = revoke.clone();
                async move {
                    state.api_keys.revoke(&tenant, &principal, input.id).await?;
                    Ok::<_, Error>(serde_json::json!({ "revoked": input.id }))
                }
            },
        )
        .for_any_signed_in()
        .destructive(),
    ]
}

fn roles_response(roles: Vec<crate::role::Role>) -> Vec<RoleResponse> {
    roles.into_iter().map(RoleResponse::from).collect()
}

/// Tool về người dùng: cùng service và permission với `/api/v1/auth/me` và
/// `/api/v1/auth/users`.
fn users(state: &Arc<HttpState>) -> Vec<Tool> {
    let (me, catalog, list, update) = (state.clone(), state.clone(), state.clone(), state.clone());
    vec![
        Tool::new(
            "whoami",
            "Get the calling user (id, email, name) and the permissions they hold. Start here to \
             learn what you may do.",
            move |tenant: TenantContext, principal: Principal, _: NoInput| {
                let state = me.clone();
                async move { MeResponse::load(&state, &tenant, &principal).await }
            },
        )
        .for_any_signed_in()
        .read_only(),
        Tool::new(
            "list_permissions",
            "List every permission that roles can grant, with what each allows.",
            move |_tenant: TenantContext, _: Principal, _: NoInput| {
                let state = catalog.clone();
                async move {
                    Ok::<_, Error>(PermissionResponse::catalog(
                        &state.roles,
                        autoxide_kernel::i18n::Language::En,
                    ))
                }
            },
        )
        .requires(ROLES_READ)
        .read_only(),
        Tool::new(
            "list_users",
            "List user accounts, newest first; `search` finds users by email or name. Follow \
             `pagination.next_cursor` for more.",
            move |tenant: TenantContext, _: Principal, input: ListUsersInput| {
                let state = list.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, input.order);
                    let page = state
                        .users
                        .list(&tenant, input.search.as_deref(), None, &page.page()?)
                        .await?;
                    let page =
                        describe_page(&state.users, &state.roles, &tenant, page, false).await?;
                    Ok::<Page<UserResponse>, Error>(page)
                }
            },
        )
        .requires(USERS_READ)
        .read_only(),
        Tool::new(
            "update_user",
            "Rename, disable or enable a user account. Disabling signs the user out \
             everywhere; the last active administrator cannot be disabled.",
            move |tenant: TenantContext, principal: Principal, input: UpdateUserInput| {
                let state = update.clone();
                async move {
                    // Như REST: không tác động lên người có permission mà mình không có.
                    state.roles.require_covers(&tenant, &principal, input.id).await?;
                    let changes = UserChanges {
                        name: input.name,
                        disabled: input.disabled,
                        ..Default::default()
                    };
                    let user = state
                        .users
                        .update(&tenant, Actor::from(&principal), input.id, changes, None)
                        .await?;
                    describe(&state.users, &tenant, user).await
                }
            },
        )
        .requires(USERS_MANAGE),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_testkit::TestDatabase;
    use serde_json::{Value, json};

    use crate::user::NewUser;

    fn state(services: &crate::testing::Services) -> Arc<HttpState> {
        Arc::new(HttpState {
            oidc: Arc::new(crate::oidc::OidcService {
                enabled: false,
                providers: crate::oidc::ProviderService {
                    secrets: autoxide_kernel::secret::SecretKey::new(
                        services.database.clone(),
                        None,
                    ),
                    outbox: autoxide_kernel::event::Outbox::standalone("auth"),
                },
                client: crate::oidc::OidcClient::new(false).unwrap(),
                users: services.users.clone(),
                sessions: services.sessions.clone(),
                outbox: autoxide_kernel::event::Outbox::standalone("auth"),
                secrets: autoxide_kernel::secret::SecretKey::new(services.database.clone(), None),
                databases: autoxide_kernel::tenancy::DatabaseRegistry::single(
                    services.database.clone(),
                ),
                orgs: None,
                entitlements: Default::default(),
                self_registration: false,
            }),
            sessions: services.sessions.clone(),
            users: services.users.clone(),
            roles: services.roles.clone(),
            api_keys: services.api_keys.clone(),
            mfa: services.mfa.clone(),
            resets: services.resets.clone(),
            verifications: services.verifications.clone(),
            passkeys: services.passkeys.clone(),
            reauth: services.reauth.clone(),
            orgs: None,
            support: Arc::new(crate::support::SupportService {
                databases: autoxide_kernel::tenancy::DatabaseRegistry::single(
                    services.database.clone(),
                ),
                outbox: autoxide_kernel::event::Outbox::standalone("auth"),
                orgs: None,
                max_hours: 8,
            }),
            invitations: Arc::new(crate::invitation::Invitations {
                users: services.users.clone(),
                sessions: services.sessions.clone(),
                jobs: autoxide_kernel::job::JobQueue::new("auth"),
                mailer: services.mails.clone(),
                translations: autoxide_kernel::i18n::Translations::default(),
                outbox: autoxide_kernel::event::Outbox::standalone("auth"),
                databases: autoxide_kernel::tenancy::DatabaseRegistry::single(
                    services.database.clone(),
                ),
                links: Default::default(),
                url: None,
                days: 7,
                entitlements: Default::default(),
            }),
            login_links: Arc::new(crate::login_link::LoginLinks {
                enabled: false,
                jobs: autoxide_kernel::job::JobQueue::new("auth"),
                users: services.users.clone(),
                sessions: services.sessions.clone(),
                mailer: services.mails.clone(),
                translations: autoxide_kernel::i18n::Translations::default(),
                outbox: autoxide_kernel::event::Outbox::standalone("auth"),
                links: Default::default(),
                url: None,
                minutes: 15,
                databases: autoxide_kernel::tenancy::DatabaseRegistry::single(
                    services.database.clone(),
                ),
            }),
            delegations: Arc::new(crate::delegation::DelegationService {
                databases: autoxide_kernel::tenancy::DatabaseRegistry::single(
                    services.database.clone(),
                ),
                outbox: autoxide_kernel::event::Outbox::standalone("auth"),
            }),
            personal_data: services.personal_data.clone(),
            self_registration: false,
            country_header: None,
        })
    }

    async fn call(
        tools: &[Tool],
        tenant: &TenantContext,
        name: &str,
        principal: Principal,
        input: Value,
    ) -> Result<Value, Error> {
        let tool = tools.iter().find(|tool| tool.name() == name).unwrap();
        assert!(tool.allows(&principal), "`{name}` is not allowed");
        tool.call(tenant.clone(), principal, input).await
    }

    #[tokio::test]
    async fn tool_goi_cung_service_voi_rest() {
        TestDatabase::run(async |test| {
            let services = crate::testing::services(test).await;
            let tenant = services.tenant.clone();
            let ada =
                NewUser { email: "ada@example.com".into(), name: "Ada".into(), password: None };
            let ada = services.users.create(&tenant, Actor::system(), ada).await.unwrap();
            let tenant = services.tenant.clone();
            let tools = all(&state(&services));
            let admin = Principal::with_all_permissions(ada.id);
            let nobody = Principal::new(ada.id, []);

            let me = call(&tools, &tenant, "whoami", nobody.clone(), Value::Null).await.unwrap();
            assert_eq!(
                (me["email"].as_str(), me["all_permissions"].as_bool()),
                (Some("ada@example.com"), Some(false))
            );
            // Tool cần permission: người không có thì không được thấy.
            let guarded = ["list_permissions", "list_users", "update_user"];
            for name in guarded {
                let tool = tools.iter().find(|tool| tool.name() == name).unwrap();
                assert!(!tool.allows(&nobody), "{name}");
            }

            let catalog =
                call(&tools, &tenant, "list_permissions", admin.clone(), json!({})).await.unwrap();
            assert!(
                catalog.as_array().unwrap().iter().any(|item| item["name"] == "auth.users.read")
            );
            let page = call(&tools, &tenant, "list_users", admin.clone(), json!({"limit": 1}))
                .await
                .unwrap();
            assert_eq!(page["data"][0]["email"], "ada@example.com");
            assert!(page["pagination"].is_object());
            let found = json!({"search": "ADA@"});
            let page = call(&tools, &tenant, "list_users", admin.clone(), found).await.unwrap();
            assert_eq!(page["data"].as_array().unwrap().len(), 1);
            let none = json!({"search": "nobody"});
            let page = call(&tools, &tenant, "list_users", admin.clone(), none).await.unwrap();
            assert_eq!(page["data"], json!([]));
            let bad =
                call(&tools, &tenant, "list_users", admin.clone(), json!({"order": "sideways"}))
                    .await;
            assert_eq!(bad.unwrap_err().code(), "validation_failed");

            let input = json!({"id": ada.id, "name": "Ada Lovelace"});
            let changed = call(&tools, &tenant, "update_user", admin.clone(), input).await.unwrap();
            assert_eq!(changed["name"], "Ada Lovelace");
            let input = json!({"id": ada.id, "name": ""});
            let invalid =
                call(&tools, &tenant, "update_user", admin.clone(), input).await.unwrap_err();
            assert_eq!(invalid.code(), "validation_failed");
            let input = json!({"id": Uuid::now_v7(), "name": "Ghost"});
            let missing = call(&tools, &tenant, "update_user", admin, input).await.unwrap_err();
            assert_eq!(missing.code(), "auth.user_not_found");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn tool_role_va_api_key_dung_service_cua_rest() {
        TestDatabase::run(async |test| {
            let services = crate::testing::services(test).await;
            let tenant = services.tenant.clone();
            let password = "correct horse battery staple".to_owned();
            let ada = NewUser {
                email: "ada@example.com".into(),
                name: "Ada".into(),
                password: Some(password.clone()),
            };
            let ada = services.users.create(&tenant, Actor::system(), ada).await.unwrap();
            let tenant = services.tenant.clone();
            let tools = all(&state(&services));
            let admin = Principal::with_all_permissions(ada.id);
            let reader = Principal::new(ada.id, ["auth.roles.read".to_owned()]);

            for name in ["create_role", "replace_role", "delete_role", "set_user_roles"] {
                let tool = tools.iter().find(|tool| tool.name() == name).unwrap();
                assert!(!tool.allows(&reader), "{name}");
            }
            let input = json!({"name": "editor", "permissions": ["auth.users.read"]});
            let role = call(&tools, &tenant, "create_role", admin.clone(), input).await.unwrap();
            assert_eq!(role["permissions"], json!(["auth.users.read"]));
            let id = role["id"].clone();
            let input = json!({"id": id, "name": "editor", "permissions": []});
            let replaced = call(&tools, &tenant, "replace_role", admin.clone(), input).await.unwrap();
            assert_eq!(replaced["permissions"], json!([]));
            let page = call(&tools, &tenant, "list_roles", reader.clone(), json!({})).await.unwrap();
            assert!(page["data"].as_array().unwrap().iter().any(|role| role["name"] == "editor"));
            let input = json!({"user_id": ada.id, "role_ids": [id]});
            let set = call(&tools, &tenant, "set_user_roles", admin.clone(), input).await.unwrap();
            assert_eq!(set[0]["name"], "editor");
            let of_user = json!({"id": ada.id});
            let roles = call(&tools, &tenant, "get_user_roles", admin.clone(), of_user).await.unwrap();
            assert_eq!(roles.as_array().unwrap().len(), 1);

            // API key: cần mật khẩu như REST; key không tạo được key.
            let input = json!({"name": "agent", "permissions": ["auth.users.read"], "current_password": "nope"});
            let wrong = call(&tools, &tenant, "create_api_key", admin.clone(), input).await.unwrap_err();
            assert_eq!(wrong.code(), "auth.wrong_password");
            let input = json!({"name": "agent", "permissions": ["auth.users.read"], "current_password": password});
            let created = call(&tools, &tenant, "create_api_key", admin.clone(), input.clone()).await.unwrap();
            assert!(created["key"].as_str().unwrap().starts_with("axk1_"));
            // Kết quả có secret: response MCP mang `Cache-Control: no-store` (D-149).
            assert!(tools.iter().any(|tool| tool.name() == "create_api_key" && tool.is_secret()));
            let key_id: Uuid = created["id"].as_str().unwrap().parse().unwrap();
            let via_key = admin.clone().via_api_key(key_id);
            let refused = call(&tools, &tenant, "create_api_key", via_key, input).await.unwrap_err();
            assert_eq!(refused.code(), "auth.api_key_cannot_create_keys");
            let keys = call(&tools, &tenant, "list_api_keys", admin.clone(), json!({})).await.unwrap();
            assert!(keys["data"][0].get("key").is_none());
            let revoked = call(&tools, &tenant, "revoke_api_key", admin.clone(), json!({"id": key_id})).await;
            assert_eq!(revoked.unwrap()["revoked"], key_id.to_string());

            let deleted = call(&tools, &tenant, "delete_role", admin, json!({"id": id})).await.unwrap();
            assert_eq!(deleted["deleted"], id);
        })
        .await
        .unwrap();
    }
}

/// The units a user should work in, with the roles there.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetMembershipsInput {
    /// Id of the user.
    user_id: Uuid,
    /// Replaces every membership; `[]` removes all.
    #[schema(inline)]
    memberships: Vec<MembershipInput>,
}

/// The roles to give in one unit.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MembershipInput {
    /// The unit; the id of the tenant is the root.
    org_id: Uuid,
    /// Ids of the roles in this unit.
    role_ids: Vec<Uuid>,
}

/// Which memberships to list.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListMembershipsInput {
    /// Only memberships on this unit (with `inherited`: also on the units above it).
    org_id: Option<Uuid>,
    /// Only memberships with this role.
    role_id: Option<Uuid>,
    /// Only the memberships of this user.
    user_id: Option<Uuid>,
    /// With `org_id`: also memberships on the units above it.
    #[serde(default)]
    inherited: bool,
    /// Maximum number of items to return, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `desc` (newest first, the default) or `asc`.
    order: Option<String>,
}

/// A new invitation.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateInvitationInput {
    /// Email to invite.
    email: String,
    /// Display name for the new account.
    name: Option<String>,
    /// Unit the roles apply at; omit for the root unit. It must be in your scope.
    org_id: Option<Uuid>,
    /// Roles given on acceptance; you must have every permission of each.
    role_ids: Vec<Uuid>,
    /// Your own reference, up to 200 characters.
    reference: Option<String>,
}

/// Which invitations to list.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListInvitationsInput {
    /// Only `pending`, `accepted` or `revoked`.
    status: Option<String>,
    /// Maximum number of items to return, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `desc` (newest first, the default) or `asc`.
    order: Option<String>,
}

/// Tool về nhà cung cấp đăng nhập (bước 40 P3): cùng service và permission với
/// `/api/v1/auth/oidc-providers` và `/api/v1/auth/me/identities`. Luồng đăng nhập không có tool:
/// nó cần trình duyệt, còn MCP xác thực bằng API key.
fn oidc(state: &Arc<HttpState>) -> Vec<Tool> {
    use crate::http::oidc::{IdentityResponse, ProviderRequest, ProviderResponse};
    use crate::permissions::{OIDC_MANAGE, OIDC_READ};
    let (list, create, update, delete, mine) =
        (state.clone(), state.clone(), state.clone(), state.clone(), state.clone());
    vec![
        Tool::new(
            "list_oidc_providers",
            "List the sign-in providers (Google, Microsoft, GitHub, OpenID Connect) of this \
             tenant, with disabled ones; client secrets are never shown.",
            move |tenant: TenantContext, _: Principal, _: NoInput| {
                let state = list.clone();
                async move {
                    let rows = state.oidc.providers.list(&tenant).await?;
                    let data: Vec<ProviderResponse> = rows.into_iter().map(Into::into).collect();
                    Ok::<_, Error>(autoxide_kernel::response::Data { data })
                }
            },
        )
        .requires(OIDC_READ)
        .read_only(),
        Tool::new(
            "create_oidc_provider",
            "Add a sign-in provider to this tenant; register the application at the provider \
             first. The client secret is stored encrypted and never shown again.",
            move |tenant: TenantContext, principal: Principal, input: ProviderRequest| {
                let state = create.clone();
                async move {
                    let oidc = &state.oidc;
                    let row = oidc
                        .providers
                        .create(&tenant, &principal, &oidc.client, input.into_new())
                        .await?;
                    Ok::<_, Error>(ProviderResponse::from(row))
                }
            },
        )
        .requires(OIDC_MANAGE),
        Tool::new(
            "update_oidc_provider",
            "Change a sign-in provider of this tenant (name, issuer, client, redirect pages, \
             allowed email domains, sign-up, default role, enabled).",
            move |tenant: TenantContext, principal: Principal, input: UpdateOidcProviderInput| {
                let state = update.clone();
                async move {
                    let oidc = &state.oidc;
                    let changes = crate::oidc::provider::ProviderChanges {
                        name: input.name,
                        issuer: input.issuer,
                        client_id: input.client_id,
                        client_secret: input.client_secret,
                        scopes: input.scopes,
                        redirect_uris: input.redirect_uris,
                        allowed_email_domains: input.allowed_email_domains,
                        allow_signup: input.allow_signup,
                        default_org_id: input.default_org_id,
                        default_role_id: input.default_role_id,
                        enabled: input.enabled,
                    };
                    let row = oidc
                        .providers
                        .update(&tenant, &principal, &oidc.client, input.id, changes)
                        .await?;
                    Ok::<_, Error>(ProviderResponse::from(row))
                }
            },
        )
        .requires(OIDC_MANAGE),
        Tool::new(
            "delete_oidc_provider",
            "Remove a sign-in provider from this tenant, with every account link to it.",
            move |tenant: TenantContext, principal: Principal, input: IdInput| {
                let state = delete.clone();
                async move {
                    state.oidc.providers.delete(&tenant, &principal, input.id).await?;
                    Ok::<_, Error>(serde_json::json!({ "deleted": input.id }))
                }
            },
        )
        .requires(OIDC_MANAGE)
        .destructive(),
        Tool::new(
            "list_my_identities",
            "List the outside sign-ins (Google, Microsoft, GitHub…) linked to your own account.",
            move |tenant: TenantContext, principal: Principal, _: NoInput| {
                let state = mine.clone();
                async move {
                    let linked = state.oidc.identities(&tenant, principal.user_id()).await?;
                    let data: Vec<IdentityResponse> = linked.into_iter().map(Into::into).collect();
                    Ok::<_, Error>(autoxide_kernel::response::Data { data })
                }
            },
        )
        .for_any_signed_in()
        .read_only(),
    ]
}

/// Tool về lời mời (D-312): cùng service với `/api/v1/auth/invitations`.
fn invitations(state: &Arc<HttpState>) -> Vec<Tool> {
    use crate::http::invitations::InvitationResponse;
    use crate::permissions::INVITATIONS_MANAGE;
    let (create, list, revoke) = (state.clone(), state.clone(), state.clone());
    vec![
        Tool::new(
            "create_invitation",
            "Invite someone by email to work at an organization unit with given roles; the mail \
             goes out shortly and the link can be used once.",
            move |tenant: TenantContext, principal: Principal, input: CreateInvitationInput| {
                let state = create.clone();
                async move {
                    let input = crate::invitation::NewInvitation {
                        email: input.email,
                        name: input.name,
                        org_id: input.org_id,
                        role_ids: input.role_ids,
                        reference: input.reference,
                    };
                    let language = autoxide_kernel::i18n::Language::En;
                    let row = state
                        .invitations
                        .create(&tenant, Some(&principal), input, language)
                        .await?;
                    Ok::<_, Error>(InvitationResponse::from(row))
                }
            },
        )
        .requires(INVITATIONS_MANAGE),
        Tool::new(
            "list_invitations",
            "List the invitations at units in your scope, newest first; filter by status.",
            move |tenant: TenantContext, principal: Principal, input: ListInvitationsInput| {
                let state = list.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, input.order).page()?;
                    let status = input.status.as_deref();
                    let page = state.invitations.list(&tenant, &principal, status, &page).await?;
                    Ok::<_, Error>(page.map(InvitationResponse::from))
                }
            },
        )
        .requires(INVITATIONS_MANAGE)
        .read_only(),
        Tool::new(
            "revoke_invitation",
            "Revoke an invitation: its link stops working.",
            move |tenant: TenantContext, principal: Principal, input: IdInput| {
                let state = revoke.clone();
                async move {
                    let row = state.invitations.revoke(&tenant, &principal, input.id).await?;
                    Ok::<_, Error>(InvitationResponse::from(row))
                }
            },
        )
        .requires(INVITATIONS_MANAGE),
    ]
}

/// Tool về membership theo nút (bước 34): cùng service với `/api/v1/auth/memberships`.
fn memberships(state: &Arc<HttpState>) -> Vec<Tool> {
    let (list, set, count) = (state.clone(), state.clone(), state.clone());
    vec![
        Tool::new(
            "count_memberships",
            "Count the people of every organization unit: `direct` (memberships on the unit) and \
             `total` (on the unit or a unit above it).",
            move |tenant: TenantContext, _: Principal, _: autoxide_kernel::mcp::NoInput| {
                let state = count.clone();
                async move {
                    let counts = state.roles.member_counts(&tenant).await?;
                    let data: Vec<crate::http::memberships::MemberCountResponse> =
                        counts.into_iter().map(Into::into).collect();
                    Ok::<_, Error>(autoxide_kernel::response::Data { data })
                }
            },
        )
        .requires(USERS_READ)
        .read_only(),
        Tool::new(
            "list_memberships",
            "List who works in which organization unit with which role; filter by unit, role or \
             user, and with `inherited` also count memberships on the units above.",
            move |tenant: TenantContext, _: Principal, input: ListMembershipsInput| {
                let state = list.clone();
                async move {
                    let filter = crate::role::MembershipFilter {
                        org_id: input.org_id,
                        role_id: input.role_id,
                        user_id: input.user_id,
                        inherited: input.inherited,
                    };
                    let page = autoxide_kernel::mcp::PageInput::new(
                        input.limit,
                        input.cursor,
                        input.order,
                    )
                    .page()?;
                    let page = state.roles.list_memberships(&tenant, &filter, &page).await?;
                    Ok::<_, Error>(page.map(crate::http::memberships::MembershipRowResponse::from))
                }
            },
        )
        .requires(USERS_READ)
        .read_only(),
        Tool::new(
            "set_memberships",
            "Replace every organization unit a user works in and the roles there. You can only \
             give roles whose permissions you have, in units you work in.",
            move |tenant: TenantContext, principal: Principal, input: SetMembershipsInput| {
                let state = set.clone();
                async move {
                    let wanted = input
                        .memberships
                        .into_iter()
                        .map(|entry| (entry.org_id, entry.role_ids))
                        .collect();
                    let after = state
                        .roles
                        .set_memberships(&tenant, &principal, input.user_id, wanted)
                        .await?;
                    let after =
                        crate::http::memberships::memberships(&state, &tenant, after).await?;
                    Ok::<_, Error>(serde_json::json!({ "memberships": after.data }))
                }
            },
        )
        .requires(ROLES_MANAGE),
    ]
}
