//! Lỗi của service gói, thuê bao, ngoại lệ (mã ổn định `plans.*`).

use super::*;

pub(super) fn code_taken(error: sea_orm::DbErr, code: &str) -> Error {
    if matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) {
        Error::conflict("plans.plan_code_taken", "The plan code `{code}` is already used.")
            .with_param("code", code.to_owned())
    } else {
        error.into()
    }
}

pub(super) fn one_of(field: &str, values: &[&str]) -> FieldError {
    let listed: Vec<String> = values.iter().map(|value| format!("`{value}`")).collect();
    FieldError::new(field, "one_of", "must be one of {values}")
        .with_param("values", listed.join(", "))
}

pub(super) fn range(field: &str, min: i64, max: i64) -> FieldError {
    FieldError::new(field, "range", "must be between {min} and {max}")
        .with_param("min", min)
        .with_param("max", max)
}

pub(crate) fn plan_not_found(id: Uuid) -> Error {
    Error::not_found("plans.plan_not_found", "There is no plan `{id}`.").with_param("id", id)
}

pub(crate) fn plan_not_active(code: &str) -> Error {
    Error::conflict("plans.plan_not_active", "The plan `{code}` is no longer sold.")
        .with_param("code", code.to_owned())
}

pub(super) fn plan_in_use() -> Error {
    Error::conflict(
        "plans.plan_in_use",
        "Tenants use this plan: it can only be retired, not deleted or turned into another kind.",
    )
}

pub(crate) fn price_not_found(id: Uuid) -> Error {
    Error::not_found("plans.price_not_found", "There is no price `{id}` of this plan.")
        .with_param("id", id)
}

pub(crate) fn subscription_not_found() -> Error {
    Error::not_found("plans.subscription_not_found", "This tenant has no subscription.")
}

pub(crate) fn invalid_transition(status: &str) -> Error {
    Error::conflict(
        "plans.invalid_transition",
        "The subscription is `{status}`, so this cannot be done.",
    )
    .with_param("status", status.to_owned())
}

pub(crate) fn addon_not_allowed(code: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "plans.addon_not_allowed",
        "`{code}` is not the right kind of plan here: a subscription has one base plan and any number of add-ons.",
    )
    .with_param("code", code.to_owned())
}

pub(super) fn override_not_found(id: Uuid) -> Error {
    Error::not_found("plans.override_not_found", "This tenant has no override `{id}`.")
        .with_param("id", id)
}

pub(crate) fn platform_exempt() -> Error {
    Error::new(
        ErrorKind::Validation,
        "plans.platform_exempt",
        "The platform tenant is never limited by a plan: it has no subscription or overrides.",
    )
}
