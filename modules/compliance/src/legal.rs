//! Hồ sơ pháp lý (chủ sở hữu website) theo khách và theo nút người bán (bước 37, D-357).
//!
//! Một nút một hồ sơ; nút gốc (`root_of(tenant)`) là hồ sơ của cả khách. Website, trang công khai
//! thông tin chủ sở hữu và bộ hồ sơ Bộ Công Thương dùng hồ sơ của nút gần nhất (chính nút của
//! website, rồi các nút cấp trên). Địa chỉ kiểm bằng `autoxide_address::api::AdministrativeUnits`.
//!
//! Căn cứ (cần đối chiếu văn bản gốc / người có chuyên môn pháp lý): NĐ 52/2013/NĐ-CP sửa bởi NĐ
//! 85/2021/NĐ-CP — thông tin chủ sở hữu phải công khai trên website thương mại điện tử.

use autoxide_address::api::{Address, AddressInput};
use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{OrgId, OrgTree, ScopeExt, TenantContext, root_of, site_not_found};
use chrono::{DateTime, NaiveDate, Utc};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{kyc_profile, legal_profile};
use crate::events;
use crate::permissions::{LEGAL_MANAGE, LEGAL_READ};
use crate::service::{ComplianceService, now};

pub(crate) const ENTITY_TYPES: [&str; 3] = ["individual", "household_business", "organization"];

pub(crate) fn profile_not_found() -> Error {
    Error::not_found("compliance.profile_not_found", "There is no such legal profile.")
}

fn profile_exists(org: Uuid) -> Error {
    Error::conflict(
        "compliance.profile_exists",
        "The organization unit `{org_id}` already has a legal profile; change that one.",
    )
    .with_param("org_id", org)
}

/// Mã số thuế Việt Nam: 10 chữ số, 10 chữ số + `-` + 3 chữ số (đơn vị phụ thuộc), hay 12 chữ số (cá
/// nhân dùng số định danh cá nhân làm mã số thuế từ 01/7/2025 — cần đối chiếu).
pub(crate) fn valid_tax_code(code: &str) -> bool {
    let bytes = code.as_bytes();
    let digits = |part: &[u8]| part.iter().all(u8::is_ascii_digit);
    match bytes.len() {
        10 | 12 => digits(bytes),
        14 => digits(&bytes[..10]) && bytes[10] == b'-' && digits(&bytes[11..]),
        _ => false,
    }
}

/// Các trường của hồ sơ. Khi tạo: `None` = không có. Khi sửa: `None` giữ nguyên, chuỗi rỗng xoá
/// (trừ `entity_type`, `legal_name` không xoá được).
#[derive(Debug, Clone, Default)]
pub(crate) struct ProfileFields {
    pub entity_type: Option<String>,
    pub legal_name: Option<String>,
    pub trade_name: Option<String>,
    pub tax_code: Option<String>,
    pub registration_number: Option<String>,
    pub registration_issued_on: Option<String>,
    pub registration_issued_by: Option<String>,
    /// `Some(None)`: bỏ địa chỉ.
    pub address: Option<Option<AddressInput>>,
    pub representative_name: Option<String>,
    pub representative_title: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    /// `Some(None)`: bỏ liên kết.
    pub kyc_profile_id: Option<Option<Uuid>>,
}

