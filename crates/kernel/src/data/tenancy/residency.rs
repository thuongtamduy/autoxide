//! Per-tenant data residency (step 37, D-360): a tenant can require its data to stay in one country
//! (`kernel_tenants.data_residency`, e.g. `VN`; Law on Cybersecurity 116/2025/QH15 and its implementing decree; to be
//! checked against the original texts); each placement records the country where its server is
//! (`kernel_placements.country_code`).
//!
//! - Setting a requirement for a tenant whose current placement has an unknown or different country → rejected
//!   (`tenant_residency_violation`), nothing is written.
//! - Moving a tenant to a placement that does not meet it → the move is blocked with the same error code
//!   (`moves.rs`).
//! - Removing the requirement (`None`) is always allowed.
//!
//! Both columns are control plane; country codes are ISO 3166-1 alpha-2, uppercase.

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};

use super::admin::{placement, tenant};
use super::{DatabaseRegistry, Placements, TenantId, errors};
use crate::error::{Error, FieldError};

/// Where a tenant's data is stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Residency {
    pub tenant: TenantId,
    /// Country where the tenant requires its data to stay (ISO 3166-1 alpha-2), `None` when not required.
    pub required: Option<String>,
    /// Name of the placement that holds the tenant's data.
    pub placement: String,
    /// Country of that placement; `None` when nobody has recorded it.
    pub placement_country: Option<String>,
}

impl Residency {
    /// Meets the requirement: none is required, or the placement is in exactly that country.
    pub fn compliant(&self) -> bool {
        match &self.required {
            None => true,
            Some(required) => self.placement_country.as_deref() == Some(required.as_str()),
        }
    }
}

/// Normalizes a country code: two letters → uppercase; a wrong form → a validation error on field `field`.
pub fn parse_country(raw: &str, field: &str) -> Result<String, Error> {
    let code = raw.trim().to_ascii_uppercase();
    if code.len() == 2 && code.bytes().all(|byte| byte.is_ascii_uppercase()) {
        Ok(code)
    } else {
        Err(Error::validation(vec![FieldError::new(
            field,
            "invalid",
            "must be a two-letter ISO 3166-1 country code, e.g. `VN`",
        )]))
    }
}

/// Whether placement `target` meets the tenant's requirement `required`; if not → an error.
pub(super) fn check_target(
    required: Option<&str>,
    target_country: Option<&str>,
) -> Result<(), Error> {
    match required {
        Some(required) if target_country != Some(required) => {
            Err(errors::tenant_residency_violation(required))
        }
        _ => Ok(()),
    }
}

impl DatabaseRegistry {
    async fn residency_of(&self, row: tenant::Model) -> Result<Residency, Error> {
        let placement = placement::Entity::find_by_id(row.placement_id)
            .one(self.control().read())
            .await?
            .ok_or_else(|| Error::internal("a tenant points to a missing placement"))?;
        Ok(Residency {
            tenant: TenantId::from_uuid(row.id),
            required: row.data_residency,
            placement: placement.name,
            placement_country: placement.country_code,
        })
    }

    /// Where the data of tenant `tenant` is stored.
    pub async fn residency(&self, tenant: TenantId) -> Result<Residency, Error> {
        let row = tenant::Entity::find_by_id(tenant.as_uuid())
            .one(self.control().read())
            .await?
            .ok_or_else(|| errors::tenant_not_found(&tenant.to_string()))?;
        self.residency_of(row).await
    }

    /// Sets (`Some("VN")`) or removes (`None`) the storage-location requirement of tenant `tenant`. If the
    /// current placement does not meet it → `tenant_residency_violation` (409), nothing is written. The
    /// caller checks permission (platform staff only).
    pub async fn set_residency(
        &self,
        tenant: TenantId,
        country: Option<&str>,
    ) -> Result<Residency, Error> {
        let country = country.map(|raw| parse_country(raw, "data_residency")).transpose()?;
        let current = self.residency(tenant).await?;
        check_target(country.as_deref(), current.placement_country.as_deref())?;
        tenant::Entity::update_many()
            .set(tenant::ActiveModel { data_residency: Set(country.clone()), ..Default::default() })
            .filter(tenant::Column::Id.eq(tenant.as_uuid()))
            .exec(self.control().write())
            .await?;
        Ok(Residency { required: country, ..current })
    }

