//! Mục định danh người bán theo NĐ 248/2026/NĐ-CP Điều 18 (LG-3c, D-20261128-kyc-article-18) — cần đối chiếu văn
//! bản gốc: cá nhân — họ tên, ngày sinh, số định danh cá nhân; tổ chức, hộ kinh doanh — tên, trụ sở, số định danh,
//! người đại diện; cá nhân nước ngoài — hộ chiếu còn hạn ít nhất 6 tháng (lúc xét duyệt). Áp dụng từ 01/01/2027
//! (Điều 52 theo các bản tổng quan).
//!
//! Mục nào bắt buộc là dữ liệu có ngày hiệu lực (`seller_details`, `passport_min_valid_months` của quy tắc
//! `kyc.required_activities`), không viết cứng. Định danh vẫn là định danh (trạng thái `verified` không đổi);
//! cổng chỉ chặn việc **bán** (`Activity::Sell`) khi hồ sơ thiếu mục, với `compliance.kyc_required` và
//! `profile_status = incomplete`. Hồ sơ cũ thiếu mục bổ sung qua nhà cung cấp (lượt mới) hay người duyệt
//! (`PUT /kyc/profiles/{id}/details`).

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::time::{Date, Timestamp};
use serde_json::Value;
use uuid::Uuid;

use crate::entity::kyc_profile;
use crate::permissions::KYC_REVIEW;
use crate::service::{ComplianceService, require_session};

/// Mục Điều 18 người duyệt ghi sau khi đối chiếu giấy tờ. Trường trống: giữ giá trị đang có.
#[derive(Debug, Clone, Default)]
pub(crate) struct SellerDetails {
    pub birth_date: Option<String>,
    pub address: Option<String>,
    pub representative_name: Option<String>,
    pub document_country: Option<String>,
    pub document_expires_on: Option<String>,
}

impl SellerDetails {
    /// Tên các mục có giá trị (cho event audit, không kèm giá trị).
    pub(crate) fn names(&self) -> Vec<&'static str> {
        [
            ("birth_date", &self.birth_date),
            ("address", &self.address),
            ("representative_name", &self.representative_name),
            ("document_country", &self.document_country),
            ("document_expires_on", &self.document_expires_on),
        ]
        .into_iter()
        .filter(|(_, value)| value.is_some())
        .map(|(name, _)| name)
        .collect()
    }
}

fn present(value: Option<&String>) -> bool {
    value.is_some_and(|text| !text.trim().is_empty())
}

/// Mục còn thiếu của hồ sơ `row` để được **bán** theo thân quy tắc `rule` (rỗng: đủ). Tên mục là tên trường của
/// hồ sơ; `document_expires_on` cũng báo khi hộ chiếu còn hạn chưa đủ lâu lúc xác thực.
pub(crate) fn missing_for_sale(rule: &Value, row: &kyc_profile::Model) -> Vec<&'static str> {
    let mut missing = Vec::new();
    let required = rule["seller_details"][row.party_type.as_str()].as_array();
    for name in required.into_iter().flatten().filter_map(Value::as_str) {
        let (field, value) = match name {
            "birth_date" => ("birth_date", row.birth_date.as_ref()),
            "address" => ("address", row.address.as_ref()),
            "representative_name" => ("representative_name", row.representative_name.as_ref()),
            "document_country" => ("document_country", row.document_country.as_ref()),
            "document_expires_on" => ("document_expires_on", row.document_expires_on.as_ref()),
            // Mục quy tắc đòi mà core chưa có chỗ ghi: không được coi là đủ (fail-closed).
            _ => {
                tracing::error!(
                    field = name,
                    "kyc rule asks for a seller detail core does not keep"
                );
                missing.push("unknown_detail");
                continue;
            }
        };
        if !present(value) {
            missing.push(field);
        }
    }
    let passport = row.document_kind.as_deref() == Some("passport");
    if let (true, Some(months)) = (passport, rule["passport_min_valid_months"].as_i64()) {
        if !present(row.document_country.as_ref()) && !missing.contains(&"document_country") {
            missing.push("document_country");
        }
        let until = row.document_expires_on.as_deref().and_then(|text| text.parse::<Date>().ok());
        let checked_on = row.verified_at.map(Timestamp::date_utc);
        let long_enough = match (until, checked_on) {
            (Some(until), Some(on)) => i32::try_from(months)
                .ok()
                .and_then(|months| on.checked_add_months(months))
                .is_some_and(|needed| until >= needed),
            _ => false,
        };
        if !long_enough && !missing.contains(&"document_expires_on") {
            missing.push("document_expires_on");
        }
    }
    missing
}

