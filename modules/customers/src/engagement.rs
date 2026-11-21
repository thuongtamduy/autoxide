//! Giữ chân người mua (bước 47 mục 5): danh sách yêu thích (gộp danh sách của trình duyệt khi đăng
//! nhập), sản phẩm vừa xem, nhắc giỏ bỏ dở.
//!
//! Nhắc giỏ là tin QUẢNG CÁO (NĐ 91/2020/NĐ-CP — cần đối chiếu văn bản gốc): chỉ lên lịch khi người
//! mua có đồng ý `marketing` trong sổ đồng ý của `compliance`, kiểm lại ngay trước khi gửi, dừng ngay
//! khi rút đồng ý hay khi người đó đặt đơn; `notifications` áp khung giờ và link huỷ nhận.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_compliance::api::{ConsentKey, ConsentSubject};
use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::event::{Actor, Delivered, EventHandler};
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, root_of};
use autoxide_notifications::api::{
    Category, Channel, Notification, NotificationKind, NotificationTemplate, Recipient,
};
use autoxide_orders::api::events as orders;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{cart_reminder, recent_view, reminder_rule, wishlist_item};
use crate::events::{CartReminderSent, Change};
use crate::profiles::subject;
use crate::service::{CustomersService, now};

pub(crate) const SEND_CART_REMINDER: &str = "customers.send_cart_reminder";
pub(crate) const PURGE_RECENT_VIEWS: &str = "customers.purge_recent_views";

const SCHEDULED: &str = "scheduled";
const SENT: &str = "sent";
const CANCELLED: &str = "cancelled";
const SKIPPED: &str = "skipped_no_consent";

/// Tin nhắc giỏ: quảng cáo, chỉ gửi khi có đồng ý.
pub(crate) const CART_REMINDER: NotificationKind = NotificationKind::new(
    Category::Marketing,
    &[Channel::Email, Channel::Zns, Channel::WebPush, Channel::MobilePush, Channel::InApp],
    NotificationTemplate::new(
        "customers.cart_reminder",
        "You left items in your cart",
        "The items you chose are still waiting in your cart. Finish your order when you are ready.",
        "Items waiting in your cart",
    ),
);

fn wishlist_full(max: u64) -> Error {
    Error::new(
        ErrorKind::Validation,
        "customers.wishlist_full",
        "A wishlist holds at most {max} products.",
    )
    .with_param("max", max)
}

fn marketing() -> ConsentKey {
    ConsentKey::new("marketing", "", "")
}

/// Một dòng yêu thích người mua gửi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WishlistEntry {
    pub product_id: Uuid,
    pub variant_id: Option<Uuid>,
}

fn variant_key(variant: Option<Uuid>) -> String {
    variant.map(|id| id.to_string()).unwrap_or_default()
}

impl CustomersService {
    pub(crate) async fn wishlist(
        &self,
        tenant: &TenantContext,
        user: Uuid,
    ) -> Result<Vec<wishlist_item::Model>, Error> {
        let profile = self.me(tenant, user).await?;
        Ok(wishlist_item::Entity::find()
            .filter(wishlist_item::Column::ProfileId.eq(profile.id))
            .scoped(tenant)
            .order_by_desc(wishlist_item::Column::AddedAt)
            .limit(self.config.wishlist_max)
            .all(tenant.database().read())
            .await?)
    }

