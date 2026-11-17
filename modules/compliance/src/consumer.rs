//! Bảo vệ người tiêu dùng (bước 37, D-364; Luật Bảo vệ quyền lợi người tiêu dùng 19/2023/QH15 và NĐ
//! 55/2024/NĐ-CP, hiệu lực 01/7/2024 — cần đối chiếu văn bản gốc / người có chuyên môn pháp lý):
//!
//! - Lý do được ẩn nội dung của người tiêu dùng (đánh giá, bình luận) là dữ liệu có ngày hiệu lực:
//!   quy tắc `moderation.hide_reasons` (`reasons`, và `surfaces.<nơi>` khi một nơi có danh sách
//!   riêng). Module ẩn đánh giá hỏi [`crate::api::ModerationPolicy`] trước khi ẩn, và ghi lý do vào
//!   event của chính nó — đánh giá tiêu cực hợp lệ không bị ẩn.
//! - Kênh khiếu nại: vụ việc `consumer_complaint` (hạn từ `cases.deadlines`), liên hệ là chính sách
//!   `complaints.contact`, công khai cùng hạn qua `GET /public/complaints`.
//! - Điều kiện giao dịch chung: văn bản `general_conditions` có phiên bản (đã có), công khai qua
//!   `GET /public/documents/general_conditions`.

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use chrono::Utc;
use serde_json::Value;

use crate::http_cases::ComplaintChannelResponse;
use crate::service::ComplianceService;

pub(crate) fn hide_reason_not_allowed(reason: &str, allowed: &[String]) -> Error {
    Error::new(
        autoxide_kernel::error::ErrorKind::Validation,
        "compliance.hide_reason_not_allowed",
        "Content of consumers may not be hidden for `{reason}`; allowed reasons: {allowed}.",
    )
    .with_param("reason", reason)
    .with_param("allowed", allowed.join(", "))
}

/// Lý do của nơi `surface` trong thân quy tắc.
pub(crate) fn reasons_of(body: Option<&Value>, surface: &str) -> Vec<String> {
    let list = |value: Option<&Value>| -> Option<Vec<String>> {
        value?
            .as_array()
            .map(|items| items.iter().filter_map(|item| item.as_str().map(str::to_owned)).collect())
    };
    body.and_then(|body| list(body.get("surfaces").and_then(|surfaces| surfaces.get(surface))))
        .or_else(|| body.and_then(|body| list(body.get("reasons"))))
        .unwrap_or_default()
}

impl ComplianceService {
    /// Lý do được ẩn ở nơi `surface` và căn cứ của quy tắc hiệu lực.
    pub(crate) async fn hide_reasons(
        &self,
        tenant: &TenantContext,
        surface: &str,
    ) -> Result<(Vec<String>, Option<String>), Error> {
        let rule = self.rule_for(tenant, "moderation.hide_reasons", Utc::now()).await?;
        let legal_basis = rule.as_ref().map(|row| row.legal_basis.clone());
        let body = rule.and_then(|row| serde_json::from_str::<Value>(&row.body).ok());
        Ok((reasons_of(body.as_ref(), surface), legal_basis))
    }

    pub(crate) async fn check_hide_reason(
        &self,
        tenant: &TenantContext,
        surface: &str,
        reason: &str,
    ) -> Result<(), Error> {
        let (allowed, _) = self.hide_reasons(tenant, surface).await?;
        if allowed.iter().any(|item| item == reason) {
            Ok(())
        } else {
            Err(hide_reason_not_allowed(reason, &allowed))
        }
    }

    pub(crate) async fn complaint_channel(
        &self,
        tenant: &TenantContext,
    ) -> Result<ComplaintChannelResponse, Error> {
        let rule = self.rule_for(tenant, "cases.deadlines", Utc::now()).await?;
        let legal_basis = rule.as_ref().map(|row| row.legal_basis.clone());
        let deadlines = self.deadlines_of(tenant, "consumer_complaint").await?;
        Ok(ComplaintChannelResponse {
            contact: self.policy(tenant, "complaints.contact").await?,
            submit_path: "/api/v1/compliance/public/complaints".to_owned(),
            acknowledge_within_hours: deadlines.ack_hours,
            respond_within_working_days: deadlines.respond_days,
            decide_within_working_days: deadlines.decide_days,
            general_conditions_path: "/api/v1/compliance/public/documents/general_conditions"
                .to_owned(),
            legal_basis,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ly_do_theo_noi_hay_danh_sach_chung() {
        let body = json!({"reasons": ["spam", "offensive"], "surfaces": {"reviews": ["spam"]}});
        assert_eq!(reasons_of(Some(&body), "reviews"), vec!["spam"]);
        assert_eq!(reasons_of(Some(&body), "comments"), vec!["spam", "offensive"]);
        assert!(reasons_of(None, "reviews").is_empty());
    }
}
