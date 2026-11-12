//! Dữ liệu cá nhân trong module `content`.
//!
//! Tài khoản bị xoá (`privacy::USER_DELETED`): xoá hồ sơ tác giả gắn với người đó (cùng liên
//! kết của hồ sơ với các mục), bỏ trống `created_by` ở mọi bảng, huỷ lịch người đó đặt. Bài và
//! trang vẫn giữ: nội dung là của site, không của người viết. Nhận lại cùng event thì kết quả
//! không đổi (D-054).

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, EventHandler};
use autoxide_kernel::privacy::{PersonalData, deleted_user};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{author, entry, redirect, revision, schedule};
use crate::service::ContentService;

#[async_trait::async_trait]
impl PersonalData for ContentService {
    async fn export(&self, user_id: Uuid) -> Result<Value, Error> {
        let read = self.database.read();
        let authors = author::Entity::find()
            .filter(author::Column::UserId.eq(user_id))
            .order_by_asc(author::Column::Id)
            .all(read)
            .await?;
        let entries = entry::Entity::find()
            .filter(entry::Column::CreatedBy.eq(user_id))
            .order_by_asc(entry::Column::Id)
            .all(read)
            .await?;
        Ok(json!({
            "authors": authors.iter().map(|author| json!({
                "id": author.id,
                "display_name": author.display_name,
                "slug": author.slug,
                "bio": author.bio,
                "website_url": author.website_url,
                "created_at": author.created_at,
            })).collect::<Vec<_>>(),
            "entries": entries.iter().map(|entry| json!({
                "id": entry.id,
                "kind": entry.kind,
                "locale": entry.locale,
                "title": entry.title,
                "status": entry.status,
                "created_at": entry.created_at,
            })).collect::<Vec<_>>(),
        }))
    }
}

#[async_trait::async_trait]
impl EventHandler for ContentService {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(user_id) = deleted_user(event) else {
            return Ok(());
        };
        // Huỷ lịch trước khi bỏ `created_by`: sau đó không còn biết lịch nào của người này.
        let canceled = self.cancel_schedules_of(user_id).await?;
        let profiles = author::Entity::find()
            .filter(author::Column::UserId.eq(user_id))
            .all(self.database.read())
            .await?;
        for profile in &profiles {
            self.remove_author(profile.id, Actor::system()).await?;
        }
        // Mỗi lệnh tự đủ (không cần chung transaction): chạy lại giữa chừng vẫn ra cùng kết quả.
        let write = self.database.write();
        let none = || Expr::value(Option::<Uuid>::None);
        entry::Entity::update_many()
            .col_expr(entry::Column::CreatedBy, none())
            .filter(entry::Column::CreatedBy.eq(user_id))
            .exec(write)
            .await?;
        revision::Entity::update_many()
            .col_expr(revision::Column::CreatedBy, none())
            .filter(revision::Column::CreatedBy.eq(user_id))
            .exec(write)
            .await?;
        schedule::Entity::update_many()
            .col_expr(schedule::Column::CreatedBy, none())
            .filter(schedule::Column::CreatedBy.eq(user_id))
            .exec(write)
            .await?;
        redirect::Entity::update_many()
            .col_expr(redirect::Column::CreatedBy, none())
            .filter(redirect::Column::CreatedBy.eq(user_id))
            .exec(write)
            .await?;
        tracing::info!(user_id = %user_id, authors = profiles.len(), schedules = canceled, "content of a deleted user unlinked");
        Ok(())
    }
}
