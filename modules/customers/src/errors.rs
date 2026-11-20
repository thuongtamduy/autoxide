//! Lỗi của module `customers`. `code` là hợp đồng với client và là i18n key (bản dịch ở
//! `locales/vi.json`); câu tiếng Anh ở đây là bản mặc định.

use autoxide_kernel::error::{Error, ErrorKind};
use uuid::Uuid;

pub(crate) fn profile_not_found() -> Error {
    Error::not_found("customers.profile_not_found", "No customer profile with this id.")
}

pub(crate) fn phone_taken(existing: Uuid) -> Error {
    Error::conflict(
        "customers.phone_taken",
        "This phone number already has a customer profile ({id}); open that profile instead.",
    )
    .with_param("id", existing)
}

pub(crate) fn group_not_found() -> Error {
    Error::not_found("customers.group_not_found", "No customer group with this id.")
}

pub(crate) fn group_name_taken(name: &str) -> Error {
    Error::conflict(
        "customers.group_name_taken",
        "A customer group named {name} already exists; choose another name.",
    )
    .with_param("name", name)
}

pub(crate) fn group_not_manual() -> Error {
    Error::new(
        ErrorKind::Conflict,
        "customers.group_not_manual",
        "Members of a rule group follow its rule; change the rule instead of adding or removing \
         members by hand.",
    )
}

pub(crate) fn busy() -> Error {
    Error::unavailable(
        "customers.busy",
        "Another request changed this customer profile at the same time; try again in a moment.",
    )
}
