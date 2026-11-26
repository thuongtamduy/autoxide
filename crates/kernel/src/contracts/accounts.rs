//! Events about user accounts that other modules react to without depending on the module that owns accounts
//! (`auth`), like [`crate::privacy::USER_DELETED`].
//!
//! [`ACCESS_REVOKED`]: every way into an account was cut — the account was locked out (a stranger may have had it,
//! e.g. the owner said "this was not me", or the email changed through SCIM), recovered by email, had every session
//! revoked with a password change, was disabled, or was deleted. Something the account set up to act on its behalf
//! later (an automation rule, a token) must stop until a person checks it (D-20261125-durable-grants,
//! D-20261126-automation-owner-revoked).

use uuid::Uuid;

use crate::event::Delivered;

/// Name of the event emitted when every way into a user account was cut. `payload.after.user_id` is the
/// account's id and `payload.after.reason` says why (`locked_out`, `recovered`, `sessions_revoked`, `disabled`,
/// `deleted`).
pub const ACCESS_REVOKED: &str = "auth.access_revoked";

/// User id of an [`ACCESS_REVOKED`] event; `None` if the event is different or has the wrong shape.
pub fn revoked_user(event: &Delivered) -> Option<Uuid> {
    if event.name != ACCESS_REVOKED {
        return None;
    }
    event.payload.get("after")?.get("user_id")?.as_str()?.parse().ok()
}
