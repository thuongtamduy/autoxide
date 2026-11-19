//! Tin nhắn: gửi (chống trùng theo `client_id` của máy gửi), danh sách, thu hồi trong
//! [`RECALL_WINDOW`], "đang gõ". Tin chỉ thêm — không sửa được.

use std::time::Duration;

use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::EnqueueOptions;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use autoxide_kernel::throttle::Throttle;
use chrono::DateTime;
use chrono::Utc;
use sea_orm::sea_query::{Expr, ExprTrait as _, OnConflict};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter};
use serde_json::json;
use uuid::Uuid;

use crate::assist::AutoReplyJob;
use crate::entity::{block, message, thread};
use crate::events::{self, Change};
use crate::notify::NotifyJob;
use crate::permissions::REPLY;
use crate::service::{
    AuthorKind, CLOSED, ConversationsService, MODULE, Message, Thread, Viewer, audience,
    message_snapshot, now, target,
};
use crate::settings::Policy;
use crate::threads::actor_of;

/// Thu hồi được trong chừng này sau khi gửi (D-20261119-step61-conversations).
pub(crate) const RECALL_WINDOW: chrono::Duration = chrono::Duration::minutes(2);
/// Số ảnh tối đa của một tin.
pub(crate) const MEDIA_MAX: usize = 5;
/// Độ dài tối đa của `client_id`.
pub(crate) const CLIENT_ID_MAX: usize = 100;

/// Một tin cần gửi.
#[derive(Debug, Clone, Default)]
pub(crate) struct NewMessage {
    pub body: String,
    /// Mã máy gửi tự sinh cho tin này: gửi lại cùng mã (mất mạng, retry) không thêm tin thứ hai.
    pub client_id: String,
    pub media_ids: Vec<Uuid>,
}

impl ConversationsService {
    /// Kiểm tin; trả bản đã chuẩn hoá. Có ảnh thì chữ được để trống.
    pub(crate) fn check_message(
        &self,
        new: &NewMessage,
        problems: &mut Vec<FieldError>,
    ) -> NewMessage {
        let body = new.body.trim().to_owned();
        let max = self.config.max_body_chars;
        let length = body.chars().count();
        if length > max || (length == 0 && new.media_ids.is_empty()) {
            problems.push(
                FieldError::new("body", "length", "must be 1 to {max} characters, not only spaces")
                    .with_param("max", max),
            );
        }
        let client_id = new.client_id.trim().to_owned();
        if !(1..=CLIENT_ID_MAX).contains(&client_id.chars().count())
            || client_id.starts_with("auto:")
            || client_id.starts_with("call:")
        {
            problems.push(
                FieldError::new(
                    "client_id",
                    "invalid",
                    "must be 1 to {max} characters made by your app for this message",
                )
                .with_param("max", CLIENT_ID_MAX),
            );
        }
        if new.media_ids.len() > MEDIA_MAX {
            problems.push(
                FieldError::new("media_ids", "too_many", "at most {max} images per message")
                    .with_param("max", MEDIA_MAX),
            );
        }
        NewMessage { body, client_id, media_ids: new.media_ids.clone() }
    }

    /// Gửi một tin vào cuộc `thread_id`. Gửi lại cùng `client_id` trả tin cũ (`false`), không thêm.
    pub(crate) async fn send(
        &self,
        tenant: &TenantContext,
        viewer: Viewer<'_>,
        thread_id: Uuid,
        new: NewMessage,
    ) -> Result<(Message, bool), Error> {
        let thread = self.get(tenant, viewer, thread_id).await?;
        if let Viewer::Staff(principal) = viewer {
            principal.require_at(tenant.database().read(), &REPLY, target(&thread)).await?;
        }
        let mut problems = Vec::new();
        let new = self.check_message(&new, &mut problems);
        crate::service::finish(problems)?;
        if let Some(existing) = self.by_client_id(tenant, thread_id, &new.client_id).await? {
            return Ok((existing, false));
        }
        if thread.status == CLOSED {
            return Err(thread_closed(thread_id));
        }
        let (kind, owner) = match viewer {
            Viewer::Customer(user) => {
                self.guard_customer(tenant, user, OrgId::from_uuid(thread.org_id)).await?;
                (AuthorKind::Customer, Some(user))
            }
            Viewer::Staff(principal) => {
                self.throttle(tenant, principal.user_id()).await?;
                (AuthorKind::Agent, None)
            }
        };
        self.check_attachments(tenant, owner, &new.media_ids).await?;
        let policy = self.parts.settings.policy(tenant, OrgId::from_uuid(thread.org_id)).await?;
        let body = self.prepare_body(&policy, &new.body);
        let at = now();
        let transaction = tenant.database().begin_write().await?;
        let author = Some(viewer.user_id());
        let inserted = self
            .insert_message(&transaction, tenant, &thread, kind, author, body, &new, at)
            .await?;
        let Some(message) = inserted else {
            // Hai lần gửi cùng `client_id` cùng lúc: bên thua trả tin của bên thắng.
            transaction.rollback().await?;
            let existing = self.by_client_id(tenant, thread_id, &new.client_id).await?;
            return existing
                .map(|message| (message, false))
                .ok_or_else(|| Error::internal("a duplicate chat message disappeared"));
        };
        self.after_message(&transaction, tenant, actor_of(viewer), &thread, &message).await?;
        transaction.commit().await?;
        Ok((message, true))
    }

