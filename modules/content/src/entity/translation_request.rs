//! Bảng `content_translation_requests`: yêu cầu dịch một mục bằng AI (D-109), chạy bằng job.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_translation_requests")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub source_entry_id: Uuid,
    /// Revision của bản gốc được ghim lúc yêu cầu: sửa bản gốc sau đó không đổi bản dịch này.
    pub source_revision_id: Uuid,
    pub target_locale: String,
    /// Bản dịch đã có lúc yêu cầu (sẽ được cập nhật); `NULL`: tạo mới.
    pub target_entry_id: Option<Uuid>,
    /// `updated_at` của bản dịch lúc yêu cầu: khác lúc ghi thì có người đã sửa — không ghi đè.
    pub target_updated_at: Option<DateTimeUtc>,
    /// `queued`, `running`, `succeeded`, `failed`.
    pub status: String,
    /// `<nguồn>:<ngôn ngữ>` khi đang chờ hay đang chạy, `NULL` khi xong: unique index chặn hai
    /// yêu cầu cùng lúc cho một cặp (hai `NULL` không trùng nhau trên cả SQLite và Postgres).
    pub active_key: Option<String>,
    /// Mã lỗi khi `failed`.
    pub error_code: Option<String>,
    /// Bản dịch đã tạo hay cập nhật khi `succeeded`.
    pub result_entry_id: Option<Uuid>,
    pub created_by: Option<Uuid>,
    pub created_by_api_key_id: Option<Uuid>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}
