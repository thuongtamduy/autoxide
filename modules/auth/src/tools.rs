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
use crate::http::users::UserResponse;
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
    /// Every permission the role grants (replaces the old list); see `list_permissions`.
    #[serde(default)]
    permissions: Vec<String>,
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

pub(crate) fn all(state: &Arc<HttpState>) -> Vec<Tool> {
    let mut tools = users(state);
    tools.extend(roles(state));
    tools.extend(api_keys(state));
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
    vec![
        Tool::new(
            "list_roles",
            "List roles with the permissions they grant; follow `pagination.next_cursor` for more.",
            move |_tenant: TenantContext, _: Principal, input: PageInput| {
                let state = list.clone();
                async move {
                    let page = state.roles.list(&input.page()?).await?;
                    Ok::<Page<RoleResponse>, Error>(page.map(RoleResponse::from))
                }
            },
        )
        .requires(ROLES_READ)
        .read_only(),
        Tool::new("get_role", "Get one role.", move |_tenant: TenantContext, _: Principal, input: IdInput| {
            let state = get.clone();
            async move { Ok::<_, Error>(RoleResponse::from(state.roles.get(input.id).await?)) }
        })
        .requires(ROLES_READ)
        .read_only(),
        Tool::new(
            "create_role",
            "Create a role that grants the listed permissions (see `list_permissions`).",
            move |_tenant: TenantContext, principal: Principal, input: RoleRequest| {
                let state = create.clone();
                async move {
                    let role = state.roles.create(Actor::from(&principal), input.into()).await?;
                    Ok::<_, Error>(RoleResponse::from(role))
                }
            },
        )
        .requires(ROLES_MANAGE),
        Tool::new(
            "replace_role",
            "Replace the name, description and permissions of a role. The built-in `admin` \
             role cannot be changed.",
            move |_tenant: TenantContext, principal: Principal, input: ReplaceRoleInput| {
                let state = replace.clone();
                async move {
                    let role = RoleInput {
                        name: input.name,
                        description: input.description,
                        permissions: input.permissions,
                    };
                    let role =
                        state.roles.update(Actor::from(&principal), input.id, role, None).await?;
                    Ok::<_, Error>(RoleResponse::from(role))
                }
            },
        )
        .requires(ROLES_MANAGE),
        Tool::new(
            "delete_role",
            "Delete a role; users who had it lose its permissions. The built-in `admin` role \
             cannot be deleted.",
            move |_tenant: TenantContext, principal: Principal, input: IdInput| {
                let state = delete.clone();
                async move {
                    state.roles.delete(Actor::from(&principal), input.id).await?;
                    Ok::<_, Error>(serde_json::json!({ "deleted": input.id }))
                }
            },
        )
        .requires(ROLES_MANAGE)
        .destructive(),
        Tool::new(
            "get_user_roles",
            "List the roles of a user (`id` is the user's id).",
            move |_tenant: TenantContext, _: Principal, input: IdInput| {
                let state = of_user.clone();
                async move { Ok::<_, Error>(roles_response(state.roles.user_roles(input.id).await?)) }
            },
        )
        .requires(USERS_READ)
        .read_only(),
        Tool::new(
            "set_user_roles",
            "Replace every role of a user with `role_ids`. The last administrator cannot lose \
             the `admin` role.",
            move |_tenant: TenantContext, principal: Principal, input: SetUserRolesInput| {
                let state = set.clone();
                async move {
                    let actor = Actor::from(&principal);
                    let roles = state.roles.set_user_roles(actor, input.user_id, input.role_ids);
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
            move |_tenant: TenantContext, principal: Principal, input: PageInput| {
                let state = list.clone();
                async move {
                    let page = state.api_keys.list(principal.user_id(), &input.page()?).await?;
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
            move |_tenant: TenantContext, principal: Principal, input: CreateApiKeyRequest| {
                let state = create.clone();
                async move {
                    let created = state.api_keys.create(&principal, input.into()).await?;
                    Ok::<_, Error>(CreatedApiKeyResponse::from(created))
                }
            },
        )
        .for_any_signed_in()
        .returns_secret(),
        Tool::new(
            "revoke_api_key",
            "Revoke one of your API keys; it stops working at once.",
            move |_tenant: TenantContext, principal: Principal, input: IdInput| {
                let state = revoke.clone();
                async move {
                    state.api_keys.revoke(&principal, input.id).await?;
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
            move |_tenant: TenantContext, principal: Principal, _: NoInput| {
                let state = me.clone();
                async move {
                    let user = state.users.get(principal.user_id()).await?;
                    let user = user.ok_or_else(crate::token::invalid_token)?;
                    Ok(MeResponse::new(user, &principal))
                }
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
            move |_tenant: TenantContext, _: Principal, input: ListUsersInput| {
                let state = list.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, input.order);
                    let page = state.users.list(input.search.as_deref(), &page.page()?).await?;
                    Ok::<Page<UserResponse>, Error>(page.map(UserResponse::from))
                }
            },
        )
        .requires(USERS_READ)
        .read_only(),
        Tool::new(
            "update_user",
            "Rename, disable or enable a user account. Disabling signs the user out \
             everywhere; the last active administrator cannot be disabled.",
            move |_tenant: TenantContext, principal: Principal, input: UpdateUserInput| {
                let state = update.clone();
                async move {
                    // Như REST: không tác động lên người có permission mà mình không có.
                    state.roles.require_covers(&principal, input.id).await?;
                    let changes = UserChanges { name: input.name, disabled: input.disabled };
                    let user = state
                        .users
                        .update(Actor::from(&principal), input.id, changes, None)
                        .await?;
                    Ok::<_, Error>(UserResponse::from(user))
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
            sessions: services.sessions.clone(),
            users: services.users.clone(),
            roles: services.roles.clone(),
            api_keys: services.api_keys.clone(),
            mfa: services.mfa.clone(),
            resets: services.resets.clone(),
            verifications: services.verifications.clone(),
            passkeys: services.passkeys.clone(),
            personal_data: services.personal_data.clone(),
            self_registration: false,
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
            let ada =
                NewUser { email: "ada@example.com".into(), name: "Ada".into(), password: None };
            let ada = services.users.create(Actor::system(), ada).await.unwrap();
            let tenant = TenantContext::legacy_default(&services.database);
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
            let password = "correct horse battery staple".to_owned();
            let ada = NewUser {
                email: "ada@example.com".into(),
                name: "Ada".into(),
                password: Some(password.clone()),
            };
            let ada = services.users.create(Actor::system(), ada).await.unwrap();
            let tenant = TenantContext::legacy_default(&services.database);
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
            assert!(created["key"].as_str().unwrap().starts_with("axk_"));
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
