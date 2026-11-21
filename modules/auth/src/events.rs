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
    SIGNED_IN, SignedIn, "auth.signed_in", "A user signed in; `after.method` says how (`password`, `password_and_mfa`, `passkey`, `email_link`, `oidc`…).";
    SIGNED_OUT, SignedOut, "auth.signed_out", "A user signed out of one session; `after.session_id` names it when it was ended from the session list.";
    SESSIONS_REVOKED, SessionsRevoked, "auth.sessions_revoked", "Several sessions of a user were ended at once: by the user (`after.kept` is the session they kept) or by an administrator; `after.count` is how many.";
    REFRESH_TOKEN_REUSED, RefreshTokenReused, "auth.refresh_token_reused", "A used refresh token was sent again; the session was ended in case it was stolen.";
    ROLE_CREATED, RoleCreated, "auth.role_created", "A role was created.";
    ROLE_UPDATED, RoleUpdated, "auth.role_updated", "A role's name, description or permissions changed.";
    ROLE_DELETED, RoleDeleted, "auth.role_deleted", "A role was deleted.";
    USER_ROLES_CHANGED, UserRolesChanged, "auth.user_roles_changed", "The roles of a user changed.";
    MEMBERSHIPS_CHANGED, MembershipsChanged, "auth.memberships_changed", "The organization units a user works in, or their roles there, changed.";
    API_KEY_CREATED, ApiKeyCreated, "auth.api_key_created", "An API key was created.";
    API_KEY_REVOKED, ApiKeyRevoked, "auth.api_key_revoked", "An API key was revoked.";
    GUARDRAIL_SET, GuardrailSet, "auth.guardrail_set", "This tenant set or changed the most permissions a tenant below it can use (`after`: tenant, permissions).";
    GUARDRAIL_REMOVED, GuardrailRemoved, "auth.guardrail_removed", "This tenant removed the guardrail it had set for a tenant below it (`before.tenant_id`).";
    API_KEY_APPROVED, ApiKeyApproved, "auth.api_key_approved", "An API key that waited for approval was approved and works from now on (`after`: owner, name, prefix).";
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
    LOGIN_LINK_USED, LoginLinkUsed, "auth.login_link_used", "A user signed in with a link sent by email (two-step sign-in, when on, still followed).";
    INVITATION_CREATED, InvitationCreated, "auth.invitation_created", "Someone was invited by email to work at a unit with given roles (`after`: email, unit, roles, expiry).";
    INVITATION_ACCEPTED, InvitationAccepted, "auth.invitation_accepted", "An invitation was accepted: the person (`after.user_id`, created when new) got its roles at its unit.";
    INVITATION_REVOKED, InvitationRevoked, "auth.invitation_revoked", "An invitation was revoked; its link stopped working.";
    SUPPORT_ACCESS_GRANTED, SupportAccessGranted, "auth.support_access_granted", "Platform staff were granted time-limited access to this tenant's data (`after`: staff account, role, basis, reason, legal reference, expiry).";
    SUPPORT_ACCESS_STARTED, SupportAccessStarted, "auth.support_access_started", "Platform staff entered this tenant's data for the first time under a support access grant.";
    SUPPORT_ACCESS_REVOKED, SupportAccessRevoked, "auth.support_access_revoked", "A support access grant was revoked (`after.revoked_by_tenant` is `true` when the tenant revoked it).";
    DELEGATION_GRANTED, DelegationGranted, "auth.delegation_granted", "This tenant let holders of one of its roles work in a child tenant (`after`: child tenant, role, unit, expiry, reason).";
    DELEGATION_REVOKED, DelegationRevoked, "auth.delegation_revoked", "A delegation to a child tenant was revoked; holders of the role lose access at their next request.";
    IDENTITY_LINKED, IdentityLinked, "auth.identity_linked", "A sign-in with an outside provider (Google, Microsoft, GitHub, OpenID Connect) was linked to an account (`after`: provider id and slug, identity id, `reason` `verified_email`, `signup` or `user`).";
    IDENTITY_UNLINKED, IdentityUnlinked, "auth.identity_unlinked", "A sign-in with an outside provider was removed from an account (`before`: provider id, identity id).";
    OIDC_PROVIDER_CREATED, OidcProviderCreated, "auth.oidc_provider_created", "A sign-in provider was added to the tenant (`after`: slug, kind, name, issuer; never the client secret).";
    OIDC_PROVIDER_UPDATED, OidcProviderUpdated, "auth.oidc_provider_updated", "A sign-in provider of the tenant was changed (`before` / `after` without the client secret; `after.secret_changed`).";
    SCIM_TOKEN_CREATED, ScimTokenCreated, "auth.scim_token_created", "A SCIM token was created for the tenant's identity provider (`after`: id, name, prefix; never the token).";
    SCIM_TOKEN_REVOKED, ScimTokenRevoked, "auth.scim_token_revoked", "A SCIM token was revoked; the identity provider using it is refused from now on.";
    SCIM_GROUP_CREATED, ScimGroupCreated, "auth.scim_group_created", "The identity provider created a SCIM group (`after`: id, name, member ids).";
    SCIM_GROUP_UPDATED, ScimGroupUpdated, "auth.scim_group_updated", "The identity provider renamed a SCIM group or changed its members (`after`: name, `added`, `removed` user ids).";
    SCIM_GROUP_DELETED, ScimGroupDeleted, "auth.scim_group_deleted", "The identity provider deleted a SCIM group; its mapping rules went with it.";
    SCIM_GROUP_MAPPED, ScimGroupMapped, "auth.scim_group_mapped", "A rule now gives the members of a SCIM group a role at a unit (`after`: rule id, group, unit, role).";
    SCIM_GROUP_UNMAPPED, ScimGroupUnmapped, "auth.scim_group_unmapped", "A rule from a SCIM group to a role at a unit was removed (`before`: rule id, group, unit, role).";
    OIDC_PROVIDER_DELETED, OidcProviderDeleted, "auth.oidc_provider_deleted", "A sign-in provider was removed from the tenant, with every account link to it.";
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
