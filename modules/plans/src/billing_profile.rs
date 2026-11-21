//! Hồ sơ xuất hoá đơn của khách (bước 35 mục 4): tên pháp lý, mã số thuế, email nhận hoá đơn, số
//! điện thoại, địa chỉ. Hoá đơn thu phí SaaS (bước 36, `invoices`) lấy bên mua từ đây; mail nhắc
//! thanh toán gửi về email này khi thuê bao không đặt `billing_email` riêng.
//!
//! Bảng `plans_billing_profile` ở database điều khiển như mọi bảng của `plans`; một dòng mỗi khách.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_address::api::{Address, AddressInput};
use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::{TenantContext, TenantId};
use axum::extract::{Path, State};
use sea_orm::sea_query::OnConflict;
use sea_orm::{EntityTrait, Set};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::{Validate, ValidateEmail};

use crate::events;
use crate::http::operator;
use crate::permissions::{BILLING_MANAGE, BILLING_READ, SUBSCRIPTIONS_READ};
use crate::service::{PlansService, now};

pub(crate) mod entity {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "plans_billing_profile")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        pub legal_name: String,
        pub tax_code: Option<String>,
        pub email: String,
        pub phone: Option<String>,
        pub address_country: Option<String>,
        pub address_province_code: Option<String>,
        pub address_province_name: Option<String>,
        pub address_commune_code: Option<String>,
        pub address_commune_name: Option<String>,
        pub address_line: Option<String>,
        pub updated_by: Option<Uuid>,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) fn routes() -> OpenApiRouter<Arc<PlansService>> {
    OpenApiRouter::new().routes(routes!(get_profile, put_profile)).routes(routes!(tenant_profile))
}

/// Who the platform invoices: your tenant's legal details.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct BillingProfileResponse {
    tenant_id: Uuid,
    /// Legal name of the business or person invoiced.
    legal_name: String,
    /// Tax code (Vietnam: 10 digits, or 10 digits, a dash and 3 digits for a branch).
    tax_code: Option<String>,
    /// Where invoices and billing reminders go.
    email: String,
    phone: Option<String>,
    address: Option<Address>,
    /// User who last changed it.
    updated_by: Option<Uuid>,
    updated_at: Timestamp,
}

impl From<entity::Model> for BillingProfileResponse {
    fn from(row: entity::Model) -> Self {
        let address = address_of(&row);
        BillingProfileResponse {
            tenant_id: row.tenant_id,
            legal_name: row.legal_name,
            tax_code: row.tax_code,
            email: row.email,
            phone: row.phone,
            address,
            updated_by: row.updated_by,
            updated_at: row.updated_at,
        }
    }
}

/// The whole billing profile; it replaces the one there is.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BillingProfileRequest {
    /// Legal name, 1 to 200 characters.
    #[schema(example = "Cong ty TNHH Vi Du")]
    legal_name: String,
    /// Tax code: 10 digits, or 10 digits, `-` and 3 digits; leave out for a person without one.
    #[schema(example = "0101234567")]
    tax_code: Option<String>,
    /// Where invoices and billing reminders go.
    #[schema(example = "billing@example.com")]
    email: String,
    /// Up to 20 characters.
    phone: Option<String>,
    /// Address; names are filled in from the codes (`GET /api/v1/address/provinces`).
    address: Option<AddressInput>,
}

/// Dữ liệu đã kiểm.
#[derive(Debug, Clone)]
pub(crate) struct ProfileInput {
    pub legal_name: String,
    pub tax_code: Option<String>,
    pub email: String,
    pub phone: Option<String>,
    pub address: Option<AddressInput>,
}

impl From<BillingProfileRequest> for ProfileInput {
    fn from(input: BillingProfileRequest) -> Self {
        ProfileInput {
            legal_name: input.legal_name,
            tax_code: input.tax_code,
            email: input.email,
            phone: input.phone,
            address: input.address,
        }
    }
}

pub(crate) fn billing_profile_not_found() -> Error {
    Error::new(
        ErrorKind::NotFound,
        "plans.billing_profile_not_found",
        "The tenant has no billing profile yet; set one with `PUT /api/v1/plans/billing-profile`.",
    )
}

