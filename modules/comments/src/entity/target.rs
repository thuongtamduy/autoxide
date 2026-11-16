//! Bảng `comments_targets`: trạng thái và số đếm của một đích. Khoá chính (`tenant_id`,
//! `target_type`, `target_id`); dòng tạo khi có bình luận đầu tiên (hoặc khi quản trị viên đặt
//! trạng thái).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "comments_targets")]
pub struct Model {
    /// Khách sở hữu dòng (bước 33), cột đầu của khoá chính.
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub target_type: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub target_id: String,
    /// UUIDv7 cấp khi tạo (dòng cũ: cấp ở migration `m0012`), cho cursor của danh sách (D-270);
    /// unique trong khách (`comments_targets_uid_key`).
    pub uid: Uuid,
    /// `open` hoặc `closed`.
    pub state: String,
    /// Ẩn mọi bình luận khỏi phần công khai (vd đích đã bị xoá).
    pub hidden: bool,
    /// Tự đóng từ lúc này.
    pub closes_at: Option<DateTimeUtc>,
    /// Số bình luận đang hiện.
    pub published_count: i32,
    /// Số bình luận chờ duyệt.
    pub pending_count: i32,
    /// Số bình luận gốc (câu hỏi) đang hiện (D-202).
    pub published_thread_count: i32,
    /// Số bình luận gốc chờ duyệt.
    pub pending_thread_count: i32,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
