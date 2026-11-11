//! Event module `auth` phát. Mọi event có cùng hình dạng `{"before": …, "after": …}`
//! (trường nào không có thì bỏ): audit log lấy "trước / sau" từ đó.
//!
//! Không bao giờ đặt mật khẩu, hash, secret TOTP, token hay API key vào event.

use autoxide_kernel::event::{Event, EventInfo};
use serde::Serialize;
use serde_json::Value;

/// Khai một event: hằng `EventInfo` (cho `Module::events`) và kiểu dữ liệu của nó.
macro_rules! events {
    ($($info:ident, $kind:ident, $name:literal, $description:literal;)*) => {
        $(
            pub(crate) const $info: EventInfo = EventInfo::new($name, $description);

            #[derive(Debug, Default, Serialize)]
            pub(crate) struct $kind {
                #[serde(skip_serializing_if = "Option::is_none")]
                pub before: Option<Value>,
                #[serde(skip_serializing_if = "Option::is_none")]
                pub after: Option<Value>,
            }

            impl Event for $kind {
                const NAME: &'static str = $name;
            }
        )*

        /// Mọi event module `auth` phát.
        pub(crate) fn all() -> Vec<EventInfo> {
            vec![$($info),*]
        }
    };
}

events! {
    USER_CREATED, UserCreated, "auth.user_created", "A user account was created.";
    USER_UPDATED, UserUpdated, "auth.user_updated", "A user account was renamed, disabled or enabled.";
    PASSWORD_CHANGED, PasswordChanged, "auth.password_changed", "A user's password was changed; their sessions were ended.";
    SIGNED_IN, SignedIn, "auth.signed_in", "A user signed in.";
    SIGNED_OUT, SignedOut, "auth.signed_out", "A user signed out of one session.";
    REFRESH_TOKEN_REUSED, RefreshTokenReused, "auth.refresh_token_reused", "A used refresh token was sent again; the session was ended in case it was stolen.";
    ROLE_CREATED, RoleCreated, "auth.role_created", "A role was created.";
    ROLE_UPDATED, RoleUpdated, "auth.role_updated", "A role's name, description or permissions changed.";
    ROLE_DELETED, RoleDeleted, "auth.role_deleted", "A role was deleted.";
    USER_ROLES_CHANGED, UserRolesChanged, "auth.user_roles_changed", "The roles of a user changed.";
    API_KEY_CREATED, ApiKeyCreated, "auth.api_key_created", "An API key was created.";
    API_KEY_REVOKED, ApiKeyRevoked, "auth.api_key_revoked", "An API key was revoked.";
    MFA_ENABLED, MfaEnabled, "auth.mfa_enabled", "A user turned on two-step sign-in.";
    MFA_DISABLED, MfaDisabled, "auth.mfa_disabled", "Two-step sign-in was turned off, by the user or by an administrator.";
    RECOVERY_CODES_REPLACED, RecoveryCodesReplaced, "auth.recovery_codes_replaced", "A user made new recovery codes.";
}

/// Chủ thể (subject) của event về người dùng `id`.
pub(crate) fn user_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("auth.user:{id}"))
}
