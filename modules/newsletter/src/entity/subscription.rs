//! Bảng `newsletter_subscriptions`: người đăng ký nào ở danh sách nào, trạng thái riêng của từng
//! danh sách. Khoá chính (`tenant_id`, `subscriber_id`, `list_id`).

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "newsletter_subscriptions")]
pub struct Model {
    /// Khách sở hữu dòng (bước 33), cột đầu của khoá chính.
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub subscriber_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub list_id: Uuid,
    /// `unconfirmed`, `confirmed` hoặc `unsubscribed`.
    pub status: String,
    /// JSON bằng chứng đồng ý `{text, version, at, source, ip_fingerprint}`, nếu có.
    pub consent: Option<String>,
    /// Lý do bộ chống spam giữ lại (`too_fast,duplicate`…): đăng ký bị buộc xác nhận qua mail.
    pub hold_reason: Option<String>,
    pub requested_at: Timestamp,
    pub confirmed_at: Option<Timestamp>,
    pub unsubscribed_at: Option<Timestamp>,
    pub updated_at: Timestamp,
    /// Sổ đồng ý của `compliance` đã biết tới đâu: `''`, `granted` hay `withdrawn` (D-367).
    pub ledger_state: String,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
