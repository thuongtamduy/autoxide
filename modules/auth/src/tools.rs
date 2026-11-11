//! Tool MCP của module `auth` (D-068): thông tin người gọi, danh mục permission, và
//! quản lý người dùng. Mỗi tool gọi đúng service mà endpoint REST tương ứng gọi, với
//! cùng permission — tool không làm được gì hơn REST.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::mcp::{NoInput, PageInput, Tool};
use autoxide_kernel::pagination::Page;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::http::HttpState;
use crate::http::roles::PermissionResponse;
use crate::http::session::MeResponse;
use crate::http::users::UserResponse;
use crate::permissions::{ROLES_READ, USERS_MANAGE, USERS_READ};
use crate::user::UserChanges;

/// Changes to a user account; omitted fields stay as they are.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateUserInput {
    /// Id of the user.
    id: Uuid,
    /// New display name, 1 to 200 characters.
    #[validate(length(min = 1, max = 200))]
    name: Option<String>,
    /// `true` disables the account and signs it out everywhere; `false` enables it.
    disabled: Option<bool>,
}

pub(crate) fn all(state: &Arc<HttpState>) -> Vec<Tool> {
    let (me, catalog, list, update) = (state.clone(), state.clone(), state.clone(), state.clone());
    vec![
        Tool::new(
            "whoami",
            "Get the calling user (id, email, name) and the permissions they hold. Start here to \
             learn what you may do.",
            move |principal: Principal, _: NoInput| {
                let state = me.clone();
                async move {
                    let user = state.users.get(principal.user_id()).await?;
                    let user = user.ok_or_else(crate::token::invalid_token)?;
                    Ok(MeResponse::new(user, &principal))
                }
            },
        )
        .read_only(),
        Tool::new(
            "list_permissions",
            "List every permission that roles can grant, with what each allows.",
            move |_: Principal, _: NoInput| {
                let state = catalog.clone();
                async move { Ok::<_, Error>(PermissionResponse::catalog(&state.roles)) }
            },
        )
        .requires(ROLES_READ)
        .read_only(),
        Tool::new(
            "list_users",
            "List user accounts, newest first; follow `pagination.next_cursor` for more.",
            move |_: Principal, input: PageInput| {
                let state = list.clone();
                async move {
                    let page = state.users.list(&input.page()?).await?;
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
            move |principal: Principal, input: UpdateUserInput| {
                let state = update.clone();
                async move {
                    let changes = UserChanges { name: input.name, disabled: input.disabled };
                    let user =
                        state.users.update(Actor::from(&principal), input.id, changes).await?;
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
            self_registration: false,
        })
    }

    async fn call(
        tools: &[Tool],
        name: &str,
        principal: Principal,
        input: Value,
    ) -> Result<Value, Error> {
        let tool = tools.iter().find(|tool| tool.name() == name).unwrap();
        assert!(tool.allows(&principal), "`{name}` is not allowed");
        tool.call(principal, input).await
    }

    #[tokio::test]
    async fn tool_goi_cung_service_voi_rest() {
        TestDatabase::run(async |test| {
            let services = crate::testing::services(test).await;
            let ada =
                NewUser { email: "ada@example.com".into(), name: "Ada".into(), password: None };
            let ada = services.users.create(Actor::system(), ada).await.unwrap();
            let tools = all(&state(&services));
            let admin = Principal::with_all_permissions(ada.id);
            let nobody = Principal::new(ada.id, []);

            let me = call(&tools, "whoami", nobody.clone(), Value::Null).await.unwrap();
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

            let catalog = call(&tools, "list_permissions", admin.clone(), json!({})).await.unwrap();
            assert!(
                catalog.as_array().unwrap().iter().any(|item| item["name"] == "auth.users.read")
            );
            let page =
                call(&tools, "list_users", admin.clone(), json!({"limit": 1})).await.unwrap();
            assert_eq!(page["data"][0]["email"], "ada@example.com");
            assert!(page["pagination"].is_object());
            let bad = call(&tools, "list_users", admin.clone(), json!({"order": "sideways"})).await;
            assert_eq!(bad.unwrap_err().code(), "validation_failed");

            let input = json!({"id": ada.id, "name": "Ada Lovelace"});
            let changed = call(&tools, "update_user", admin.clone(), input).await.unwrap();
            assert_eq!(changed["name"], "Ada Lovelace");
            let input = json!({"id": ada.id, "name": ""});
            let invalid = call(&tools, "update_user", admin.clone(), input).await.unwrap_err();
            assert_eq!(invalid.code(), "validation_failed");
            let input = json!({"id": Uuid::now_v7(), "name": "Ghost"});
            let missing = call(&tools, "update_user", admin, input).await.unwrap_err();
            assert_eq!(missing.code(), "auth.user_not_found");
        })
        .await
        .unwrap();
    }
}
