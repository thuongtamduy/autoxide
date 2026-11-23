//! Contract of the `address` module for other modules: the address types and the trait for looking up
//! administrative units. Other modules get the trait at startup:
//! `context.require::<dyn AdministrativeUnits>()?` (registered AFTER `address`).

use autoxide_kernel::error::{Error, FieldError};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A fake administrative catalog for other modules' tests (feature `testing`).
#[cfg(feature = "testing")]
pub mod testing;

/// A postal address in an administrative unit (Vietnam: province and commune, since 2025-07-01).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct Address {
    /// ISO 3166-1 alpha-2 country code.
    #[schema(example = "VN")]
    pub country: String,
    /// Code of the province (2 digits for Vietnam).
    #[schema(example = "01")]
    pub province_code: String,
    /// Name of the province when the address was written.
    pub province_name: String,
    /// Code of the commune, ward or special zone (5 digits for Vietnam).
    #[schema(example = "00004")]
    pub commune_code: String,
    /// Name of the commune when the address was written.
    pub commune_name: String,
    /// House number, street, village…
    #[schema(example = "12 Trang Tien")]
    pub line: String,
}

/// An address to check: codes only; names are filled in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct AddressInput {
    #[schema(example = "VN")]
    pub country: String,
    #[schema(example = "01")]
    pub province_code: String,
    #[schema(example = "00004")]
    pub commune_code: String,
    /// House number, street, village… (1 to 300 characters).
    pub line: String,
}

impl AddressInput {
    pub fn new(
        country: impl Into<String>,
        province_code: impl Into<String>,
        commune_code: impl Into<String>,
        line: impl Into<String>,
    ) -> Self {
        AddressInput {
            country: country.into(),
            province_code: province_code.into(),
            commune_code: commune_code.into(),
            line: line.into(),
        }
    }
}

/// An administrative unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct AdministrativeUnit {
    #[schema(example = "01")]
    pub code: String,
    #[schema(example = "Thanh pho Ha Noi")]
    pub name: String,
    /// `province`, `city` (centrally governed city), `commune`, `ward` or `special_zone`.
    #[schema(example = "city")]
    pub kind: String,
    /// 1: province level; 2: commune level.
    pub level: i32,
    /// Code of the province, for a commune.
    pub parent_code: Option<String>,
    /// First day in force (`YYYY-MM-DD`).
    #[schema(example = "2025-07-01")]
    pub valid_from: String,
    /// First day no longer in force; `null` while in force.
    pub valid_to: Option<String>,
}

/// Lookup of administrative units (published by the `address` module).
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait AdministrativeUnits: Send + Sync {
    /// Provinces of `country` in force on day `as_of` (`YYYY-MM-DD`; `None`: today), by code.
    async fn provinces(
        &self,
        country: &str,
        as_of: Option<&str>,
    ) -> Result<Vec<AdministrativeUnit>, Error>;
    /// Communes of province `province` (code), as above. A province that does not exist: `address.unit_not_found`.
    async fn communes(
        &self,
        country: &str,
        province: &str,
        as_of: Option<&str>,
    ) -> Result<Vec<AdministrativeUnit>, Error>;
    /// Validates an address and fills in names; errors get the prefix `field` (e.g. `address.` → `address.commune_code`).
    async fn validate(&self, input: AddressInput, field: &str) -> Result<Address, Vec<FieldError>>;
}