    /// Thêm các dòng vào danh sách (bỏ trùng). `strict`: vượt trần → lỗi; không thì thêm tới trần
    /// rồi thôi (gộp danh sách của trình duyệt).
    pub(crate) async fn add_to_wishlist(
        &self,
        tenant: &TenantContext,
        user: Uuid,
        entries: &[WishlistEntry],
        strict: bool,
    ) -> Result<Vec<wishlist_item::Model>, Error> {
        let profile = self.me(tenant, user).await?;
        let transaction = tenant.database().begin_write().await?;
        let mut count = wishlist_item::Entity::find()
            .filter(wishlist_item::Column::ProfileId.eq(profile.id))
            .scoped(tenant)
            .count(&transaction)
            .await?;
        for entry in entries {
            let key = variant_key(entry.variant_id);
            let exists = wishlist_item::Entity::find_by_id((
                tenant.id().as_uuid(),
                profile.id,
                entry.product_id,
                key.clone(),
            ))
            .one(&transaction)
            .await?
            .is_some();
            if exists {
                continue;
            }
            if count >= self.config.wishlist_max {
                if strict {
                    return Err(wishlist_full(self.config.wishlist_max));
                }
                break;
            }
            wishlist_item::Entity::insert(wishlist_item::ActiveModel {
                tenant_id: Set(tenant.id().as_uuid()),
                profile_id: Set(profile.id),
                product_id: Set(entry.product_id),
                variant_key: Set(key),
                variant_id: Set(entry.variant_id),
                added_at: Set(now()),
                baseline_price_minor: Set(None),
                baseline_currency: Set(None),
                baseline_at: Set(None),
                back_in_stock_alerted_at: Set(None),
                price_drop_alerted_at: Set(None),
            })
            .on_conflict(
                OnConflict::columns([
                    wishlist_item::Column::TenantId,
                    wishlist_item::Column::ProfileId,
                    wishlist_item::Column::ProductId,
                    wishlist_item::Column::VariantKey,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(&transaction)
            .await?;
            count += 1;
        }
        transaction.commit().await?;
        // Giá mốc cho báo giảm giá (D-233): đọc `catalog` sau commit. Lỗi đọc giá không làm hỏng
        // việc thêm; mốc được đặt ở lần đầu thấy giá.
        if let Err(error) = self.fill_baselines(tenant, profile.id).await {
            tracing::warn!(error = %error, "wishlist baseline prices not read");
        }
        self.wishlist(tenant, user).await
    }

    /// Bỏ sản phẩm (mọi biến thể) khỏi danh sách.
    pub(crate) async fn remove_from_wishlist(
        &self,
        tenant: &TenantContext,
        user: Uuid,
        product: Uuid,
    ) -> Result<(), Error> {
        let profile = self.me(tenant, user).await?;
        wishlist_item::Entity::delete_many()
            .filter(wishlist_item::Column::ProfileId.eq(profile.id))
            .filter(wishlist_item::Column::ProductId.eq(product))
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        Ok(())
    }

    pub(crate) async fn recently_viewed(
        &self,
        tenant: &TenantContext,
        user: Uuid,
    ) -> Result<Vec<recent_view::Model>, Error> {
        let profile = self.me(tenant, user).await?;
        Ok(recent_view::Entity::find()
            .filter(recent_view::Column::ProfileId.eq(profile.id))
            .filter(
                recent_view::Column::ViewedAt
                    .gte(now() - SignedDuration::from_hours(24 * self.config.recent_views_days)),
            )
            .scoped(tenant)
            .order_by_desc(recent_view::Column::ViewedAt)
            .limit(self.config.recent_views_max)
            .all(tenant.database().read())
            .await?)
    }

    /// Ghi một lần xem (xem lại: đưa lên đầu); giữ tối đa `RECENT_VIEWS_MAX` sản phẩm.
    pub(crate) async fn record_view(
        &self,
        tenant: &TenantContext,
        user: Uuid,
        product: Uuid,
    ) -> Result<(), Error> {
        let profile = self.me(tenant, user).await?;
        let transaction = tenant.database().begin_write().await?;
        recent_view::Entity::insert(recent_view::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            profile_id: Set(profile.id),
            product_id: Set(product),
            viewed_at: Set(now()),
        })
        .on_conflict(
            OnConflict::columns([
                recent_view::Column::TenantId,
                recent_view::Column::ProfileId,
                recent_view::Column::ProductId,
            ])
            .update_column(recent_view::Column::ViewedAt)
            .to_owned(),
        )
        .exec_without_returning(&transaction)
        .await?;
        let all: Vec<(Uuid, Timestamp)> = recent_view::Entity::find()
            .filter(recent_view::Column::ProfileId.eq(profile.id))
            .scoped(tenant)
            .select_only()
            .column(recent_view::Column::ProductId)
            .column(recent_view::Column::ViewedAt)
            .order_by_desc(recent_view::Column::ViewedAt)
            .into_tuple()
            .all(&transaction)
            .await?;
        let old: Vec<Uuid> = all
            .into_iter()
            .skip(usize::try_from(self.config.recent_views_max).unwrap_or(50))
            .map(|(id, _)| id)
            .collect();
        if !old.is_empty() {
            recent_view::Entity::delete_many()
                .filter(recent_view::Column::ProfileId.eq(profile.id))
                .filter(recent_view::Column::ProductId.is_in(old))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Xoá lần xem cũ hơn `RECENT_VIEWS_DAYS` của mọi khách trong database (job hằng ngày).
    pub(crate) async fn purge_recent_views<C: ConnectionTrait>(
        &self,
        connection: &C,
    ) -> Result<u64, Error> {
        let cutoff = now() - SignedDuration::from_hours(24 * self.config.recent_views_days);
        // tenant-scope: job hằng ngày, xoá lượt xem cũ của mọi khách trên database.
        Ok(recent_view::Entity::delete_many()
            .filter(recent_view::Column::ViewedAt.lt(cutoff))
            .exec(connection)
            .await?
            .rows_affected)
    }

    // --- Nhắc giỏ bỏ dở ----------------------------------------------------------------------

    pub(crate) async fn add_reminder_rule(
        &self,
        tenant: &TenantContext,
        delays: Vec<i64>,
        enabled: bool,
    ) -> Result<reminder_rule::Model, Error> {
        let mut problems = Vec::new();
        if delays.is_empty()
            || delays.len() > 5
            || delays.iter().any(|minutes| !(15..=10_080).contains(minutes))
        {
            problems.push(autoxide_kernel::error::FieldError::new(
                "delays_minutes",
                "invalid",
                "must be 1 to 5 delays of 15 to 10080 minutes",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let mut delays = delays;
        delays.sort_unstable();
        delays.dedup();
        let at = now();
        Ok(sea_orm::ActiveModelTrait::insert(
            reminder_rule::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                delays_minutes: Set(json!(delays).to_string()),
                enabled: Set(enabled),
                effective_from: Set(at),
                created_at: Set(at),
            },
            tenant.database().write(),
        )
        .await?)
    }

    pub(crate) async fn reminder_rules(
        &self,
        tenant: &TenantContext,
    ) -> Result<Vec<reminder_rule::Model>, Error> {
        Ok(reminder_rule::Entity::find()
            .scoped(tenant)
            .order_by_desc(reminder_rule::Column::EffectiveFrom)
            .limit(50)
            .all(tenant.database().read())
            .await?)
    }

    async fn has_marketing_consent(
        &self,
        tenant: &TenantContext,
        user: Uuid,
    ) -> Result<bool, Error> {
        match &self.consents {
            Some(consents) => {
                consents.is_granted(tenant.id(), &ConsentSubject::User(user), &marketing()).await
            }
            None => Ok(false),
        }
    }

    /// `orders.cart_abandoned`: lên lịch nhắc theo quy tắc hiện hành — chỉ khi có đồng ý.
    async fn cart_abandoned(&self, tenant: &TenantContext, after: &Value) -> Result<(), Error> {
        let (Some(cart), Some(user)) = (
            after["cart_id"].as_str().and_then(|id| id.parse::<Uuid>().ok()),
            after["buyer_user_id"].as_str().and_then(|id| id.parse::<Uuid>().ok()),
        ) else {
            // Khách vãng lai: không có đồng ý ghi trong sổ — không nhắc.
            return Ok(());
        };
        let Some(rule) = reminder_rule::Entity::find()
            .filter(reminder_rule::Column::EffectiveFrom.lte(now()))
            .scoped(tenant)
            .order_by_desc(reminder_rule::Column::EffectiveFrom)
            .one(tenant.database().read())
            .await?
            .filter(|rule| rule.enabled)
        else {
            return Ok(());
        };
        let org = after["org_id"]
            .as_str()
            .and_then(|id| id.parse::<Uuid>().ok())
            .map_or_else(|| root_of(tenant.id()), OrgId::from_uuid);
        let profile = self.profile_for_user(tenant, user, org, Actor::system()).await?;
        let consent = self.has_marketing_consent(tenant, user).await?;
        let delays: Vec<i64> = serde_json::from_str(&rule.delays_minutes).unwrap_or_default();
        let at = now();
        let transaction = tenant.database().begin_write().await?;
        let rows: Vec<(i32, i64)> = if consent {
            delays
                .iter()
                .enumerate()
                .map(|(step, minutes)| (i32::try_from(step).unwrap_or(0) + 1, *minutes))
                .collect()
        } else {
            vec![(0, 0)]
        };
        for (step, minutes) in rows {
            let run_at = at + SignedDuration::from_mins(minutes);
            let inserted = cart_reminder::Entity::insert(cart_reminder::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                cart_id: Set(cart),
                profile_id: Set(profile.id),
                user_id: Set(user),
                step: Set(step),
                run_at: Set(run_at),
                status: Set(if consent { SCHEDULED } else { SKIPPED }.to_owned()),
                created_at: Set(at),
                updated_at: Set(at),
            })
            .on_conflict(
                OnConflict::columns([
                    cart_reminder::Column::TenantId,
                    cart_reminder::Column::CartId,
                    cart_reminder::Column::Step,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(&transaction)
            .await?;
            if inserted == 1 && consent {
                let job = SendCartReminder { cart_id: cart, step };
                let options = EnqueueOptions::default()
                    .run_at(run_at)
                    .unique_key(format!("cart:{cart}:{step}"));
                self.jobs.enqueue(&transaction, tenant, &job, options).await?;
            }
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Huỷ mọi nhắc đang chờ của `user` (đặt đơn, rút đồng ý).
    pub(crate) async fn cancel_reminders(
        &self,
        tenant: &TenantContext,
        user: Uuid,
    ) -> Result<u64, Error> {
        Ok(cart_reminder::Entity::update_many()
            .col_expr(cart_reminder::Column::Status, Expr::value(CANCELLED))
            .col_expr(cart_reminder::Column::UpdatedAt, Expr::value(now()))
            .filter(cart_reminder::Column::UserId.eq(user))
            .filter(cart_reminder::Column::Status.eq(SCHEDULED))
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?
            .rows_affected)
    }

    /// Rút đồng ý `marketing`: người nào có nhắc đang chờ mà không còn đồng ý thì huỷ ngay.
    async fn consent_withdrawn(
        &self,
        tenant: &TenantContext,
        payload: &Value,
    ) -> Result<(), Error> {
        if payload["purpose"].as_str() != Some("marketing") {
            return Ok(());
        }
        let users: Vec<Uuid> = cart_reminder::Entity::find()
            .filter(cart_reminder::Column::Status.eq(SCHEDULED))
            .scoped(tenant)
            .select_only()
            .column(cart_reminder::Column::UserId)
            .distinct()
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        for user in users {
            if !self.has_marketing_consent(tenant, user).await? {
                self.cancel_reminders(tenant, user).await?;
            }
        }
        Ok(())
    }

    /// Gửi một lần nhắc: kiểm lại trạng thái và đồng ý ngay trước khi gửi; `dedupe_key` theo giỏ
    /// và bước nên chạy lại không gửi hai lần.
    async fn send_cart_reminder(
        &self,
        tenant: &TenantContext,
        cart: Uuid,
        step: i32,
    ) -> Result<(), Error> {
        let Some(row) = cart_reminder::Entity::find()
            .filter(cart_reminder::Column::CartId.eq(cart))
            .filter(cart_reminder::Column::Step.eq(step))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
        else {
            return Ok(());
        };
        if row.status != SCHEDULED {
            return Ok(());
        }
        if !self.has_marketing_consent(tenant, row.user_id).await? {
            self.cancel_reminders(tenant, row.user_id).await?;
            return Ok(());
        }
        let Some(notifier) = &self.notifier else { return Ok(()) };
        let notification = Notification::new(
            CART_REMINDER.template.key,
            Recipient::User(row.user_id),
            json!({"cart_id": cart}),
            format!("customers.cart_reminder:{cart}:{step}"),
        );
        notifier.notify(tenant.id(), notification).await?;
        let transaction = tenant.database().begin_write().await?;
        let changed = cart_reminder::Entity::update_many()
            .col_expr(cart_reminder::Column::Status, Expr::value(SENT))
            .col_expr(cart_reminder::Column::UpdatedAt, Expr::value(now()))
            .filter(cart_reminder::Column::Id.eq(row.id))
            .filter(cart_reminder::Column::Status.eq(SCHEDULED))
            .scoped(tenant)
            .exec(&transaction)
            .await?
            .rows_affected;
        if changed == 1 {
            self.outbox
                .record(
                    &transaction,
                    tenant,
                    Actor::system(),
                    Some(subject(row.profile_id)),
                    &CartReminderSent(Change::after(
                        json!({"profile_id": row.profile_id, "cart_id": cart, "step": step}),
                    )),
                )
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }
}

/// Job gửi một lần nhắc giỏ.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SendCartReminder {
    pub cart_id: Uuid,
    pub step: i32,
}

impl Job for SendCartReminder {
    const KIND: &'static str = SEND_CART_REMINDER;
}

pub(crate) struct SendCartReminderJob(pub Arc<CustomersService>);

#[async_trait::async_trait]
impl JobHandler for SendCartReminderJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let tenant =
            job.tenant.as_ref().ok_or_else(|| Error::internal("reminder job without tenant"))?;
        let input: SendCartReminder =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        self.0.send_cart_reminder(tenant, input.cart_id, input.step).await
    }
}

pub(crate) struct PurgeRecentViewsJob(pub Arc<CustomersService>);

#[async_trait::async_trait]
impl JobHandler for PurgeRecentViewsJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let removed = self.0.purge_recent_views(job.database.write()).await?;
        if removed > 0 {
            tracing::info!(removed, "old recently viewed products removed");
        }
        Ok(())
    }
}

/// Bên nghe `orders.cart_abandoned`, `orders.order_placed` (huỷ nhắc) và `compliance.consent_withdrawn`.
pub(crate) struct EngagementEvents(pub Arc<CustomersService>);

#[async_trait::async_trait]
impl EventHandler for EngagementEvents {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let tenant = &event.tenant;
        match event.name.as_str() {
            orders::CART_ABANDONED => self.0.cart_abandoned(tenant, &event.payload["after"]).await,
            orders::ORDER_PLACED => {
                if let Some(user) = event.payload["after"]["buyer_user_id"]
                    .as_str()
                    .and_then(|id| id.parse::<Uuid>().ok())
                {
                    self.0.cancel_reminders(tenant, user).await?;
                }
                Ok(())
            }
            autoxide_compliance::api::CONSENT_WITHDRAWN => {
                self.0.consent_withdrawn(tenant, &event.payload).await?;
                self.0.alert_consent_withdrawn(tenant, &event.payload).await
            }
            _ => Ok(()),
        }
    }
}
