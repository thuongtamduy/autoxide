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
    USER_DELETED, UserDeleted, "auth.user_deleted", "A user account was deleted; `before.id` is its id. Modules that keep data about users delete theirs when they receive it.";
    PASSWORD_CHANGED, PasswordChanged, "auth.password_changed", "A user's password was changed; their sessions were ended.";
    SIGNED_IN, SignedIn, "auth.signed_in", "A user signed in.";
    SIGNED_OUT, SignedOut, "auth.signed_out", "A user signed out of one session; `after.session_id` names it when it was ended from the session list.";
    SESSIONS_REVOKED, SessionsRevoked, "auth.sessions_revoked", "Several sessions of a user were ended at once: by the user (`after.kept` is the session they kept) or by an administrator; `after.count` is how many.";
    REFRESH_TOKEN_REUSED, RefreshTokenReused, "auth.refresh_token_reused", "A used refresh token was sent again; the session was ended in case it was stolen.";
    ROLE_CREATED, RoleCreated, "auth.role_created", "A role was created.";
    ROLE_UPDATED, RoleUpdated, "auth.role_updated", "A role's name, description or permissions changed.";
    ROLE_DELETED, RoleDeleted, "auth.role_deleted", "A role was deleted.";
    USER_ROLES_CHANGED, UserRolesChanged, "auth.user_roles_changed", "The roles of a user changed.";
    API_KEY_CREATED, ApiKeyCreated, "auth.api_key_created", "An API key was created.";
    API_KEY_REVOKED, ApiKeyRevoked, "auth.api_key_revoked", "An API key was revoked.";
    MFA_ENABLED, MfaEnabled, "auth.mfa_enabled", "A user turned on two-step sign-in.";
    MFA_DISABLED, MfaDisabled, "auth.mfa_disabled", "Two-step sign-in was turned off, by the user or by an administrator.";
    PASSWORD_RESET_REQUESTED, PasswordResetRequested, "auth.password_reset_requested", "A password-reset mail was sent to a user.";
    EMAIL_VERIFICATION_REQUESTED, EmailVerificationRequested, "auth.email_verification_requested", "A mail to confirm an email address was sent; `after.email` is that address.";
    EMAIL_VERIFIED, EmailVerified, "auth.email_verified", "A user confirmed an email address; when it was a new one, it replaced the old (`before.email`).";
    PASSKEY_ADDED, PasskeyAdded, "auth.passkey_added", "A user added a passkey; `after.name` is its name.";
    EMAIL_CHANGE_REPORTED, EmailChangeReported, "auth.email_change_reported", "The owner of an account reported an email change they did not make (from the mail sent to the old address): the account was signed out everywhere, its password, passkeys, API keys and two-step sign-in removed, and the old address restored when free (`before.email`, `after.email`).";
    EMAIL_CHANGE_CANCELLED, EmailChangeCancelled, "auth.email_change_cancelled", "A user cancelled a pending email change; the link sent to the new address (`before.email`) stopped working.";
    PASSKEY_REMOVED, PasskeyRemoved, "auth.passkey_removed", "A passkey was removed; `before.name` is its name, `after.by_administrator` is `true` when an administrator (or the server operator) removed it.";
    RECOVERY_CODES_REPLACED, RecoveryCodesReplaced, "auth.recovery_codes_replaced", "A user made new recovery codes.";
}

/// Chủ thể (subject) của event về người dùng `id`.
pub(crate) fn user_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("auth.user:{id}"))
}

#[cfg(test)]
mod tests {
    /// Tên event xoá người dùng là hợp đồng với mọi module (kernel giữ hằng của nó).
    #[test]
    fn ten_event_xoa_nguoi_dung_khop_kernel() {
        assert_eq!(super::USER_DELETED.name(), autoxide_kernel::privacy::USER_DELETED);
    }
}