/// Mã số thuế Việt Nam: 10 chữ số, hay 10 chữ số + `-` + 3 chữ số (đơn vị phụ thuộc).
fn valid_tax_code(code: &str) -> bool {
    let bytes = code.as_bytes();
    let digits = |part: &[u8]| part.iter().all(u8::is_ascii_digit);
    match bytes.len() {
        10 => digits(bytes),
        14 => digits(&bytes[..10]) && bytes[10] == b'-' && digits(&bytes[11..]),
        _ => false,
    }
}

fn address_of(row: &entity::Model) -> Option<Address> {
    let (Some(country), Some(province_code), Some(commune_code)) =
        (&row.address_country, &row.address_province_code, &row.address_commune_code)
    else {
        return None;
    };
    // `Address` là `#[non_exhaustive]` (không dựng trực tiếp ngoài crate `address`): dựng qua serde.
    // Tên giữ như lúc ghi.
    serde_json::from_value(json!({
        "country": country,
        "province_code": province_code,
        "province_name": row.address_province_name.clone().unwrap_or_default(),
        "commune_code": commune_code,
        "commune_name": row.address_commune_name.clone().unwrap_or_default(),
        "line": row.address_line.clone().unwrap_or_default(),
    }))
    .ok()
}

impl PlansService {
    /// Hồ sơ xuất hoá đơn của khách `tenant`, nếu đã đặt.
    pub(crate) async fn billing_profile(
        &self,
        tenant: TenantId,
    ) -> Result<Option<entity::Model>, Error> {
        Ok(entity::Entity::find_by_id(tenant.as_uuid())
            .one(self.databases.control().read())
            .await?)
    }

    /// Đặt (thay) hồ sơ xuất hoá đơn của khách `tenant`; gom mọi lỗi của đầu vào báo một lần.
    pub(crate) async fn set_billing_profile(
        &self,
        actor: Actor,
        tenant: TenantId,
        input: ProfileInput,
    ) -> Result<entity::Model, Error> {
        let mut problems = Vec::new();
        let legal_name = input.legal_name.trim().to_owned();
        if legal_name.is_empty() || legal_name.chars().count() > 200 {
            problems.push(FieldError::new(
                "legal_name",
                "length",
                "must be 1 to 200 characters long",
            ));
        }
        let tax_code = input.tax_code.map(|code| code.trim().to_owned()).filter(|c| !c.is_empty());
        if tax_code.as_deref().is_some_and(|code| !valid_tax_code(code)) {
            problems.push(FieldError::new(
                "tax_code",
                "format",
                "must be 10 digits, or 10 digits, `-` and 3 digits",
            ));
        }
        let email = input.email.trim().to_owned();
        if !ValidateEmail::validate_email(&email) || email.len() > 254 {
            problems.push(FieldError::new("email", "email", "must be an email address"));
        }
        let phone = input.phone.map(|phone| phone.trim().to_owned()).filter(|p| !p.is_empty());
        if phone.as_deref().is_some_and(|phone| phone.chars().count() > 20) {
            problems.push(FieldError::new("phone", "length", "must be at most 20 characters long"));
        }
        // Kiểm địa chỉ (đọc database điều khiển) trước khi ghi.
        let address = match input.address {
            Some(address) => match &self.units {
                Some(units) => match units.validate(address, "address.").await {
                    Ok(address) => Some(address),
                    Err(fields) => {
                        problems.extend(fields);
                        None
                    }
                },
                None => {
                    problems.push(FieldError::new(
                        "address",
                        "unavailable",
                        "addresses cannot be checked on this server",
                    ));
                    None
                }
            },
            None => None,
        };
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let row = entity::ActiveModel {
            tenant_id: Set(tenant.as_uuid()),
            legal_name: Set(legal_name),
            tax_code: Set(tax_code),
            email: Set(email),
            phone: Set(phone),
            address_country: Set(address.as_ref().map(|a| a.country.clone())),
            address_province_code: Set(address.as_ref().map(|a| a.province_code.clone())),
            address_province_name: Set(address.as_ref().map(|a| a.province_name.clone())),
            address_commune_code: Set(address.as_ref().map(|a| a.commune_code.clone())),
            address_commune_name: Set(address.as_ref().map(|a| a.commune_name.clone())),
            address_line: Set(address.as_ref().map(|a| a.line.clone())),
            updated_by: Set(actor.user_id),
            updated_at: Set(now()),
        };
        let transaction = self.databases.control().begin_write().await?;
        entity::Entity::insert(row)
            .on_conflict(
                OnConflict::column(entity::Column::TenantId)
                    .update_columns([
                        entity::Column::LegalName,
                        entity::Column::TaxCode,
                        entity::Column::Email,
                        entity::Column::Phone,
                        entity::Column::AddressCountry,
                        entity::Column::AddressProvinceCode,
                        entity::Column::AddressProvinceName,
                        entity::Column::AddressCommuneCode,
                        entity::Column::AddressCommuneName,
                        entity::Column::AddressLine,
                        entity::Column::UpdatedBy,
                        entity::Column::UpdatedAt,
                    ])
                    .to_owned(),
            )
            .exec_without_returning(&transaction)
            .await?;
        // Event không mang dữ liệu cá nhân (email, số điện thoại): chỉ khách bị đổi.
        let event = events::BillingProfileUpdated(json!({ "tenant_id": tenant.as_uuid() }));
        self.record_event(&transaction, actor, tenant.as_uuid(), &event).await?;
        transaction.commit().await?;
        self.billing_profile(tenant)
            .await?
            .ok_or_else(|| Error::internal("billing profile vanished after it was written"))
    }
}

