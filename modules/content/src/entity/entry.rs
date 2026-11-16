//! Bảng `content_entries`: bản nháp làm việc của mỗi mục (bài, trang; mỗi bản dịch một dòng).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_entries")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    /// Nút sở hữu (bước 34); gốc = ID của khách. Luôn có giá trị.
    pub org_id: Option<Uuid>,
    /// `post` hoặc `page`.
    pub kind: String,
    pub locale: String,
    pub translation_group_id: Uuid,
    pub parent_id: Option<Uuid>,
    /// `parent_id`, hoặc UUID toàn số 0 khi không có cha (để unique chặn được trang gốc trùng).
    pub parent_key: Uuid,
    /// `draft`, `published` hoặc `archived`.
    pub status: String,
    pub has_unpublished_changes: bool,
    pub title: String,
    pub slug: String,
    pub excerpt: String,
    /// JSON của thân bài (xem `body.rs`).
    pub body: String,
    pub body_schema_version: i32,
    pub plain_text: String,
    pub search_key: String,
    pub word_count: i32,
    pub reading_time_minutes: i32,
    pub featured_media_id: Option<Uuid>,
    pub featured_media_alt: String,
    pub meta_title: String,
    pub meta_description: String,
    pub og_media_id: Option<Uuid>,
    pub canonical_url: String,
    pub noindex: bool,
    pub created_by: Option<Uuid>,
    /// API key đã tạo mục (`NULL`: tạo bằng phiên đăng nhập). Cùng `created_by` quyết định
    /// "mục của chính bên gọi" cho `content.write_own` / `content.publish_own`.
    pub created_by_api_key_id: Option<Uuid>,
    /// Nội dung do AI tạo (máy chủ dịch, hay bên gọi tự khai).
    pub ai_generated: bool,
    /// Lúc bên viết xin duyệt (`NULL`: không xin, hay đã đăng / rút lại).
    pub review_requested_at: Option<DateTimeUtc>,
    pub review_requested_by: Option<Uuid>,
    /// Bản dịch do máy chủ dịch từ revision này của bản gốc (D-109); `NULL`: không phải.
    pub translated_from_revision_id: Option<Uuid>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    pub revised_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
