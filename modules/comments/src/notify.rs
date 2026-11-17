//! Mail báo trả lời (chọn nhận): người viết gửi kèm `email` và `notify_replies: true` → mail xác
//! minh tới địa chỉ đó (token dùng một lần, `comments_tokens`, mục đích `verify`) → trang của
//! website gửi token tới `POST /public/verify` → dòng `comments_reply_subscriptions`. Khi một trả
//! lời được HIỆN (event `comments.comment_published` của chính module — kể cả khi được duyệt
//! sau), mỗi email theo dõi bình luận được trả lời nhận một mail.
//!
//! - Chỉ email ĐÃ XÁC MINH nhận mail báo; mỗi bình luận xác minh riêng (không ai đăng ký được
//!   email người khác bằng cách gõ nó vào).
//! - Mail gửi người lạ KHÔNG chứa chữ do người lạ gõ (hợp đồng chung §5): chỉ tiêu đề và đường
//!   dẫn của đích (do module sở hữu cấp, qua sổ đối tượng) và link — không chữ bình luận, không
//!   tên người trả lời, không ID đích do client gửi.
//! - Mỗi mail báo có token huỷ riêng (32 byte ngẫu nhiên, chỉ lưu bản băm, mục đích
//!   `unsubscribe`, sống 365 ngày) — như D-130 của `newsletter`.
//! - Event được giao lại (D-054): mail mang `dedupe_key` = trả lời + dòng theo dõi.
//! - Token trong link (xác minh, huỷ) mang khách của nó khi khách không phải khách mặc định
//!   (`autoxide_kernel::tenancy::link_token`, D-281): bấm link ở tên miền chính vẫn tới đúng khách.
//! - Giới hạn số mail xác minh tới một địa chỉ mỗi ngày bằng `Throttle` (`posting.rs`).

use std::sync::Arc;

use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::event::{Delivered, Event, EventHandler};
use autoxide_kernel::i18n::Language;
use autoxide_kernel::mail::MailTemplate;
use autoxide_kernel::resource::Resource;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext, link_token, resolve_link_token};
use autoxide_kernel::tokens::Issued;
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set};
use uuid::Uuid;

use crate::entity::{comment, subscription};
use crate::events::CommentPublished;
use crate::posting::VERIFY_HOURS;
use crate::service::{CommentsService, Status, now};

/// Mục đích của token xác minh email trong `comments_tokens`.
pub(crate) const VERIFY: &str = "verify";
/// Mục đích của token huỷ theo dõi.
pub(crate) const UNSUBSCRIBE: &str = "unsubscribe";
const UNSUBSCRIBE_DAYS: i64 = 365;

pub(crate) const VERIFY_MAIL: MailTemplate = MailTemplate::new(
    "comments.mail.verify",
    "Confirm your address for reply mails",
    "Someone asked, with this address, to get a mail when a comment on {target} gets a reply.\n\nConfirm it here: {link}\n\nThe link works for {hours} hours. If it was not you, ignore this mail: nothing more will be sent.\n",
);

pub(crate) const REPLY_MAIL: MailTemplate = MailTemplate::new(
    "comments.mail.reply",
    "New reply on {target}",
    "A comment you follow on {target} has a new reply.\n{page}\nStop these mails: {unsubscribe}\n",
);

fn link_invalid() -> Error {
    Error::bad_request(
        "comments.link_invalid",
        "This link is invalid, has expired or was already used.",
    )
}

/// Ngôn ngữ mail theo `locale` của bình luận.
fn language(locale: Option<&str>) -> Language {
    if locale == Some("vi") { Language::Vi } else { Language::En }
}

/// Tên đích trong mail: tiêu đề do module sở hữu cấp; không có thì một từ chung.
fn target_name(resource: Option<&Resource>, language: Language) -> String {
    match resource.map(|resource| resource.title.trim()).filter(|title| !title.is_empty()) {
        Some(title) => format!("\"{title}\""),
        None if language == Language::Vi => "website".to_owned(),
        None => "the website".to_owned(),
    }
}

/// Bên nhận event `comments.comment_published`: mail báo trả lời.
pub(crate) struct ReplyNotifier(pub Arc<CommentsService>);