/// Chuỗi đã cắt khoảng trắng; rỗng = `None`.
fn clean(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// Đặt `target` theo `input` (giữ nguyên khi `None`), kiểm độ dài.
fn text(
    target: &mut Option<String>,
    input: Option<String>,
    field: &str,
    max: usize,
    problems: &mut Vec<FieldError>,
) {
    if let Some(value) = input {
        let value = clean(&value);
        if value.as_ref().is_some_and(|value| value.chars().count() > max) {
            problems.push(
                FieldError::new(field, "length", "must be at most {max} characters long")
                    .with_param("max", max),
            );
        }
        *target = value;
    }
}

/// Địa chỉ đã lưu (JSON) của hồ sơ.
pub(crate) fn address_of(row: &legal_profile::Model) -> Option<Address> {
    row.address.as_deref().and_then(|text| serde_json::from_str(text).ok())
}

/// Địa chỉ một dòng: số nhà, xã, tỉnh.
pub(crate) fn address_line(address: &Address) -> String {
    [address.line.as_str(), address.commune_name.as_str(), address.province_name.as_str()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

impl ComplianceService {
    /// Kiểm và áp `input` lên `row` (gom mọi lỗi). Gọi mạng / database điều khiển (kiểm địa chỉ)
    /// NGOÀI transaction ghi.
    async fn apply_profile(
        &self,
        tenant: &TenantContext,
        row: &mut legal_profile::Model,
        input: ProfileFields,
    ) -> Result<(), Error> {
        let mut problems = Vec::new();
        if let Some(kind) = input.entity_type {
            if ENTITY_TYPES.contains(&kind.as_str()) {
                row.entity_type = kind;
            } else {
                problems.push(FieldError::new(
                    "entity_type",
                    "invalid",
                    "must be `individual`, `household_business` or `organization`",
                ));
            }
        }
        if let Some(name) = input.legal_name {
            let name = name.trim().to_owned();
            if name.is_empty() || name.chars().count() > 200 {
                problems.push(FieldError::new(
                    "legal_name",
                    "length",
                    "must be 1 to 200 characters long",
                ));
            }
            row.legal_name = name;
        }
        text(&mut row.trade_name, input.trade_name, "trade_name", 200, &mut problems);
        text(&mut row.tax_code, input.tax_code, "tax_code", 14, &mut problems);
        if row.tax_code.as_deref().is_some_and(|code| !valid_tax_code(code)) {
            problems.push(FieldError::new(
                "tax_code",
                "format",
                "must be 10 or 12 digits, or 10 digits, `-` and 3 digits",
            ));
        }
        text(
            &mut row.registration_number,
            input.registration_number,
            "registration_number",
            50,
            &mut problems,
        );
        text(
            &mut row.registration_issued_on,
            input.registration_issued_on,
            "registration_issued_on",
            10,
            &mut problems,
        );
        if row
            .registration_issued_on
            .as_deref()
            .is_some_and(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").is_err())
        {
            problems.push(FieldError::new(
                "registration_issued_on",
                "format",
                "must be a date `YYYY-MM-DD`",
            ));
        }
        text(
            &mut row.registration_issued_by,
            input.registration_issued_by,
            "registration_issued_by",
            200,
            &mut problems,
        );
        text(
            &mut row.representative_name,
            input.representative_name,
            "representative_name",
            200,
            &mut problems,
        );
        text(
            &mut row.representative_title,
            input.representative_title,
            "representative_title",
            100,
            &mut problems,
        );
        text(&mut row.phone, input.phone, "phone", 20, &mut problems);
        if row.phone.as_deref().is_some_and(|phone| {
            !phone.chars().all(|c| c.is_ascii_digit() || matches!(c, '+' | ' ' | '.' | '-'))
        }) {
            problems.push(FieldError::new("phone", "format", "is not a phone number"));
        }
        text(&mut row.email, input.email, "email", 254, &mut problems);
        if row.email.as_deref().is_some_and(|email| !autoxide_kernel::email::is_valid(email)) {
            problems.push(FieldError::new("email", "email", "must be an email address"));
        }
        match input.address {
            Some(Some(address)) => match &self.units {
                Some(units) => match units.validate(address, "address.").await {
                    Ok(address) => {
                        row.address =
                            Some(serde_json::to_string(&address).map_err(Error::internal)?);
                    }
                    Err(fields) => problems.extend(fields),
                },
                None => problems.push(FieldError::new(
                    "address",
                    "unavailable",
                    "addresses cannot be checked on this server",
                )),
            },
            Some(None) => row.address = None,
            None => {}
        }
        if let Some(kyc) = input.kyc_profile_id {
            if let Some(id) = kyc {
                let found = kyc_profile::Entity::find_by_id(id)
                    .scoped(tenant)
                    .one(tenant.database().read())
                    .await?;
                if found.is_none() {
                    problems.push(FieldError::new(
                        "kyc_profile_id",
                        "not_found",
                        "is not an identity verification profile of this tenant",
                    ));
                }
            }
            row.kyc_profile_id = kyc;
        }
        if problems.is_empty() { Ok(()) } else { Err(Error::validation(problems)) }
    }

    /// Nút của hồ sơ: `org` (phải có trong cây) hay nút gốc.
    async fn checked_org(&self, tenant: &TenantContext, org: Option<Uuid>) -> Result<OrgId, Error> {
        let root = root_of(tenant.id());
        match org.map(OrgId::from_uuid) {
            None => Ok(root),
            Some(org) if org == root => Ok(root),
            Some(org) => {
                let lineage =
                    OrgTree::ancestors(tenant.database().read(), tenant.id(), org).await?;
                if lineage.is_empty() { Err(site_not_found(org)) } else { Ok(org) }
            }
        }
    }

    pub(crate) async fn create_profile(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Option<Uuid>,
        input: ProfileFields,
    ) -> Result<legal_profile::Model, Error> {
        principal.require(&LEGAL_MANAGE)?;
        let org = self.checked_org(tenant, org).await?;
        principal
            .require_at(tenant.database().read(), &LEGAL_MANAGE, OrgTarget::new(org, None))
            .await?;
        let at = now();
        let mut row = legal_profile::Model {
            id: Uuid::now_v7(),
            tenant_id: tenant.id().as_uuid(),
            org_id: org.as_uuid(),
            entity_type: String::new(),
            legal_name: String::new(),
            trade_name: None,
            tax_code: None,
            registration_number: None,
            registration_issued_on: None,
            registration_issued_by: None,
            address: None,
            representative_name: None,
            representative_title: None,
            phone: None,
            email: None,
            kyc_profile_id: None,
            created_at: at,
            updated_at: at,
        };
        let mut missing = Vec::new();
        if input.entity_type.is_none() {
            missing.push(FieldError::new(
                "entity_type",
                "required",
                "must be `individual`, `household_business` or `organization`",
            ));
        }
        if input.legal_name.is_none() {
            missing.push(FieldError::new(
                "legal_name",
                "length",
                "must be 1 to 200 characters long",
            ));
        }
        let applied = self.apply_profile(tenant, &mut row, input).await;
        match applied {
            Err(error) if !missing.is_empty() => {
                let mut all = missing;
                all.extend(error.fields().iter().cloned());
                return Err(Error::validation(all));
            }
            Err(error) => return Err(error),
            Ok(()) if !missing.is_empty() => return Err(Error::validation(missing)),
            Ok(()) => {}
        }
        let transaction = tenant.database().begin_write().await?;
        let exists = legal_profile::Entity::find()
            .scoped(tenant)
            .filter(legal_profile::Column::OrgId.eq(org.as_uuid()))
            .one(&transaction)
            .await?;
        if exists.is_some() {
            return Err(profile_exists(org.as_uuid()));
        }
        let saved =
            legal_profile::ActiveModel::from(row).insert(&transaction).await.map_err(|error| {
                if crate::service::is_unique_violation(&error) {
                    profile_exists(org.as_uuid())
                } else {
                    error.into()
                }
            })?;
        let event = events::LegalProfileUpdated(
            json!({"profile_id": saved.id, "org_id": saved.org_id, "created": true}),
        );
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                Some(format!("compliance.legal_profile:{}", saved.id)),
                &event,
            )
            .await?;
        transaction.commit().await?;
        Ok(saved)
    }

    /// Hồ sơ `id` người gọi được xem / sửa với `permission`; ngoài phạm vi → 404.
    pub(crate) async fn profile_in_scope(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        permission: &autoxide_kernel::access::Permission,
        id: Uuid,
    ) -> Result<legal_profile::Model, Error> {
        principal.require(permission)?;
        let row = legal_profile::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(profile_not_found)?;
        principal
            .require_at(
                tenant.database().read(),
                permission,
                OrgTarget::new(OrgId::from_uuid(row.org_id), None),
            )
            .await
            .map_err(|error| {
                if error.code() == "org_out_of_scope" { profile_not_found() } else { error }
            })?;
        Ok(row)
    }

    pub(crate) async fn get_profile(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<legal_profile::Model, Error> {
        self.profile_in_scope(tenant, principal, &LEGAL_READ, id).await
    }

    pub(crate) async fn list_profiles_legal(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<legal_profile::Model>, Error> {
        let scope = principal.scope_condition(&LEGAL_READ, legal_profile::Column::OrgId, None)?;
        let select = legal_profile::Entity::find().scoped(tenant).filter(scope);
        fetch_page(select, legal_profile::Column::Id, |row| row.id, page, tenant.database().read())
            .await
    }

    /// Sửa hồ sơ `id`; `expected` là phiên bản (`If-Match`) phải khớp.
    pub(crate) async fn update_profile(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        expected: Option<DateTime<Utc>>,
        input: ProfileFields,
    ) -> Result<legal_profile::Model, Error> {
        let before = self.profile_in_scope(tenant, principal, &LEGAL_MANAGE, id).await?;
        let mut row = before.clone();
        self.apply_profile(tenant, &mut row, input).await?;
        row.updated_at = now();
        let transaction = tenant.database().begin_write().await?;
        let mut update = legal_profile::Entity::update_many()
            .set(legal_profile::ActiveModel::from(row.clone()).reset_all())
            .filter(legal_profile::Column::TenantId.eq(tenant.id().as_uuid()))
            .filter(legal_profile::Column::Id.eq(id));
        // Không gửi `If-Match`: vẫn chống đè giữa lúc đọc và lúc ghi của chính request này.
        update = update
            .filter(legal_profile::Column::UpdatedAt.eq(expected.unwrap_or(before.updated_at)));
        let result = update.exec(&transaction).await?;
        if result.rows_affected == 0 {
            return Err(autoxide_kernel::version::changed());
        }
        let event = events::LegalProfileUpdated(
            json!({"profile_id": row.id, "org_id": row.org_id, "created": false}),
        );
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                Some(format!("compliance.legal_profile:{}", row.id)),
                &event,
            )
            .await?;
        transaction.commit().await?;
        Ok(row)
    }

    /// Hồ sơ áp cho nút `org`: của chính nút, hay của nút cấp trên gần nhất có hồ sơ.
    pub(crate) async fn profile_for_org(
        &self,
        tenant: &TenantContext,
        org: OrgId,
    ) -> Result<Option<legal_profile::Model>, Error> {
        let root = root_of(tenant.id());
        let mut lineage = if org == root {
            vec![root]
        } else {
            OrgTree::ancestors(tenant.database().read(), tenant.id(), org).await?
        };
        if !lineage.contains(&root) {
            lineage.push(root);
        }
        let rows = legal_profile::Entity::find()
            .scoped(tenant)
            .filter(legal_profile::Column::OrgId.is_in(lineage.iter().map(OrgId::as_uuid)))
            .all(tenant.database().read())
            .await?;
        Ok(lineage
            .iter()
            .find_map(|node| rows.iter().find(|row| row.org_id == node.as_uuid()).cloned()))
    }

    /// Biến mẫu của hồ sơ: `owner.*`, `contact.*` (thông tin vốn phải công khai).
    pub(crate) fn template_variables_of(row: &legal_profile::Model) -> Value {
        let address = address_of(row);
        json!({
            "owner": {
                "entity_type": row.entity_type,
                "legal_name": row.legal_name,
                "trade_name": row.trade_name.clone().unwrap_or_else(|| row.legal_name.clone()),
                "tax_code": row.tax_code,
                "registration_number": row.registration_number,
                "registration_issued_on": row.registration_issued_on,
                "registration_issued_by": row.registration_issued_by,
                "address": address.as_ref().map(address_line),
                "representative_name": row.representative_name,
                "representative_title": row.representative_title,
            },
            "contact": {
                "email": row.email,
                "phone": row.phone,
                "address": address.as_ref().map(address_line),
            },
        })
    }

    /// Biến mẫu cho nút `org` (`None`: cả khách); không có hồ sơ → `None`.
    pub(crate) async fn template_variables(
        &self,
        tenant: &TenantContext,
        org: Option<OrgId>,
    ) -> Result<Option<Value>, Error> {
        let org = org.unwrap_or_else(|| root_of(tenant.id()));
        Ok(self.profile_for_org(tenant, org).await?.as_ref().map(Self::template_variables_of))
    }
}
