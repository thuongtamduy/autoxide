//! Dữ liệu cá nhân trong module `newsletter`.
//!
//! - Theo email (người đăng ký không có tài khoản): `GET` / `DELETE
//!   /api/v1/newsletter/data?email=` (`audience/subscribers.rs`), permission `newsletter.export`.
//!   Người đăng ký KHÔNG gắn với tài khoản: đăng ký công khai chỉ có email, nên mọi yêu cầu của
//!   chủ dữ liệu đi theo email.
//! - Theo tài khoản: chỉ `created_by` của danh sách và chiến dịch. Xuất: những thứ người đó tạo.
//!   Tài khoản bị xoá (`privacy::USER_DELETED`): bỏ trống `created_by`; danh sách và chiến dịch
//!   vẫn giữ (là của site). Nhận lại cùng event thì kết quả không đổi (D-054).

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::privacy::{PersonalData, deleted_user};
use autoxide_kernel::tenancy::TenantContext;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{campaign, list};
use crate::service::NewsletterService;

#[async_trait::async_trait]
impl PersonalData for NewsletterService {
    async fn export(&self, _tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
        let read = self.database.read();
        let lists =
            list::Entity::find().filter(list::Column::CreatedBy.eq(user_id)).all(read).await?;
        let campaigns = campaign::Entity::find()
            .filter(campaign::Column::CreatedBy.eq(user_id))
            .all(read)
            .await?;
        Ok(json!({
            "lists": lists.iter().map(|row| json!({
                "id": row.id,
                "key": row.key,
                "created_at": row.created_at,
            })).collect::<Vec<_>>(),
            "campaigns": campaigns.iter().map(|row| json!({
                "id": row.id,
                "subject": row.subject,
                "status": row.status,
                "created_at": row.created_at,
            })).collect::<Vec<_>>(),
        }))
    }
}

#[async_trait::async_trait]
impl EventHandler for NewsletterService {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(user_id) = deleted_user(event) else {
            return Ok(());
        };
        let write = self.database.write();
        let lists = list::Entity::update_many()
            .col_expr(list::Column::CreatedBy, Expr::value(Option::<Uuid>::None))
            .filter(list::Column::CreatedBy.eq(user_id))
            .exec(write)
            .await?
            .rows_affected;
        let campaigns = campaign::Entity::update_many()
            .col_expr(campaign::Column::CreatedBy, Expr::value(Option::<Uuid>::None))
            .filter(campaign::Column::CreatedBy.eq(user_id))
            .exec(write)
            .await?
            .rows_affected;
        tracing::info!(user_id = %user_id, lists, campaigns, "newsletter items of a deleted user unlinked");
        Ok(())
    }
}
