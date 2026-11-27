//! Bảng `content_published_words`: chỉ mục đảo của bản công khai (PT-15, D-20261126-content-word-index) — mỗi từ của
//! một bài đã đăng một dòng. Ghi cùng transaction với bản đăng; đọc khi tìm chữ công khai.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_published_words")]
pub struct Model {
    /// Khách sở hữu dòng (bước 33), cột đầu của khoá chính.
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub entry_id: Uuid,
    /// Một từ đã gấp dấu, chữ thường (`publishing::words::words`).
    #[sea_orm(primary_key, auto_increment = false)]
    pub word: String,
    /// Ngôn ngữ của bản đăng (lọc trước khi so từ).
    pub locale: String,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
