//! Bảng `forms_submissions`: bài gửi của một form.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "forms_submissions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub form_id: Uuid,
    /// Phiên bản trường của form lúc gửi.
    pub form_version: i32,
    /// JSON `{key: value}` đã kiểm và chuẩn hoá.
    pub data: String,
    /// `new`, `read`, `archived` hoặc `spam`.
    pub status: String,
    /// Lý do bị giữ lại (`too_fast,duplicate`…), khi là spam.
    pub spam_reason: Option<String>,
    /// Email người gửi (trim + chữ thường), để xuất / xoá theo email.
    pub email: Option<String>,
    pub locale: Option<String>,
    pub page_url: Option<String>,
    pub referrer: Option<String>,
    pub user_agent: Option<String>,
    /// `ip_hash::fingerprint`; xoá sau `AUTOXIDE_FORMS_IP_RETENTION_DAYS`.
    pub ip_fingerprint: Option<String>,
    /// JSON bằng chứng đồng ý `{text, version, at, source, ip_fingerprint}`, nếu form có ô
    /// đồng ý.
    pub consent: Option<String>,
    /// `antispam::content_hash` của câu trả lời, để dò bài trùng.
    pub content_hash: String,
    /// `search::fold` của mọi câu trả lời dạng chữ, để tìm.
    pub search_key: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}
