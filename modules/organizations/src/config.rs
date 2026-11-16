//! Cấu hình của module `organizations`, tiền tố `AUTOXIDE_ORGANIZATIONS_`.

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const MAX_DEPTH: Setting = Setting::new(
    "AUTOXIDE_ORGANIZATIONS_MAX_DEPTH",
    "Most levels below the root of a tenant's organization tree (2 to 32).",
)
.with_default("8");

#[derive(Debug, Clone, Copy)]
pub(crate) struct OrganizationsConfig {
    pub max_depth: u8,
}

impl OrganizationsConfig {
    pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<Self, Invalid> {
        let max_depth = reader.get_with(&MAX_DEPTH, |raw| parse_in_range(raw, 2u8, 32))?;
        Ok(OrganizationsConfig { max_depth })
    }
}