/// Get your tenant's billing profile.
///
/// Requires `plans.billing.read`.
#[utoipa::path(
    get,
    path = "/billing-profile",
    extensions(("x-permissions" = json!(["plans.billing.read"]))),
    operation_id = "get_billing_profile",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The billing profile.", body = Data<BillingProfileResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `plans.billing.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "None set yet; code `plans.billing_profile_not_found`.", body = Problem),
    )
)]
async fn get_profile(
    State(service): State<Arc<PlansService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<BillingProfileResponse>, Error> {
    principal.require(&BILLING_READ)?;
    let row = service.billing_profile(tenant.id()).await?.ok_or_else(billing_profile_not_found)?;
    Ok(Data { data: row.into() })
}

/// Set your tenant's billing profile.
///
/// Requires `plans.billing.manage`. The body replaces the whole profile. Invoices of the platform
/// name this buyer; billing reminders go to `email` unless the subscription has its own
/// `billing_email`.
#[utoipa::path(
    put,
    path = "/billing-profile",
    extensions(("x-permissions" = json!(["plans.billing.manage"]))),
    operation_id = "set_billing_profile",
    security(("bearer" = [])),
    request_body = BillingProfileRequest,
    responses(
        (status = 200, description = "The billing profile after the change.", body = Data<BillingProfileResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `plans.billing.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem is listed in `errors` (an unknown province or commune is under `address.`).", body = Problem),
    )
)]
async fn put_profile(
    State(service): State<Arc<PlansService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<BillingProfileRequest>,
) -> Result<Data<BillingProfileResponse>, Error> {
    principal.require(&BILLING_MANAGE)?;
    let row =
        service.set_billing_profile(Actor::from(&principal), tenant.id(), input.into()).await?;
    Ok(Data { data: row.into() })
}

/// Get a tenant's billing profile.
///
/// Platform staff with `plans.subscriptions.read`.
#[utoipa::path(
    get,
    path = "/tenants/{tenant_id}/billing-profile",
    extensions(("x-permissions" = json!(["plans.subscriptions.read"]))),
    operation_id = "get_tenant_billing_profile",
    security(("bearer" = [])),
    params(("tenant_id" = Uuid, Path, description = "Id of the tenant.")),
    responses(
        (status = 200, description = "The billing profile.", body = Data<BillingProfileResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.read` (`permission_denied`).", body = Problem),
        (status = 404, description = "None set yet; code `plans.billing_profile_not_found`.", body = Problem),
    )
)]
async fn tenant_profile(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(tenant_id): Path<Uuid>,
) -> Result<Data<BillingProfileResponse>, Error> {
    operator(&principal, &SUBSCRIPTIONS_READ)?;
    let row = service
        .billing_profile(TenantId::from_uuid(tenant_id))
        .await?
        .ok_or_else(billing_profile_not_found)?;
    Ok(Data { data: row.into() })
}

#[cfg(test)]
mod tests {
    use super::valid_tax_code;

    #[test]
    fn ma_so_thue() {
        assert!(valid_tax_code("0101234567"));
        assert!(valid_tax_code("0101234567-001"));
        assert!(!valid_tax_code("010123456"));
        assert!(!valid_tax_code("0101234567001"));
        assert!(!valid_tax_code("0101234567_001"));
        assert!(!valid_tax_code("01012345a7"));
    }
}