#[async_trait::async_trait]
impl EventHandler for ReplyNotifier {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        if event.name != CommentPublished::NAME {
            return Ok(());
        }
        let id =
            event.payload.get("after").and_then(|after| after.get("id")).and_then(|id| id.as_str());
        match id.and_then(|id| id.parse::<Uuid>().ok()) {
            Some(id) => self.0.notify_reply(&event.tenant, id).await.map(|_| ()),
            None => Ok(()),
        }
    }
}

impl CommentsService {
    /// Đích của bình luận trong sổ đối tượng (tiêu đề, đường dẫn), khi loại có bộ tra.
    async fn resource_of(
        &self,
        tenant: &TenantContext,
        row: &comment::Model,
    ) -> Result<Option<Resource>, Error> {
        if !self.resources.knows(&row.target_type) {
            return Ok(None);
        }
        self.resources.resolve(tenant, &row.target_type, &row.target_id).await
    }

    /// Đường dẫn đầy đủ tới trang của đích, khi biết địa chỉ website và đường dẫn.
    async fn page_link(
        &self,
        tenant: &TenantContext,
        resource: Option<&Resource>,
    ) -> Result<Option<String>, Error> {
        let Some(site) = self.config.site_url.as_deref() else { return Ok(None) };
        // Khách đã đặt `kernel.public_url`: trang mở trên tên miền của khách.
        let site = self.site_links().rewrite(tenant, site).await?;
        let path = resource.and_then(|resource| resource.url.as_deref()).unwrap_or("");
        Ok(Some(format!("{site}{path}")))
    }

    /// Link theo tên miền của khách (`kernel.public_url`).
    fn site_links(&self) -> autoxide_kernel::tenancy::SiteLinks {
        autoxide_kernel::tenancy::SiteLinks::new(self.settings.clone())
    }

    /// Xếp mail xác minh `email` (token `issued`) cho bình luận `row`.
    pub(crate) async fn send_verification(
        &self,
        tenant: &TenantContext,
        row: &comment::Model,
        email: &str,
        issued: &Issued,
        resource: Option<&Resource>,
    ) -> Result<(), Error> {
        let Some(url) = self.config.verify_url.as_deref() else {
            return Ok(());
        };
        let language = language(row.locale.as_deref());
        // Link mang khách của token (`autoxide_kernel::tenancy::link_token`).
        let url = self.site_links().rewrite(tenant, url).await?;
        let link = url.replace("{token}", &link_token(tenant, &issued.token));
        let target = target_name(resource, language);
        let hours = VERIFY_HOURS.to_string();
        let params =
            [("target", target.as_str()), ("link", link.as_str()), ("hours", hours.as_str())];
        let mut mail = VERIFY_MAIL.mail(email, &self.translations, language, &params);
        mail.dedupe_key = Some(format!("comments.verify:{}", issued.id));
        self.mailer.send(tenant, mail).await
    }

