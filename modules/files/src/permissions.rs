//! Permission của module `files`.

use autoxide_kernel::access::{Permission, Risk};

pub(crate) const FILES_UPLOAD: Permission = Permission::new(
    "files.upload",
    "Upload files, and see, download and delete the files you uploaded.",
);

pub(crate) const FILES_MANAGE: Permission =
    Permission::new("files.manage", "See, download and delete the files of every user.")
        .with_risk(Risk::SensitiveData);

pub(crate) fn all() -> Vec<Permission> {
    vec![FILES_UPLOAD, FILES_MANAGE]
}
