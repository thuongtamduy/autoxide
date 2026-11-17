//! Cung cấp thông tin người bán cho cơ quan có thẩm quyền (bước 37, D-363): ghi cơ quan, số văn bản,
//! căn cứ, đối tượng và TÊN các trường đã đưa (không giá trị) — sổ để kiểm tra lại; giá trị chỉ trả
//! một lần trong response. Nguồn: hồ sơ pháp lý, hồ sơ định danh và số điện thoại đã xác thực mà
//! chính `compliance` giữ.
//!
//! Căn cứ — cần đối chiếu văn bản gốc / người có chuyên môn pháp lý: Luật Thương mại điện tử 2025,
//! NĐ 85/2021/NĐ-CP (cung cấp thông tin khi cơ quan yêu cầu), Luật Bảo vệ dữ liệu cá nhân
//! 91/2025/QH15 (cung cấp dữ liệu cá nhân theo yêu cầu của cơ quan có thẩm quyền).

use std::collections::BTreeMap;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use chrono::{DateTime, Duration, Utc};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{
    account_verification, case, case_entry, disclosure, kyc_profile, legal_profile,
};
use crate::events;
use crate::permissions::DISCLOSURES_MANAGE;
use crate::service::{ComplianceService, now, require_session};

/// Trường cung cấp được về một nút (người bán).
pub(crate) const ORG_FIELDS: [&str; 9] = [
    "legal_name",
    "trade_name",
    "tax_code",
    "registration_number",
    "address",
    "representative_name",
    "phone",
    "email",
    "verified_name",
];
/// Trường cung cấp được về một người dùng.
pub(crate) const USER_FIELDS: [&str; 5] =
    ["verified_name", "document_kind", "document_last4", "kyc_status", "phone"];

/// Một lần cung cấp thông tin.
#[derive(Debug, Clone)]
pub(crate) struct NewDisclosure {
    pub case_id: Option<Uuid>,
    pub authority: String,
    pub request_reference: String,
    pub legal_basis: String,
    pub requested_at: Option<DateTime<Utc>>,
    pub subject: String,
    pub fields: Vec<String>,
}

/// (`org` / `user`, ID) của đối tượng.
fn parse_subject(subject: &str) -> Option<(&'static str, Uuid)> {
    let (kind, id) = subject.split_once(':')?;
    let kind = match kind {
        "org" => "org",
        "user" => "user",
        _ => return None,
    };
    Some((kind, id.parse().ok()?))
}

