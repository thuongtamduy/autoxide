//! Bảng `content_campaign_drafts`: yêu cầu bộ nháp chiến dịch từ một chương trình khuyến mại (bước 67).

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_campaign_drafts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    pub org_id: Option<Uuid>,
    pub promotion_id: Uuid,
    pub locale: String,
    /// JSON: mảng kênh.
    pub channels: String,
    pub instruction: String,
    pub banner_placement: Option<String>,
    pub newsletter_list_id: Option<Uuid>,
    pub status: String,
    pub tries: i32,
    pub error_code: Option<String>,
    /// JSON: kết quả của từng kênh.
    pub results: Option<String>,
    /// JSON của `PrincipalSnapshot`.
    pub principal: String,
    pub requested_by: Uuid,
    pub created_at: Timestamp,
    pub finished_at: Option<Timestamp>,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
