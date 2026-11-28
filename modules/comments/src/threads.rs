//! Đọc công khai: luồng của một đích, trả lời của một bình luận, một bình luận.
//!
//! - Luồng: bình luận gốc đang hiện (và bình luận gốc đã xoá mà còn trả lời — giữ chỗ trong
//!   luồng), cursor theo UUIDv7 (mới / cũ trước), mỗi bình luận gốc kèm vài trả lời đầu (cũ
//!   trước) và `reply_count`.
//! - Trả lời: cursor, cũ trước mặc định.
//! - Đích bị ẩn (`hidden`) thì không có gì công khai.
//! - Tìm (`q`, D-202): bình luận gốc có chữ hay tên khớp, không phân biệt hoa thường và dấu;
//!   khớp ĐÚNG dấu xếp trước khớp không dấu (`Find::page_tiers`), trong mỗi tầng theo `order`.
//!
//! Trả lời đầu của mỗi bình luận gốc lấy bằng một câu truy vấn nhỏ có index `(root_id, status,
//! id)` cho mỗi bình luận gốc có trả lời (tối đa 200 / trang); chưa đo thấy chậm.

use autoxide_kernel::db::{Cond, Queries as _};
use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Page, PageRequest, Pagination};
use autoxide_kernel::search;
use autoxide_kernel::tenancy::TenantContext;
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
fn visible_root() -> Cond<comment::Model> {
    Cond::any().with(comment::Model::STATUS.eq(Status::Published.as_str().to_owned())).with(
        Cond::all()
            .with(comment::Model::STATUS.eq(Status::Deleted.as_str().to_owned()))
            .with(comment::Model::REPLY_COUNT.gt(0)),
    )
}

impl CommentsService {
    /// Một trang bình luận gốc của đích, mỗi cái kèm tối đa `replies` trả lời đầu. `search`:
    /// chỉ bình luận gốc có chữ hay tên chứa chuỗi này (đã kiểm độ dài), đúng dấu xếp trước.
    pub(crate) async fn thread(
        &self,
        tenant: &TenantContext,
        kind: &str,
        id: &str,
        page: &PageRequest,
        replies: u32,
        search: Option<&str>,
    ) -> Result<Page<ThreadRoot>, Error> {
        let target = self.target(tenant, kind, id).await?;
        if target.hidden {
            let pagination =
                Pagination::new(page.limit(), None).with_total(page.wants_total().then_some(0));
            return Ok(Page { data: Vec::new(), pagination });
        }
        let read = tenant.db().read();
        let select = read
            .find::<comment::Model>()
            .filter(comment::Model::TARGET_TYPE.eq(kind.to_owned()))
            .filter(comment::Model::TARGET_ID.eq(id.to_owned()))
            .filter(comment::Model::DEPTH.eq(0))
            .filter(visible_root());
        let Page { data: roots, pagination } = match search {
            None => select.page(comment::Model::ID, page).await?,
            Some(text) => {
                // Hai cột lưu sẵn dạng chữ thường (`exact_key` giữ dấu, `fold` bỏ dấu): so chuỗi tìm cùng dạng.
                let tiers = [
                    comment::Model::MATCH_KEY.contains(&search::exact_key(text)),
                    comment::Model::SEARCH_KEY.contains(&search::fold(text)),
                ];
                select.page_tiers(&tiers, comment::Model::ID, page).await?
            }
        };
        let replies = replies.min(REPLIES_MAX);
        let mut all = Vec::new();
        let mut counts = Vec::new();
        for root in &roots {
            let found = if replies > 0 && root.reply_count > 0 {
                read.find::<comment::Model>()
                    .filter(comment::Model::ROOT_ID.eq(root.id))
                    .filter(comment::Model::DEPTH.eq(1))
                    .filter(comment::Model::STATUS.eq(Status::Published.as_str().to_owned()))
                    .order_asc(comment::Model::ID)
                    .limit(u64::from(replies))
                    .all()
                    .await?
            } else {
                Vec::new()
            };
            counts.push(found.len());
            all.extend(found);
        }
        let mut named_roots = self.named(tenant, roots).await?.into_iter();
        let mut named_replies = self.named(tenant, all).await?.into_iter();
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
        tenant: &TenantContext,
        id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<Comment>, Error> {
        let root = self.public_row(tenant, id).await?;
        let Page { data, pagination } = tenant
            .db()
            .read()
            .find::<comment::Model>()
            .filter(comment::Model::ROOT_ID.eq(root.root_id))
            .filter(comment::Model::DEPTH.eq(1))
            .filter(comment::Model::STATUS.eq(Status::Published.as_str().to_owned()))
            .page(comment::Model::ID, page)
            .await?;
        Ok(Page { data: self.named(tenant, data).await?, pagination })
    }

    /// Bình luận `id` khi người lạ được thấy nó.
    pub(crate) async fn public_comment(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Comment, Error> {
        let row = self.public_row(tenant, id).await?;
        self.named(tenant, vec![row]).await?.into_iter().next().ok_or_else(|| comment_not_found(id))
    }

    async fn public_row(&self, tenant: &TenantContext, id: Uuid) -> Result<comment::Model, Error> {
        let row = self.find(tenant, id).await?;
        let shown = match Status::of(&row) {
            Status::Published => true,
            Status::Deleted => row.depth == 0 && row.reply_count > 0,
            _ => false,
        };
        if !shown || self.target(tenant, &row.target_type, &row.target_id).await?.hidden {
            return Err(comment_not_found(id));
        }
        Ok(row)
    }
}

/// Yêu cầu trang của danh sách trả lời: cũ trước khi client không nêu `order`.
pub(crate) fn oldest_first(order: Option<&str>) -> Option<String> {
    Some(order.map_or_else(|| "asc".to_owned(), str::to_owned))
}
