//! Chiến dịch cho bài mới: nghe event `content.entry_published` **bằng tên** (không phụ thuộc
//! crate `content`), lấy tiêu đề, tóm tắt, đường dẫn qua sổ đối tượng của kernel
//! (`resolve("content.entry", id)`, D-123), rồi với mỗi danh sách bật chế độ "bài mới":
//!
//! - `draft`: tạo chiến dịch nháp chờ người duyệt;
//! - `send`: tạo và gửi ngay (thiếu cấu hình cần cho gửi thì chỉ tạo nháp, ghi cảnh báo).
//!
//! Event này cũng phát khi bài đã đăng được đăng lại (bản công khai đổi), và có thể bị giao lại
//! (D-054): `(source, source_ref)` là unique, chèn bằng `ON CONFLICT DO NOTHING`, nên một bài
//! sinh nhiều nhất một chiến dịch cho mỗi danh sách — mãi mãi. Chỉ bài viết (`kind = post`) đang
//! công khai; trang (`page`) không sinh bản tin.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, EventHandler};
use autoxide_kernel::tenancy::TenantContext;
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter};
use uuid::Uuid;

use crate::audience::lists::{List, NewPosts};
use crate::campaigns::compose::NEW_POST;
use crate::campaigns::{CampaignStatus, blank, summary};
use crate::entity::{campaign, list};
use crate::events::{self, Change};
use crate::service::{NewsletterService, now};
use crate::text::Locale;

/// Tên event "đã đăng" của module `content`.
pub(crate) const ENTRY_PUBLISHED: &str = "content.entry_published";
/// Loại đối tượng trong sổ của kernel.
const ENTRY: &str = "content.entry";
/// `source` của chiến dịch sinh từ bài mới.
const SOURCE: &str = "content.entry";

pub(crate) struct NewPostHandler(pub Arc<NewsletterService>);

#[async_trait::async_trait]
impl EventHandler for NewPostHandler {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        if event.name != ENTRY_PUBLISHED {
            return Ok(());
        }
        let published = event.payload.get("published");
        let kind = published.and_then(|value| value.get("kind")).and_then(|kind| kind.as_str());
        if kind.is_some_and(|kind| kind != "post") {
            return Ok(());
        }
        let id = published
            .and_then(|value| value.get("entry_id"))
            .and_then(|id| id.as_str())
            .or_else(|| {
                event.subject.as_deref().and_then(|subject| subject.strip_prefix("content.entry:"))
            })
            .and_then(|id| id.parse::<Uuid>().ok());
        match id {
            Some(id) => self.0.new_post(id).await.map(|_| ()),
            None => Ok(()),
        }
    }
}

impl NewsletterService {
    /// Tạo chiến dịch cho bài `entry_id` ở mọi danh sách bật chế độ "bài mới". Trả số chiến dịch
    /// vừa tạo (0 khi đã có từ trước).
    pub(crate) async fn new_post(&self, entry_id: Uuid) -> Result<usize, Error> {
        if !self.resources.knows(ENTRY) {
            return Ok(0);
        }
        let Some(resource) = self.resources.resolve(ENTRY, &entry_id.to_string()).await? else {
            return Ok(0);
        };
        if !resource.public {
            return Ok(0);
        }
        let rows = list::Entity::find()
            .filter(list::Column::NewPosts.ne(NewPosts::Off.as_str()))
            .all(self.database.read())
            .await?;
        let post_locale = resource.locale.as_deref().and_then(Locale::parse);
        let url = match (&self.config.site_url, &resource.url) {
            (Some(site), Some(path)) => Some(format!("{site}{path}")),
            _ => None,
        };
        let mut created = 0;
        for row in rows {
            let list = List::parse(row)?;
            if list.new_post_locale().is_some_and(|wanted| Some(wanted) != post_locale) {
                continue;
            }
            let locale = post_locale.unwrap_or_default();
            let link = url.clone().unwrap_or_default();
            let params = [
                ("title", resource.title.as_str()),
                ("summary", resource.summary.as_str()),
                ("url", link.as_str()),
            ];
            let rendered = NEW_POST.render(&self.translations, locale.language(), &params);
            let now = now();
            let mut row = blank(list.row.id, now);
            row.subject = rendered.subject.chars().take(200).collect();
            row.body_text = rendered.text;
            row.locale = locale.as_str().to_owned();
            row.source = Some(SOURCE.to_owned());
            row.source_ref = Some(format!("{entry_id}:{}", list.row.id));
            // Gửi ngay chỉ khi đủ cấu hình (link tới bài, URL công khai, trang lựa chọn).
            let send = list.new_posts() == NewPosts::Send
                && url.is_some()
                && self.check_ready(&row).is_ok();
            if list.new_posts() == NewPosts::Send && !send {
                tracing::warn!(list_id = %list.row.id, "new-post campaign kept as a draft: set AUTOXIDE_NEWSLETTER_SITE_URL, AUTOXIDE_HTTP_PUBLIC_URL and AUTOXIDE_NEWSLETTER_PREFERENCES_URL to send it at once");
            }
            if send {
                row.status = CampaignStatus::Sending.as_str().to_owned();
                row.started_at = Some(now);
                row.run = 1;
            }
            let transaction = self.database.begin_write().await?;
            let inserted = campaign::Entity::insert(row.clone().into_active_model())
                .on_conflict(
                    OnConflict::columns([campaign::Column::Source, campaign::Column::SourceRef])
                        .do_nothing()
                        .to_owned(),
                )
                .exec_without_returning(&transaction)
                .await?;
            if inserted == 0 {
                transaction.rollback().await?;
                continue;
            }
            let subject = events::campaign_subject(row.id);
            let change = Change { before: None, after: Some(summary(&row)) };
            self.outbox
                .record(
                    &transaction,
                    &TenantContext::legacy_default(&self.database),
                    Actor::system(),
                    subject.clone(),
                    &events::CampaignCreated(change),
                )
                .await?;
            if send {
                self.enqueue_prepare(&transaction, row.id, row.run, None, None).await?;
                let change = Change { before: None, after: Some(summary(&row)) };
                self.outbox
                    .record(
                        &transaction,
                        &TenantContext::legacy_default(&self.database),
                        Actor::system(),
                        subject,
                        &events::CampaignStarted(change),
                    )
                    .await?;
            }
            transaction.commit().await?;
            created += 1;
            tracing::info!(campaign_id = %row.id, list_id = %list.row.id, send, "newsletter campaign for a new post");
        }
        Ok(created)
    }
}