/// Hồ sơ không sống lâu hơn giấy tờ: `expires_at` không quá hết ngày `document_expires_on` (UTC).
pub(crate) fn capped_expiry(
    expires_at: Option<Timestamp>,
    document_expires_on: Option<&str>,
) -> Option<Timestamp> {
    let end = document_expires_on
        .and_then(|text| text.parse::<Date>().ok())
        .and_then(Date::succ)
        .map(Date::start_of_day_utc);
    match (expires_at, end) {
        (Some(at), Some(end)) => Some(at.min(end)),
        (at, None) => at,
        (None, end) => end,
    }
}

/// Kiểm và chuẩn hoá đầu vào; gom mọi lỗi để báo một lần (D-083).
fn checked(input: SellerDetails) -> Result<SellerDetails, Error> {
    let mut problems = Vec::new();
    let trim = |value: Option<String>| value.map(|text| text.trim().to_owned());
    let today = Date::today_utc();
    let birth_date = trim(input.birth_date);
    if let Some(text) = &birth_date {
        match text.parse::<Date>() {
            Ok(date) if date <= today => {}
            Ok(_) => {
                problems.push(FieldError::new("birth_date", "range", "must not be in the future"))
            }
            Err(_) => problems.push(FieldError::new(
                "birth_date",
                "invalid",
                "must be a date `YYYY-MM-DD`",
            )),
        }
    }
    let address = trim(input.address);
    if address.as_ref().is_some_and(|text| text.is_empty() || text.chars().count() > 500) {
        problems.push(FieldError::new("address", "length", "must have 1 to 500 characters"));
    }
    let representative_name = trim(input.representative_name);
    if representative_name
        .as_ref()
        .is_some_and(|text| text.is_empty() || text.chars().count() > 200)
    {
        problems.push(FieldError::new(
            "representative_name",
            "length",
            "must have 1 to 200 characters",
        ));
    }
    let document_country = trim(input.document_country).map(|text| text.to_ascii_uppercase());
    if document_country
        .as_ref()
        .is_some_and(|text| text.len() != 2 || !text.chars().all(|c| c.is_ascii_uppercase()))
    {
        problems.push(FieldError::new(
            "document_country",
            "invalid",
            "must be a two-letter ISO 3166-1 country code, like `VN`",
        ));
    }
    let document_expires_on = trim(input.document_expires_on);
    if document_expires_on.as_ref().is_some_and(|text| text.parse::<Date>().is_err()) {
        problems.push(FieldError::new(
            "document_expires_on",
            "invalid",
            "must be a date `YYYY-MM-DD`",
        ));
    }
    if birth_date.is_none()
        && address.is_none()
        && representative_name.is_none()
        && document_country.is_none()
        && document_expires_on.is_none()
    {
        problems.push(FieldError::new("details", "required", "give at least one detail"));
    }
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    Ok(SellerDetails {
        birth_date,
        address,
        representative_name,
        document_country,
        document_expires_on,
    })
}