    /// Xác minh email bằng token trong mail: bình luận được đánh dấu email đã xác minh và email đó
    /// theo dõi trả lời của nó. Khách lấy từ token khi token mang khách; không thì `request`
    /// (khách của request, `None`: khách mặc định) — xem
    /// `autoxide_kernel::tenancy::resolve_link_token`.
    pub(crate) async fn verify(
        &self,
        request: Option<&TenantContext>,
        token: &str,
    ) -> Result<(), Error> {
        let Some((resolved, raw)) = resolve_link_token(&self.databases, request, token).await?
        else {
            return Err(link_invalid());
        };
        let tenant = &resolved;
        let transaction = tenant.database().begin_write().await?;
        let Some(consumed) = self.tokens.consume(&transaction, tenant, VERIFY, raw).await? else {
            transaction.rollback().await?;
            return Err(link_invalid());
        };
        let id: Uuid = consumed.subject.parse().map_err(Error::internal)?;
        let email = consumed.data.unwrap_or_default();
        let found = comment::Entity::find_by_id(id).scoped(tenant).one(&transaction).await?;
        let Some(found) = found.filter(|row| row.author_email.as_deref() == Some(email.as_str()))
        else {
            // Bình luận đã bị xoá hay email đã bị xoá: token hết tác dụng.
            transaction.commit().await?;
            return Err(link_invalid());
        };
        let mut row = found.into_active_model();
        row.email_verified = Set(true);
        comment::Entity::update(row).validate()?.scoped(tenant).exec(&transaction).await?;
        let follow = subscription::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            comment_id: Set(id),
            email: Set(email),
            created_at: Set(now()),
        };
        subscription::Entity::insert(follow)
            .on_conflict(
                OnConflict::columns([
                    subscription::Column::TenantId,
                    subscription::Column::CommentId,
                    subscription::Column::Email,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(&transaction)
            .await?;
        transaction.commit().await?;
        tracing::info!(comment_id = %id, "reply mails confirmed");
        Ok(())
    }

    /// Huỷ theo dõi bằng token trong mail báo; `all`: mọi bình luận theo dõi bằng cùng email.
    /// Token lạ / đã dùng không phải lỗi (không lộ gì). Trả số dòng theo dõi đã xoá. Khách như
    /// [`Self::verify`].
    pub(crate) async fn unsubscribe(
        &self,
        request: Option<&TenantContext>,
        token: &str,
        all: bool,
    ) -> Result<u64, Error> {
        let Some((resolved, raw)) = resolve_link_token(&self.databases, request, token).await?
        else {
            return Ok(0);
        };
        let tenant = &resolved;
        let transaction = tenant.database().begin_write().await?;
        let Some(consumed) = self.tokens.consume(&transaction, tenant, UNSUBSCRIBE, raw).await?
        else {
            transaction.rollback().await?;
            return Ok(0);
        };
        let id: Uuid = consumed.subject.parse().map_err(Error::internal)?;
        let Some(found) =
            subscription::Entity::find_by_id(id).scoped(tenant).one(&transaction).await?
        else {
            transaction.commit().await?;
            return Ok(0);
        };
        let delete = if all {
            subscription::Entity::delete_many()
                .scoped(tenant)
                .filter(subscription::Column::Email.eq(found.email.as_str()))
        } else {
            subscription::Entity::delete_many()
                .scoped(tenant)
                .filter(subscription::Column::Id.eq(id))
        };
        let deleted = delete.exec(&transaction).await?.rows_affected;
        transaction.commit().await?;
        Ok(deleted)
    }

    /// Mail báo trả lời `id` (đã hiện) tới mọi email theo dõi bình luận được trả lời, trừ chính
    /// người trả lời. Trả số mail đã xếp hàng.
    pub(crate) async fn notify_reply(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<usize, Error> {
        let Some(unsubscribe_url) = self.config.unsubscribe_url.as_deref() else {
            return Ok(0);
        };
        let reply = match self.find(tenant, id).await {
            Ok(found) => found,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error),
        };
        let Some(followed_id) = reply.reply_to_id else {
            return Ok(0);
        };
        if Status::of(&reply) != Status::Published {
            return Ok(0);
        }
        let followers = subscription::Entity::find()
            .scoped(tenant)
            .filter(subscription::Column::CommentId.eq(followed_id))
            .all(tenant.database().read())
            .await?;
        if followers.is_empty() {
            return Ok(0);
        }
        let followed = comment::Entity::find_by_id(followed_id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        let language = language(followed.as_ref().and_then(|row| row.locale.as_deref()));
        let resource = self.resource_of(tenant, &reply).await?;
        let target = target_name(resource.as_ref(), language);
        let page = self
            .page_link(tenant, resource.as_ref())
            .await?
            .map(|link| format!("\n{link}\n"))
            .unwrap_or_default();
        let mut sent = 0;
        for follower in followers {
            if reply.author_email.as_deref() == Some(follower.email.as_str()) {
                continue;
            }
            let issued = self
                .tokens
                .issue(
                    tenant.database().write(),
                    tenant,
                    UNSUBSCRIBE,
                    &follower.id.to_string(),
                    None,
                    chrono::Duration::days(UNSUBSCRIBE_DAYS),
                )
                .await?;
            let unsubscribe = self
                .site_links()
                .rewrite(tenant, unsubscribe_url)
                .await?
                .replace("{token}", &link_token(tenant, &issued.token));
            let params = [
                ("target", target.as_str()),
                ("page", page.as_str()),
                ("unsubscribe", unsubscribe.as_str()),
            ];
            let mut mail = REPLY_MAIL.mail(&follower.email, &self.translations, language, &params);
            mail.dedupe_key = Some(format!("comments.reply:{id}:{}", follower.id));
            self.mailer.send(tenant, mail).await?;
            sent += 1;
        }
        Ok(sent)
    }
}
