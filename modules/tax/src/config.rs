//! Cấu hình của module `tax`.

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const EXPIRY_WARNING_DAYS: Setting = Setting::new(
    "AUTOXIDE_TAX_EXPIRY_WARNING_DAYS",
    "`autoxide doctor` warns when a tax rate ends within this many days and no rate follows it for \
     the same category (import a newer tax data pack), 1 to 365.",
)
.with_default("60");

pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<i64, Invalid> {
    reader.get_with(&EXPIRY_WARNING_DAYS, |raw| parse_in_range(raw, 1i64, 365))
}