    async fn by_client_id(
        &self,
        tenant: &TenantContext,
        thread_id: Uuid,
        client_id: &str,
    ) -> Result<Option<Message>, Error> {
        Ok(message::Entity::find()
            .filter(message::Column::ThreadId.eq(thread_id))
            .filter(message::Column::ClientId.eq(client_id))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?)
    }

    /// Ghi tin và cập nhật cuộc trò chuyện (thời điểm, bộ đếm chưa đọc của phía bên kia) trong
    /// `connection`. Bộ đếm cộng bằng biểu thức trong câu `UPDATE`: hai tin cùng lúc không mất lượt.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn insert_message<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        thread: &Thread,
        kind: AuthorKind,
        author: Option<Uuid>,
        body: String,
        new: &NewMessage,
        at: DateTime<Utc>,
    ) -> Result<Option<Message>, Error> {
        self.insert_row(connection, tenant, thread, kind, author, body, new, false, None, at).await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn insert_row<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        thread: &Thread,
        kind: AuthorKind,
        author: Option<Uuid>,
        body: String,
        new: &NewMessage,
        auto_reply: bool,
        call_id: Option<Uuid>,
        at: DateTime<Utc>,
    ) -> Result<Option<Message>, Error> {
        let row = message::Model {
            id: Uuid::now_v7(),
            tenant_id: tenant.id().as_uuid(),
            thread_id: thread.id,
            author_kind: kind.as_str().to_owned(),
            author_user_id: author,
            body,
            media_ids: json!(new.media_ids).to_string(),
            client_id: new.client_id.clone(),
            call_id,
            auto_reply,
            recalled_at: None,
            created_at: at,
        };
        // Trùng `client_id` thì không chèn — `ON CONFLICT DO NOTHING`, không để lệnh lỗi trong
        // transaction (Postgres huỷ cả transaction sau một lệnh lỗi, D-158).
        let inserted = message::Entity::insert(row.clone().into_active_model())
            .on_conflict(
                OnConflict::columns([
                    message::Column::TenantId,
                    message::Column::ThreadId,
                    message::Column::ClientId,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(connection)
            .await?;
        if inserted == 0 {
            return Ok(None);
        }
        let mut update = thread::Entity::update_many()
            .col_expr(thread::Column::LastMessageAt, Expr::value(at))
            .col_expr(thread::Column::UpdatedAt, Expr::value(at));
        update = match kind {
            AuthorKind::Customer => update
                .col_expr(thread::Column::UnreadStaff, Expr::col(thread::Column::UnreadStaff).add(1)),
            AuthorKind::Agent => update
                .col_expr(
                    thread::Column::UnreadCustomer,
                    Expr::col(thread::Column::UnreadCustomer).add(1),
                )
                // Nhân viên đã trả lời: đếm lại trả lời tự động, không còn chờ người, bỏ bản nháp cũ.
                .col_expr(thread::Column::AutoRepliesInRow, Expr::value(0))
                .col_expr(thread::Column::NeedsAgent, Expr::value(false))
                .col_expr(thread::Column::DraftReply, Expr::value(Option::<String>::None))
                .col_expr(thread::Column::DraftFor, Expr::value(Option::<Uuid>::None)),
            AuthorKind::System => update.col_expr(
                thread::Column::UnreadCustomer,
                Expr::col(thread::Column::UnreadCustomer).add(1),
            ),
        };
        update.filter(thread::Column::Id.eq(thread.id)).scoped(tenant).exec(connection).await?;
        Ok(Some(row))
    }

    /// Việc đi kèm một tin mới, cùng transaction: event, job trả lời tự động (tin của người mua),
    /// job báo tin chưa đọc cho phía bên kia.
    pub(crate) async fn after_message<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        actor: Actor,
        thread: &Thread,
        message: &Message,
    ) -> Result<(), Error> {
        let event = events::MessageCreated(Change {
            before: None,
            after: Some(message_snapshot(message)),
            audience: audience(thread),
        });
        let subject = events::thread_subject(thread.id);
        self.parts.outbox.record(connection, tenant, actor, subject, &event).await?;
        let customer_wrote = message.author_kind == AuthorKind::Customer.as_str();
        if customer_wrote && !thread.auto_reply_paused && self.parts.responder.is_some() {
            let job = AutoReplyJob { thread_id: thread.id, message_id: message.id };
            let options = EnqueueOptions::default().unique_key(format!("auto:{}", message.id));
            self.parts.jobs.enqueue(connection, tenant, &job, options).await?;
        }
        let side = if customer_wrote { "staff" } else { "customer" };
        let minutes = i64::from(self.config.notify_after_minutes);
        let window = minutes * 60;
        let bucket = message.created_at.timestamp().div_euclid(window);
        let job = NotifyJob { thread_id: thread.id, side: side.to_owned(), message_id: message.id };
        let options = EnqueueOptions::default()
            .run_at(message.created_at + chrono::Duration::minutes(minutes))
            .unique_key(format!("notify:{}:{side}:{bucket}", thread.id));
        self.parts.jobs.enqueue(connection, tenant, &job, options).await?;
        Ok(())
    }

    /// Chữ sẽ lưu: che liên hệ khi nút bật.
    pub(crate) fn prepare_body(&self, policy: &Policy, body: &str) -> String {
        if policy.mask_contacts { crate::mask::mask(body, &policy.mask_terms) } else { body.to_owned() }
    }

    /// Người mua được viết tới nút `org` không: chưa bị chặn, đã xác thực khi nút / khách đòi, chưa
    /// vượt số tin mỗi phút.
    pub(crate) async fn guard_customer(
        &self,
        tenant: &TenantContext,
        customer: Uuid,
        org: OrgId,
    ) -> Result<(), Error> {
        let blocked = block::Entity::find()
            .filter(block::Column::OrgId.eq(org.as_uuid()))
            .filter(block::Column::UserId.eq(customer))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        if blocked.is_some() {
            return Err(Error::new(
                ErrorKind::Forbidden,
                "conversations.blocked",
                "This shop does not accept your messages.",
            ));
        }
        let policy = self.parts.settings.policy(tenant, org).await?;
        let verification = &self.parts.verification;
        let required = policy.require_verified || verification.required_everywhere(tenant).await?;
        if required && !verification.is_verified(tenant, customer).await? {
            return Err(Error::new(
                ErrorKind::Forbidden,
                "conversations.verification_required",
                "Verify your account (phone number) before writing to this shop.",
            ));
        }
        self.throttle(tenant, customer).await
    }

    /// Số tin mỗi phút của một người (mọi cuộc trò chuyện).
    async fn throttle(&self, tenant: &TenantContext, user: Uuid) -> Result<(), Error> {
        let limit = self.config.messages_per_minute;
        let throttle = Throttle::new(MODULE, limit, Duration::from_secs(60))?;
        match throttle.hit(tenant, &format!("send:{user}")).await? {
            autoxide_kernel::throttle::Outcome::Allowed => Ok(()),
            autoxide_kernel::throttle::Outcome::Limited { retry_after } => Err(Error::new(
                ErrorKind::TooManyRequests,
                "conversations.too_many_messages",
                "You send messages too fast ({limit} a minute). Wait {retry_after} seconds.",
            )
            .with_param("limit", limit)
            .with_param("retry_after", retry_after)),
        }
    }

    /// Ảnh đính kèm có trong thư viện `media`; `owner`: người phải là người tải lên (`None`: nhân
    /// viên — mọi ảnh của khách).
    pub(crate) async fn check_attachments(
        &self,
        tenant: &TenantContext,
        owner: Option<Uuid>,
        ids: &[Uuid],
    ) -> Result<(), Error> {
        if ids.is_empty() {
            return Ok(());
        }
        let Some(media) = &self.parts.media else {
            return Err(attachment_not_owned());
        };
        let found = media.uploaders(tenant, ids).await?;
        let owned = ids.iter().all(|id| {
            found.iter().any(|(known, uploader)| {
                known == id && owner.is_none_or(|owner| *uploader == Some(owner))
            })
        });
        if owned { Ok(()) } else { Err(attachment_not_owned()) }
    }

    /// Một trang tin của cuộc `thread_id` (cũ trước theo mặc định của `order`).
    pub(crate) async fn messages(
        &self,
        tenant: &TenantContext,
        viewer: Viewer<'_>,
        thread_id: Uuid,
        page: &PageRequest,
    ) -> Result<(Thread, Page<Message>), Error> {
        let thread = self.get(tenant, viewer, thread_id).await?;
        let select =
            message::Entity::find().filter(message::Column::ThreadId.eq(thread_id)).scoped(tenant);
        let page =
            fetch_page(select, message::Column::Id, |row| row.id, page, tenant.database().read())
                .await?;
        Ok((thread, page))
    }

    /// Thu hồi tin `message_id` của chính người gọi trong [`RECALL_WINDOW`]: nội dung và ảnh bị bỏ,
    /// dấu "đã thu hồi" giữ lại. Thu hồi lại tin đã thu hồi không đổi gì.
    pub(crate) async fn recall(
        &self,
        tenant: &TenantContext,
        viewer: Viewer<'_>,
        thread_id: Uuid,
        message_id: Uuid,
    ) -> Result<Message, Error> {
        let thread = self.get(tenant, viewer, thread_id).await?;
        let found = message::Entity::find_by_id(message_id)
            .filter(message::Column::ThreadId.eq(thread_id))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        let Some(before) = found.filter(|row| row.author_user_id == Some(viewer.user_id())) else {
            return Err(Error::not_found(
                "conversations.message_not_found",
                "There is no message `{id}` of yours in this conversation.",
            )
            .with_param("id", message_id));
        };
        if before.recalled_at.is_some() {
            return Ok(before);
        }
        let at = now();
        if at - before.created_at > RECALL_WINDOW {
            return Err(Error::new(
                ErrorKind::Conflict,
                "conversations.recall_expired",
                "A message can only be recalled within {minutes} minutes of sending it.",
            )
            .with_param("minutes", RECALL_WINDOW.num_minutes()));
        }
        let transaction = tenant.database().begin_write().await?;
        message::Entity::update_many()
            .col_expr(message::Column::Body, Expr::value(""))
            .col_expr(message::Column::MediaIds, Expr::value("[]"))
            .col_expr(message::Column::RecalledAt, Expr::value(Some(at)))
            .filter(message::Column::Id.eq(message_id))
            .filter(message::Column::RecalledAt.is_null())
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let after = message::Entity::find_by_id(message_id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| Error::internal("a recalled message disappeared"))?;
        let event = events::MessageRecalled(Change {
            before: Some(message_snapshot(&before)),
            after: Some(message_snapshot(&after)),
            audience: audience(&thread),
        });
        let subject = events::thread_subject(thread_id);
        self.parts.outbox.record(&transaction, tenant, actor_of(viewer), subject, &event).await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Báo "đang gõ" cho phía bên kia qua stream. Gọi dồn dập thì mỗi 3 giây chỉ phát một lần (lần
    /// khác bỏ qua, không lỗi).
    pub(crate) async fn typing(
        &self,
        tenant: &TenantContext,
        viewer: Viewer<'_>,
        thread_id: Uuid,
    ) -> Result<(), Error> {
        let thread = self.get(tenant, viewer, thread_id).await?;
        if thread.status == CLOSED {
            return Err(thread_closed(thread_id));
        }
        let throttle = Throttle::new(MODULE, 1, Duration::from_secs(3))?;
        let key = format!("typing:{}:{thread_id}", viewer.user_id());
        if !throttle.hit(tenant, &key).await?.is_allowed() {
            return Ok(());
        }
        let kind = if viewer.is_staff() { AuthorKind::Agent } else { AuthorKind::Customer };
        let event = events::Typing(Change {
            before: None,
            after: Some(json!({"thread_id": thread_id, "author_kind": kind.as_str(), "user_id": viewer.user_id()})),
            audience: audience(&thread),
        });
        let transaction = tenant.database().begin_write().await?;
        let subject = events::thread_subject(thread_id);
        self.parts.outbox.record(&transaction, tenant, actor_of(viewer), subject, &event).await?;
        transaction.commit().await?;
        Ok(())
    }
}

pub(crate) fn thread_closed(id: Uuid) -> Error {
    Error::new(
        ErrorKind::Conflict,
        "conversations.thread_closed",
        "Conversation `{id}` is closed. Start a new conversation to write again.",
    )
    .with_param("id", id)
}

pub(crate) fn attachment_not_owned() -> Error {
    Error::new(
        ErrorKind::Validation,
        "conversations.attachment_not_owned",
        "Attach only images you uploaded to the media library yourself.",
    )
}