impl ComplianceService {
    /// Người duyệt ghi mục Điều 18 đã đối chiếu với giấy tờ (hồ sơ chưa bị thu hồi); ghi ở `kyc.rs`
    /// ([`ComplianceService::save_details`]).
    pub(crate) async fn record_details(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        input: SellerDetails,
    ) -> Result<kyc_profile::Model, Error> {
        principal.require(&KYC_REVIEW)?;
        require_session(principal)?;
        let input = checked(input)?;
        let saved = self.save_details(tenant, principal.user_id(), id, &input).await?;
        // Không ghi giá trị (dữ liệu cá nhân) vào log.
        tracing::info!(profile_id = %id, "seller identity details recorded by a reviewer");
        Ok(saved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn profile(party: &str) -> kyc_profile::Model {
        let at = Timestamp::from_utc(2027, 1, 10, 3, 0, 0).unwrap();
        kyc_profile::Model {
            id: Uuid::now_v7(),
            tenant_id: Uuid::now_v7(),
            subject_kind: "org".to_owned(),
            subject_id: Uuid::now_v7().to_string(),
            party_type: party.to_owned(),
            status: "verified".to_owned(),
            method: None,
            provider: None,
            provider_reference: None,
            verified_name: Some("NGUOI BAN".to_owned()),
            document_kind: Some("cccd".to_owned()),
            document_hash: None,
            document_last4: None,
            assurance: None,
            document_files: "[]".to_owned(),
            info_request: None,
            verified_at: Some(at),
            expires_at: None,
            reason_code: None,
            reviewed_by: None,
            created_at: at,
            updated_at: at,
            review_note: None,
            verified_document_hash: None,
            birth_date: None,
            address: None,
            representative_name: None,
            document_country: None,
            document_expires_on: None,
        }
    }

    fn rule() -> Value {
        json!({
            "seller_details": {
                "individual": ["birth_date"],
                "household_business": ["address", "representative_name"],
                "organization": ["address", "representative_name"]
            },
            "passport_min_valid_months": 6
        })
    }

    #[test]
    fn muc_bat_buoc_theo_loai_ben() {
        let mut person = profile("individual");
        assert_eq!(missing_for_sale(&rule(), &person), ["birth_date"]);
        person.birth_date = Some("1990-05-01".to_owned());
        assert!(missing_for_sale(&rule(), &person).is_empty());

        let mut company = profile("organization");
        assert_eq!(missing_for_sale(&rule(), &company), ["address", "representative_name"]);
        company.address = Some("1 Trang Tien, Ha Noi".to_owned());
        company.representative_name = Some("NGUYEN VAN A".to_owned());
        assert!(missing_for_sale(&rule(), &company).is_empty());
        // Quy tắc cũ (không có `seller_details`): không đòi gì thêm.
        assert!(missing_for_sale(&json!({"sell": true}), &profile("organization")).is_empty());
    }

    /// Hộ chiếu phải còn hạn ít nhất 6 tháng tính từ ngày xác thực (10/01/2027 → 10/07/2027).
    #[test]
    fn ho_chieu_con_han_it_nhat_sau_thang_luc_xac_thuc() {
        let mut person = profile("individual");
        person.birth_date = Some("1990-05-01".to_owned());
        person.document_kind = Some("passport".to_owned());
        assert_eq!(missing_for_sale(&rule(), &person), ["document_country", "document_expires_on"]);
        person.document_country = Some("US".to_owned());
        person.document_expires_on = Some("2027-07-09".to_owned());
        assert_eq!(missing_for_sale(&rule(), &person), ["document_expires_on"]);
        person.document_expires_on = Some("2027-07-10".to_owned());
        assert!(missing_for_sale(&rule(), &person).is_empty());
    }

    #[test]
    fn muc_quy_tac_doi_ma_core_khong_co_thi_khong_du() {
        let rule = json!({"seller_details": {"individual": ["tax_code"]}});
        assert_eq!(missing_for_sale(&rule, &profile("individual")), ["unknown_detail"]);
    }

    #[test]
    fn han_ho_so_khong_qua_han_giay_to() {
        let long = Timestamp::from_utc(2030, 1, 1, 0, 0, 0).unwrap();
        let capped = capped_expiry(Some(long), Some("2028-03-31")).unwrap();
        assert_eq!(capped, Timestamp::from_utc(2028, 4, 1, 0, 0, 0).unwrap());
        assert_eq!(capped_expiry(Some(long), None), Some(long));
        assert_eq!(capped_expiry(Some(long), Some("không phải ngày")), Some(long));
    }

    #[test]
    fn dau_vao_sai_bao_mot_lan() {
        let input = SellerDetails {
            birth_date: Some("2999-01-01".to_owned()),
            address: Some("  ".to_owned()),
            representative_name: None,
            document_country: Some("Viet Nam".to_owned()),
            document_expires_on: Some("31/12/2030".to_owned()),
        };
        let error = checked(input).unwrap_err();
        assert_eq!(error.fields().len(), 4, "{error:?}");
        assert!(checked(SellerDetails::default()).is_err());
        let fine = checked(SellerDetails {
            document_country: Some("vn".to_owned()),
            ..SellerDetails::default()
        })
        .unwrap();
        assert_eq!(fine.document_country.as_deref(), Some("VN"));
    }
}
