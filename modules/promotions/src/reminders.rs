//! Nhắc khi flash sale bắt đầu (D-225). Người mua bấm "Nhắc tôi": một dòng
//! `promotions_reminders` kèm bằng chứng đồng ý riêng cho đúng lần nhắc đó trong sổ đồng ý của
//! `compliance` (bước 37) — không thành đồng ý nhận quảng cáo. Job `promotions.send_reminders`
//! gửi qua `notifications` trước giờ mở `REMINDER_LEAD_MINUTES` phút; mỗi lời nhắc gửi một lần.

use autoxide_compliance::api::{ConsentEvidence, ConsentKey, ConsentSubject};
use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use autoxide_notifications::api::{
    Category, Channel, Notification, NotificationKind, NotificationTemplate, Recipient,
};
use jiff::SignedDuration;
use sea_orm::QuerySelect;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
};
use serde::Serialize;
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::{promotion, reminder, translation};
use crate::model::Kind;
use crate::service::{PromotionsService, now};

/// Gửi trước giờ mở bấy nhiêu phút.
pub(crate) const REMINDER_LEAD_MINUTES: i64 = 10;
/// Số lời nhắc chưa gửi tối đa của một người.
pub(crate) const MAX_REMINDERS_PER_USER: u64 = 50;
/// Mục đích trong sổ đồng ý: chỉ cho lời nhắc này, không phải quảng cáo.
const CONSENT_PURPOSE: &str = "promotions.flash_sale_reminder";

/// Loại thông báo: người mua tự bấm "Nhắc tôi" nên là thông báo theo yêu cầu, không phải quảng cáo.
pub(crate) const FLASH_SALE_STARTING: NotificationKind = NotificationKind::new(
    Category::Transactional,
    &[Channel::InApp, Channel::WebPush, Channel::MobilePush, Channel::Email],
    NotificationTemplate::new(
        "promotions.flash_sale_starting",
        "{title} starts soon",
        "The flash sale {title} you asked to be reminded of starts in a few minutes.",
        "{title} starts soon",
    ),
);

/// A reminder of a flash sale.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = PromotionsReminder)]
pub(crate) struct ReminderView {
    pub promotion_id: Uuid,
    /// When the reminder is sent.
    pub remind_at: Timestamp,
    /// When it was sent; `null` until then.
    pub sent_at: Option<Timestamp>,
}

fn present(row: &reminder::Model) -> ReminderView {
    ReminderView { promotion_id: row.promotion_id, remind_at: row.remind_at, sent_at: row.sent_at }
}

fn source_ref(promotion: Uuid, user: Uuid) -> String {
    format!("promotions.reminder:{promotion}:{user}")
}