impl ComplianceService {
    /// Ghi và trả thông tin cho cơ quan. Cần `compliance.disclosures.manage` và phiên đăng nhập của
    /// người (không API key).
    pub(crate) async fn record_disclosure(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewDisclosure,
    ) -> Result<(disclosure::Model, BTreeMap<String, Value>), Error> {
        principal.require(&DISCLOSURES_MANAGE)?;
        require_session(principal)?;
        let mut problems = Vec::new();
        for (field, value, max) in [
            ("authority", &input.authority, 200),
            ("request_reference", &input.request_reference, 100),
            ("legal_basis", &input.legal_basis, 2000),
        ] {
            let count = value.trim().chars().count();
            if count == 0 || count > max {
                problems.push(
                    FieldError::new(field, "length", "must be 1 to {max} characters")
                        .with_param("max", max),
                );
            }
        }
        let subject = parse_subject(&input.subject);
        if subject.is_none() {
            problems.push(FieldError::new(
                "subject",
                "invalid",
                "must be `org:<unit id>` or `user:<user id>`",
            ));
        }
        let allowed: &[&str] = match subject {
            Some(("org", _)) => &ORG_FIELDS,
            _ => &USER_FIELDS,
        };
        if input.fields.is_empty()
            || input.fields.iter().any(|field| !allowed.contains(&field.as_str()))
        {
            problems.push(
                FieldError::new("fields", "invalid", "must be 1 or more of {allowed}")
                    .with_param("allowed", allowed.join(", ")),
            );
        }
        let at = now();
        let requested_at = input.requested_at.unwrap_or(at);
        if requested_at > at + Duration::minutes(5) {
            problems.push(FieldError::new("requested_at", "range", "must not be in the future"));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let read = tenant.database().read();
        if let Some(case_id) = input.case_id
            && case::Entity::find_by_id(case_id).scoped(tenant).one(read).await?.is_none()
        {
            return Err(crate::cases::case_not_found());
        }
        let (kind, id) = subject.unwrap_or(("user", Uuid::nil()));
        let values = self.subject_values(tenant, kind, id, &input.fields).await?;
        let mut fields = input.fields.clone();
        fields.sort();
        fields.dedup();
        let row = disclosure::Model {
            id: Uuid::now_v7(),
            tenant_id: tenant.id().as_uuid(),
            case_id: input.case_id,
            authority: input.authority.trim().to_owned(),
            request_reference: input.request_reference.trim().to_owned(),
            legal_basis: input.legal_basis.trim().to_owned(),
            requested_at,
            subject: format!("{kind}:{id}"),
            fields: json!(fields).to_string(),
            disclosed_by: principal.user_id(),
            disclosed_at: at,
        };
        let transaction = tenant.database().begin_write().await?;
        let saved = disclosure::ActiveModel::from(row).insert(&transaction).await?;
        if let Some(case_id) = saved.case_id {
            case_entry::ActiveModel::from(case_entry::Model {
                id: Uuid::now_v7(),
                tenant_id: tenant.id().as_uuid(),
                case_id,
                kind: "disclosure".to_owned(),
                author_kind: "staff".to_owned(),
                author_user_id: Some(principal.user_id()),
                body: format!(
                    "disclosed to {} ({}): {}",
                    saved.authority,
                    saved.request_reference,
                    fields.join(", ")
                ),
                files: "[]".to_owned(),
                created_at: at,
            })
            .insert(&transaction)
            .await?;
        }
        let event = events::DisclosureRecorded(json!({
            "disclosure_id": saved.id, "case_id": saved.case_id, "subject": saved.subject,
            "fields": fields,
        }));
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                Some(format!("compliance.disclosure:{}", saved.id)),
                &event,
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(disclosure_id = %saved.id, "information disclosed to an authority");
        Ok((saved, values))
    }

    /// Giá trị các trường `fields` của đối tượng (`null` khi không có).
    async fn subject_values(
        &self,
        tenant: &TenantContext,
        kind: &str,
        id: Uuid,
        fields: &[String],
    ) -> Result<BTreeMap<String, Value>, Error> {
        let read = tenant.database().read();
        let kyc = kyc_profile::Entity::find()
            .scoped(tenant)
            .filter(kyc_profile::Column::SubjectKind.eq(kind))
            .filter(kyc_profile::Column::SubjectId.eq(id.to_string()))
            .one(read)
            .await?;
        let mut known: BTreeMap<&str, Value> = BTreeMap::new();
        known
            .insert("verified_name", json!(kyc.as_ref().and_then(|row| row.verified_name.clone())));
        if kind == "org" {
            let profile = legal_profile::Entity::find()
                .scoped(tenant)
                .filter(legal_profile::Column::OrgId.eq(id))
                .one(read)
                .await?;
            if let Some(row) = profile {
                known.insert("legal_name", json!(row.legal_name));
                known.insert("trade_name", json!(row.trade_name));
                known.insert("tax_code", json!(row.tax_code));
                known.insert("registration_number", json!(row.registration_number));
                known.insert(
                    "address",
                    row.address
                        .as_deref()
                        .and_then(|text| serde_json::from_str(text).ok())
                        .unwrap_or(Value::Null),
                );
                known.insert("representative_name", json!(row.representative_name));
                known.insert("phone", json!(row.phone));
                known.insert("email", json!(row.email));
            }
        } else {
            known.insert(
                "document_kind",
                json!(kyc.as_ref().and_then(|row| row.document_kind.clone())),
            );
            known.insert(
                "document_last4",
                json!(kyc.as_ref().and_then(|row| row.document_last4.clone())),
            );
            known.insert("kyc_status", json!(kyc.as_ref().map(|row| row.status.clone())));
            let phone = account_verification::Entity::find_by_id((tenant.id().as_uuid(), id))
                .one(read)
                .await?
                .and_then(|row| row.phone);
            known.insert("phone", json!(phone));
        }
        Ok(fields
            .iter()
            .map(|field| (field.clone(), known.get(field.as_str()).cloned().unwrap_or(Value::Null)))
            .collect())
    }

    pub(crate) async fn list_disclosures(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        case_id: Option<Uuid>,
        page: &PageRequest,
    ) -> Result<Page<disclosure::Model>, Error> {
        principal.require(&DISCLOSURES_MANAGE)?;
        let mut select = disclosure::Entity::find().scoped(tenant);
        if let Some(case_id) = case_id {
            select = select.filter(disclosure::Column::CaseId.eq(case_id));
        }
        fetch_page(select, disclosure::Column::Id, |row| row.id, page, tenant.database().read())
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doi_tuong_cung_cap_dung_dang() {
        let id = Uuid::now_v7();
        assert_eq!(parse_subject(&format!("org:{id}")), Some(("org", id)));
        assert_eq!(parse_subject(&format!("user:{id}")), Some(("user", id)));
        assert_eq!(parse_subject("org:x"), None);
        assert_eq!(parse_subject(&format!("seller:{id}")), None);
    }
}
