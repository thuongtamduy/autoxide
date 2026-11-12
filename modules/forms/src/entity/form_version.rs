//! Bảng `forms_form_versions`: ảnh chụp danh sách trường mỗi khi trường đổi — bài gửi cũ luôn
//! đọc được với đúng nhãn lúc gửi.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "forms_form_versions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub form_id: Uuid,
    pub version: i32,
    /// JSON: danh sách trường của phiên bản này.
    pub fields: String,
    pub created_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}
