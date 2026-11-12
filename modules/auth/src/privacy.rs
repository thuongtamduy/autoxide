//! Phần dữ liệu cá nhân module `auth` giữ về một người dùng, cho bản xuất dữ liệu
//! (`GET /api/v1/auth/me/export`). Không có secret: không mật khẩu, hash, token, secret
//! TOTP hay khoá công khai của passkey.

use std::future::Future;
use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{MAX_LIMIT, Order, Page, PageRequest};
use autoxide_kernel::privacy::PersonalData;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api_key::ApiKeyService;
use crate::mfa::MfaService;
use crate::passkey::PasskeyService;
use crate::role::RoleService;
use crate::session::SessionService;
use crate::user::{UserService, user_not_found};

pub(crate) struct AuthPersonalData {
    pub users: Arc<UserService>,
    pub roles: Arc<RoleService>,
    pub sessions: Arc<SessionService>,
    pub api_keys: Arc<ApiKeyService>,
    pub passkeys: Arc<PasskeyService>,
    pub mfa: Arc<MfaService>,
}

#[async_trait::async_trait]
impl PersonalData for AuthPersonalData {
    async fn export(&self, user_id: Uuid) -> Result<Value, Error> {
        let user = self.users.get(user_id).await?.ok_or_else(|| user_not_found(user_id))?;
        let roles = self.roles.user_roles(user_id).await?;
        let sessions = all_pages(|page| {
            let sessions = self.sessions.clone();
            async move { sessions.list_sessions(user_id, &page).await }
        })
        .await?;
        let api_keys = all_pages(|page| {
            let api_keys = self.api_keys.clone();
            async move { api_keys.list(user_id, &page).await }
        })
        .await?;
        let passkeys = all_pages(|page| {
            let passkeys = self.passkeys.clone();
            async move { passkeys.list(user_id, &page).await }
        })
        .await?;
        let mfa = self.mfa.status(user_id).await?;
        Ok(json!({
            "profile": {
                "id": user.id,
                "email": user.email,
                "name": user.name,
                "email_verified_at": user.email_verified_at,
                "disabled_at": user.disabled_at,
                "created_at": user.created_at,
                "updated_at": user.updated_at,
            },
            "roles": roles.iter().map(|role| &role.name).collect::<Vec<_>>(),
            "sessions": sessions.iter().map(|session| json!({
                "id": session.id,
                "user_agent": session.user_agent,
                "started_at": session.started_at,
                "last_active_at": session.last_active_at,
                "expires_at": session.expires_at,
            })).collect::<Vec<_>>(),
            "api_keys": api_keys.iter().map(|key| json!({
                "id": key.id,
                "name": key.name,
                "prefix": key.prefix,
                "permissions": key.permissions,
                "expires_at": key.expires_at,
                "last_used_at": key.last_used_at,
                "created_at": key.created_at,
            })).collect::<Vec<_>>(),
            "passkeys": passkeys.iter().map(|passkey| json!({
                "id": passkey.id,
                "name": passkey.name,
                "created_at": passkey.created_at,
                "last_used_at": passkey.last_used_at,
            })).collect::<Vec<_>>(),
            "two_step_sign_in": {
                "totp_enabled": mfa.totp_enabled,
                "recovery_codes_remaining": mfa.recovery_codes_remaining,
            },
        }))
    }
}

/// Mọi phần tử của một danh sách có pagination, lấy lần lượt từng trang lớn nhất.
async fn all_pages<T, F, Next>(mut fetch: F) -> Result<Vec<T>, Error>
where
    F: FnMut(PageRequest) -> Next,
    Next: Future<Output = Result<Page<T>, Error>>,
{
    let mut request = PageRequest::first(MAX_LIMIT, Order::Asc);
    let mut items = Vec::new();
    loop {
        let page = fetch(request).await?;
        items.extend(page.data);
        match page.pagination.next_cursor {
            Some(cursor) => {
                let cursor = cursor.parse().map_err(Error::internal)?;
                request = PageRequest::first(MAX_LIMIT, Order::Asc).after(cursor);
            }
            None => return Ok(items),
        }
    }
}
