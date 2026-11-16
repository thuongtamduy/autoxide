//! Bảng `menus_items`: một mục của menu (cây nhiều tầng, thứ tự theo `position`).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "menus_items")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    pub menu_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub position: i32,
    pub label: String,
    pub url: Option<String>,
    pub ref_kind: Option<String>,
    pub ref_id: Option<String>,
    pub auto_children: bool,
    pub auto_children_limit: Option<i32>,
    pub icon: Option<String>,
    pub highlight: Option<String>,
    pub badge_text: Option<String>,
    pub badge_tone: Option<String>,
    pub badge_from: Option<DateTimeUtc>,
    pub badge_until: Option<DateTimeUtc>,
    pub feature_kind: Option<String>,
    pub feature_id: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