impl PromotionsService {
    /// "Nhắc tôi" cho flash sale `id` (công khai, chưa bắt đầu). Bấm lại: trả lời nhắc đã có.
    pub(crate) async fn remind_me(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Submitted<ReminderView>, Error> {
        let program = promotion::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .filter(|row| {
                row.public
                    && row.kind == Kind::FlashSale.as_str()
                    && matches!(row.status.as_str(), "scheduled" | "active")
            })
            .ok_or_else(|| crate::programs::not_found(id))?;
        let at = now();
        if program.starts_at <= at {
            return Err(Error::conflict(
                "promotions.flash_sale_started",
                "The flash sale has already started; there is nothing to remind you of.",
            ));
        }
        let user = principal.user_id();
        let write = tenant.database().write();
        if let Some(found) = find(tenant, write, id, user).await? {
            return Ok(Submitted::Replayed(present(&found)));
        }
        let pending = reminder::Entity::find()
            .filter(reminder::Column::UserId.eq(user))
            .filter(reminder::Column::SentAt.is_null())
            .scoped(tenant)
            .count(write)
            .await?;
        if pending >= MAX_REMINDERS_PER_USER {
            return Err(Error::conflict(
                "promotions.too_many_reminders",
                "You have {max} reminders waiting; remove one before adding another.",
            )
            .with_param("max", MAX_REMINDERS_PER_USER));
        }
        // Đồng ý ghi ở `compliance` (ngoài transaction của module này); gọi lại cùng `source_ref`
        // không ghi thêm.
        let consent_id = match &self.consents {
            Some(ledger) => Some(
                ledger
                    .grant(
                        tenant.id(),
                        ConsentSubject::User(user),
                        ConsentKey::new(CONSENT_PURPOSE, format!("promotions.promotion:{id}"), ""),
                        ConsentEvidence {
                            text: "Remind me once when this flash sale starts.".to_owned(),
                            text_version: "1".to_owned(),
                            source: "flash_sale_reminder".to_owned(),
                            ip_fingerprint: None,
                        },
                        &source_ref(id, user),
                    )
                    .await?
                    .id,
            ),
            None => None,
        };
        let remind_at =
            (program.starts_at - SignedDuration::from_mins(REMINDER_LEAD_MINUTES)).max(at);
        // Hai lần bấm cùng lúc: khoá duy nhất (khách, chương trình, người) giữ một dòng.
        reminder::Entity::insert(reminder::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            promotion_id: Set(id),
            user_id: Set(user),
            channel: Set("any".to_owned()),
            consent_id: Set(consent_id),
            remind_at: Set(remind_at),
            sent_at: Set(None),
            created_at: Set(at),
        })
        .on_conflict(
            OnConflict::columns([
                reminder::Column::TenantId,
                reminder::Column::PromotionId,
                reminder::Column::UserId,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(write)
        .await?;
        let row = find(tenant, write, id, user)
            .await?
            .ok_or_else(|| Error::internal("the reminder just written cannot be read back"))?;
        Ok(Submitted::Created(present(&row)))
    }

    /// Bỏ lời nhắc (chưa gửi) và rút đồng ý của nó. Không có thì không làm gì.
    pub(crate) async fn forget_reminder(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        let user = principal.user_id();
        let removed = reminder::Entity::delete_many()
            .filter(reminder::Column::PromotionId.eq(id))
            .filter(reminder::Column::UserId.eq(user))
            .filter(reminder::Column::SentAt.is_null())
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?
            .rows_affected;
        if removed > 0
            && let Some(ledger) = &self.consents
        {
            ledger
                .withdraw(
                    tenant.id(),
                    ConsentSubject::User(user),
                    ConsentKey::new(CONSENT_PURPOSE, format!("promotions.promotion:{id}"), ""),
                    "flash_sale_reminder",
                    &source_ref(id, user),
                )
                .await?;
        }
        Ok(())
    }

    /// Job: gửi lời nhắc tới hạn của khách. Gửi trước (cùng `dedupe_key` thì `notifications` không
    /// tạo tin thứ hai), rồi đánh dấu `sent_at` có điều kiện: hai worker không gửi hai lần.
    pub(crate) async fn send_reminders(&self, tenant: &TenantContext) -> Result<u64, Error> {
        let Some(notifier) = &self.notifier else {
            return Ok(0);
        };
        let at = now();
        let read = tenant.database().read();
        let due = reminder::Entity::find()
            .filter(reminder::Column::SentAt.is_null())
            .filter(reminder::Column::RemindAt.lte(at))
            .scoped(tenant)
            .order_by_asc(reminder::Column::RemindAt)
            .limit(500)
            .all(read)
            .await?;
        let mut sent = 0;
        for row in due {
            let program =
                promotion::Entity::find_by_id(row.promotion_id).scoped(tenant).one(read).await?;
            // Chương trình đã huỷ, kết thúc hay tạm dừng: không nhắc, chỉ đánh dấu xong.
            let live = program.as_ref().is_some_and(|found| {
                matches!(found.status.as_str(), "scheduled" | "active") && found.ends_at > at
            });
            if live && let Some(program) = &program {
                let title = translation::Entity::find()
                    .filter(translation::Column::PromotionId.eq(program.id))
                    .scoped(tenant)
                    .order_by_asc(translation::Column::Id)
                    .one(read)
                    .await?
                    .map(|text| text.title)
                    .unwrap_or_default();
                let mut notification = Notification::new(
                    FLASH_SALE_STARTING.key(),
                    Recipient::User(row.user_id),
                    json!({"title": title}),
                    format!("promotions.reminder:{}", row.id),
                );
                notification.link = Some(format!("/promotions/{}", program.id));
                notifier.notify(tenant.id(), notification).await?;
            }
            let marked = reminder::Entity::update_many()
                .col_expr(reminder::Column::SentAt, Expr::value(Some(at)))
                .filter(reminder::Column::Id.eq(row.id))
                .filter(reminder::Column::SentAt.is_null())
                .scoped(tenant)
                .exec(tenant.database().write())
                .await?
                .rows_affected;
            if live && marked > 0 {
                sent += 1;
            }
        }
        Ok(sent)
    }
}

/// Lời nhắc của người `user` cho chương trình `promotion` (đọc trên kết nối ghi: vừa ghi xong).
async fn find<C: sea_orm::ConnectionTrait>(
    tenant: &TenantContext,
    connection: &C,
    promotion: Uuid,
    user: Uuid,
) -> Result<Option<reminder::Model>, Error> {
    Ok(reminder::Entity::find()
        .filter(reminder::Column::PromotionId.eq(promotion))
        .filter(reminder::Column::UserId.eq(user))
        .scoped(tenant)
        .one(connection)
        .await?)
}
