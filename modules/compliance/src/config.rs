//! Cấu hình của module `compliance`.

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const KYC_CALLBACK_RETENTION_DAYS: Setting = Setting::new(
    "AUTOXIDE_COMPLIANCE_KYC_CALLBACK_RETENTION_DAYS",
    "Days the raw callbacks of identity providers are kept before they are deleted, 1 to 3650.",
)
.with_default("90");

pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<i64, Invalid> {
    reader.get_with(&KYC_CALLBACK_RETENTION_DAYS, |raw| parse_in_range(raw, 1i64, 3650))
}