    /// The storage location of every tenant that has a requirement (for `autoxide doctor`), by id.
    pub async fn residencies(&self) -> Result<Vec<Residency>, Error> {
        let rows = tenant::Entity::find()
            .filter(tenant::Column::DataResidency.is_not_null())
            .filter(tenant::Column::Status.ne(super::TenantStatus::Deleted.as_str()))
            .order_by_asc(tenant::Column::Id)
            .all(self.control().read())
            .await?;
        let mut all = Vec::with_capacity(rows.len());
        for row in rows {
            all.push(self.residency_of(row).await?);
        }
        Ok(all)
    }
}

impl Placements {
    /// Records the country of placement `name`'s server (`None` = unknown). If a tenant there has a storage
    /// requirement different from the new country → `tenant_residency_violation`, nothing is written.
    pub async fn set_country(&self, name: &str, country: Option<&str>) -> Result<(), Error> {
        let country = country.map(|raw| parse_country(raw, "country")).transpose()?;
        let control = self.registry().control();
        let row = placement::Entity::find()
            .filter(placement::Column::Name.eq(name))
            .one(control.read())
            .await?
            .ok_or_else(|| errors::tenant_placement_not_found(name))?;
        let tenants = tenant::Entity::find()
            .filter(tenant::Column::PlacementId.eq(row.id))
            .filter(tenant::Column::DataResidency.is_not_null())
            .all(control.read())
            .await?;
        for found in &tenants {
            check_target(found.data_residency.as_deref(), country.as_deref())?;
        }
        placement::Entity::update_many()
            .set(placement::ActiveModel { country_code: Set(country), ..Default::default() })
            .filter(placement::Column::Id.eq(row.id))
            .exec(control.write())
            .await?;
        Ok(())
    }
}

// --- Migration ------------------------------------------------------------------------------

/// `kernel_placements.country_code`, `kernel_tenants.data_residency` (TEXT NULL). Does not edit the migration
/// of step 33.
pub(crate) struct AddDataResidency;

impl crate::migration::prelude::MigrationName for AddDataResidency {
    fn name(&self) -> &str {
        "m0021_data_residency"
    }
}

#[async_trait::async_trait]
impl crate::migration::prelude::MigrationTrait for AddDataResidency {
    async fn up(
        &self,
        manager: &crate::migration::prelude::SchemaManager,
    ) -> Result<(), crate::migration::prelude::DbErr> {
        use crate::migration::prelude::*;
        manager
            .alter_table(
                Table::alter()
                    .table("kernel_placements")
                    .add_column(schema::string_len_null("country_code", 2))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table("kernel_tenants")
                    .add_column(schema::string_len_null("data_residency", 2))
                    .to_owned(),
            )
            .await
    }

    async fn down(
        &self,
        manager: &crate::migration::prelude::SchemaManager,
    ) -> Result<(), crate::migration::prelude::DbErr> {
        use crate::migration::prelude::*;
        manager
            .alter_table(
                Table::alter().table("kernel_tenants").drop_column("data_residency").to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter().table("kernel_placements").drop_column("country_code").to_owned(),
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ma_nuoc_chuan_hoa_va_kiem_noi_luu() {
        assert_eq!(parse_country(" vn ", "x").unwrap(), "VN");
        assert!(parse_country("VNM", "x").is_err());
        assert!(parse_country("V1", "x").is_err());
        assert!(check_target(None, None).is_ok());
        assert!(check_target(Some("VN"), Some("VN")).is_ok());
        let error = check_target(Some("VN"), None).unwrap_err();
        assert_eq!(error.code(), "tenant_residency_violation");
        assert!(check_target(Some("VN"), Some("SG")).is_err());
        let residency = Residency {
            tenant: crate::tenancy::DEFAULT_TENANT,
            required: Some("VN".into()),
            placement: "main".into(),
            placement_country: Some("SG".into()),
        };
        assert!(!residency.compliant());
        assert!(Residency { required: None, ..residency.clone() }.compliant());
    }
}
