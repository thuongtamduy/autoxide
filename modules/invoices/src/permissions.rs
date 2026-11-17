//! Permission của module `invoices`, theo phạm vi nút.

use autoxide_kernel::access::{Permission, Risk};

pub(crate) const READ: Permission = Permission::new(
    "invoices.read",
    "See the documents, series and mandates of the units in your scope.",
)
.scoped();

pub(crate) const WRITE: Permission = Permission::new(
    "invoices.write",
    "Write, change and delete draft documents for the units in your scope.",
)
.scoped();

pub(crate) const ISSUE: Permission = Permission::new(
    "invoices.issue",
    "Issue documents (gives them their number and sends e-invoices) for the units in your scope.",
)
.scoped()
.with_risk(Risk::Money);

pub(crate) const CORRECT: Permission = Permission::new(
    "invoices.correct",
    "Cancel, adjust and replace issued documents of the units in your scope.",
)
.scoped()
.with_risk(Risk::Money);

pub(crate) const SERIES_MANAGE: Permission = Permission::new(
    "invoices.series_manage",
    "Create and retire document series (symbols) for the units in your scope.",
)
.scoped();

pub(crate) const MANDATES_MANAGE: Permission = Permission::new(
    "invoices.mandates_manage",
    "Record and revoke mandates to issue invoices on behalf of sellers in your scope.",
)
.scoped()
.with_risk(Risk::Money);

pub(crate) const TEMPLATES_MANAGE: Permission = Permission::new(
    "invoices.templates_manage",
    "Write and change the print templates of documents for the units in your scope.",
)
.scoped();

pub(crate) const EXPORT: Permission = Permission::new(
    "invoices.export",
    "Export the general journal and documents of a period for accounting software.",
)
.with_risk(Risk::SensitiveData);

pub(crate) const ARCHIVE: Permission = Permission::new(
    "invoices.archive",
    "Make an archive copy of the tenant's accounting records (journal and documents).",
)
.with_risk(Risk::SensitiveData);

pub(crate) fn all() -> Vec<Permission> {
    vec![
        READ,
        WRITE,
        ISSUE,
        CORRECT,
        SERIES_MANAGE,
        MANDATES_MANAGE,
        TEMPLATES_MANAGE,
        EXPORT,
        ARCHIVE,
    ]
}
