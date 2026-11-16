//! Bảng `content_published`: bản công khai của mục (chỉ có khi đang công khai).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_published")]
pub struct Model {
    /// Khách sở hữu dòng (bước 33), cột đầu của khoá chính.
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    /// Nút sở hữu, chép từ mục khi đăng (bước 34): đọc công khai lọc theo website.
    pub org_id: Option<Uuid>,
    #[sea_orm(primary_key, auto_increment = false)]
    pub entry_id: Uuid,
    pub kind: String,
    pub locale: String,
    pub translation_group_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub revision_id: Uuid,
    /// Đường dẫn do máy chủ tính (`/gioi-thieu/doi-ngu`, `/blog/slug`).
    pub path: String,
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
    pub ai_generated: bool,
    /// Người (phiên đăng nhập, không phải API key) đã xác nhận duyệt nội dung do AI tạo khi đăng.
    pub ai_reviewed_by: Option<Uuid>,
    pub ai_reviewed_at: Option<DateTimeUtc>,
    /// Ngày hiển thị là ngày đăng (sửa được).
    pub published_at: DateTimeUtc,
    pub first_published_at: DateTimeUtc,
    /// Lần cuối nội dung công khai đổi (cho sitemap `lastmod`, feed `updated`).
    pub content_modified_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
