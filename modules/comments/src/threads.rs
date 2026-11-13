//! Đọc công khai: luồng của một đích, trả lời của một bình luận, một bình luận.
//!
//! - Luồng: bình luận gốc đang hiện (và bình luận gốc đã xoá mà còn trả lời — giữ chỗ trong
//!   luồng), cursor theo UUIDv7 (mới / cũ trước), mỗi bình luận gốc kèm vài trả lời đầu (cũ
//!   trước) và `reply_count`.
//! - Trả lời: cursor, cũ trước mặc định.
//! - Đích bị ẩn (`hidden`) thì không có gì công khai.
//!
//! Trả lời đầu của mỗi bình luận gốc lấy bằng một câu truy vấn nhỏ có index `(root_id, status,
//! id)` cho mỗi bình luận gốc có trả lời (tối đa 200 / trang); chưa đo thấy chậm.

use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Page, PageRequest, Pagination, fetch_page};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use uuid::Uuid;

use crate::entity::comment;
use crate::service::{Comment, CommentsService, Status, comment_not_found};

/// Số trả lời kèm mỗi bình luận gốc tối đa.
pub(crate) const REPLIES_MAX: u32 = 10;

/// Một bình luận gốc kèm vài trả lời đầu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ThreadRoot {
    pub root: Comment,
    pub replies: Vec<Comment>,
}

/// Bình luận gốc được thấy công khai: đang hiện, hay đã xoá mà còn trả lời.
fn visible_root() -> Condition {
    Condition::any().add(comment::Column::Status.eq(Status::Published.as_str())).add(
        Condition::all()
            .add(comment::Column::Status.eq(Status::Deleted.as_str()))
            .add(comment::Column::ReplyCount.gt(0)),
    )
}

impl CommentsService {
    /// Một trang bình luận gốc của đích, mỗi cái kèm tối đa `replies` trả lời đầu.
    pub(crate) async fn thread(
        &self,
        kind: &str,
        id: &str,
        page: &PageRequest,
        replies: u32,
    ) -> Result<Page<ThreadRoot>, Error> {
        let target = self.target(kind, id).await?;
        if target.hidden {
            return Ok(Page { data: Vec::new(), pagination: Pagination::new(page.limit(), None) });
        }
        let select = comment::Entity::find()
            .filter(comment::Column::TargetType.eq(kind))
            .filter(comment::Column::TargetId.eq(id))
            .filter(comment::Column::Depth.eq(0))
            .filter(visible_root());
        let Page { data: roots, pagination } =
            fetch_page(select, comment::Column::Id, |row| row.id, page, self.database.read())
                .await?;
        let replies = replies.min(REPLIES_MAX);
        let mut all = Vec::new();
        let mut counts = Vec::new();
        for root in &roots {
            let found = if replies > 0 && root.reply_count > 0 {
                comment::Entity::find()
                    .filter(comment::Column::RootId.eq(root.id))
                    .filter(comment::Column::Depth.eq(1))
                    .filter(comment::Column::Status.eq(Status::Published.as_str()))
                    .order_by_asc(comment::Column::Id)
                    .limit(u64::from(replies))
                    .all(self.database.read())
                    .await?
            } else {
                Vec::new()
            };
            counts.push(found.len());
            all.extend(found);
        }
        let mut named_roots = self.named(roots).await?.into_iter();
        let mut named_replies = self.named(all).await?.into_iter();
        let data = counts
            .into_iter()
            .filter_map(|count| {
                let root = named_roots.next()?;
                let replies = named_replies.by_ref().take(count).collect();
                Some(ThreadRoot { root, replies })
            })
            .collect();
        Ok(Page { data, pagination })
    }

    /// Một trang trả lời đang hiện của bình luận gốc `id` (cũ trước khi không nêu `order`).
    pub(crate) async fn replies(
        &self,
        id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<Comment>, Error> {
        let root = self.public_row(id).await?;
        let select = comment::Entity::find()
            .filter(comment::Column::RootId.eq(root.root_id))
            .filter(comment::Column::Depth.eq(1))
            .filter(comment::Column::Status.eq(Status::Published.as_str()));
        let Page { data, pagination } =
            fetch_page(select, comment::Column::Id, |row| row.id, page, self.database.read())
                .await?;
        Ok(Page { data: self.named(data).await?, pagination })
    }

    /// Bình luận `id` khi người lạ được thấy nó.
    pub(crate) async fn public_comment(&self, id: Uuid) -> Result<Comment, Error> {
        let row = self.public_row(id).await?;
        self.named(vec![row]).await?.into_iter().next().ok_or_else(|| comment_not_found(id))
    }

    async fn public_row(&self, id: Uuid) -> Result<comment::Model, Error> {
        let row = self.find(id).await?;
        let shown = match Status::of(&row) {
            Status::Published => true,
            Status::Deleted => row.depth == 0 && row.reply_count > 0,
            _ => false,
        };
        if !shown || self.target(&row.target_type, &row.target_id).await?.hidden {
            return Err(comment_not_found(id));
        }
        Ok(row)
    }
}

/// Yêu cầu trang của danh sách trả lời: cũ trước khi client không nêu `order`.
pub(crate) fn oldest_first(order: Option<&str>) -> Option<String> {
    Some(order.map_or_else(|| "asc".to_owned(), str::to_owned))
}
