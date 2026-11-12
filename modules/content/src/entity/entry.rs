//! Bảng `content_entries`: bản nháp làm việc của mỗi mục (bài, trang; mỗi bản dịch một dòng).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_entries")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
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
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    pub revised_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}
