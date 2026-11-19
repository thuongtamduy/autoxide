//! Mua thuê bao trong ứng dụng qua App Store và Google Play (bước 69 P4, D-20261119-step69-store).
//!
//! - Nhà vận hành nối mã sản phẩm của cửa hàng với một gói (`plans_store_product`).
//! - App của khách mua trong cửa hàng rồi gửi bằng chứng mua lên (`POST /plans/store/purchases`):
//!   kiểm với cửa hàng qua `dyn StorePurchases` (kernel), gắn chuỗi thuê bao của cửa hàng với khách
//!   (`plans_store_purchase`), rồi đặt thuê bao của khách lên gói đó tới lúc cửa hàng nói hết hạn.
//! - Cửa hàng báo gia hạn, hết hạn, hoàn tiền qua webhook có chữ ký
//!   (`POST /plans/store/notifications/{store}`); mỗi thông báo chỉ xử lý một lần
//!   (`plans_store_notification`).
//!
//! Thuê bao do cửa hàng thu tiền không có giá của nhà vận hành (`price_id` rỗng: không lập kỳ thanh
//! toán, không hoá đơn) và luôn `cancel_at_period_end`: cửa hàng không báo gia hạn thì vòng đời tự
//! huỷ khi hết kỳ — không bao giờ thành gói miễn phí. Hoàn tiền / thu hồi: huỷ ngay (chỉ đọc).
//! Tiền do Apple / Google thu và giữ (D-187).

use std::sync::Arc;

use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::event::Actor;
use autoxide_kernel::rows;
use autoxide_kernel::store::{AppStore, PurchaseState, StorePurchase, StorePurchases};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, TenantId};
use chrono::{DateTime, Duration, Utc};
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde_json::json;
use uuid::Uuid;

use crate::entity::{plan, subscription};
use crate::events;
use crate::service::{
    PlansService, invalid_transition, now, plan_not_active, plan_not_found, subscription_json,
};

pub(crate) mod entity {
    pub(crate) mod product {
        use sea_orm::entity::prelude::*;

        #[sea_orm::model]
        #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
        #[sea_orm(table_name = "plans_store_product")]
        pub struct Model {
            #[sea_orm(primary_key, auto_increment = false)]
            pub id: Uuid,
            /// `app_store` hay `google_play`.
            pub store: String,
            pub product_id: String,
            pub plan_id: Uuid,
            pub created_at: DateTimeUtc,
            pub updated_at: DateTimeUtc,
        }

        impl ActiveModelBehavior for ActiveModel {}
    }

    pub(crate) mod purchase {
        use sea_orm::entity::prelude::*;

        #[sea_orm::model]
        #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
        #[sea_orm(table_name = "plans_store_purchase")]
        pub struct Model {
            #[sea_orm(primary_key, auto_increment = false)]
            pub id: Uuid,
            pub tenant_id: Uuid,
            pub store: String,
            /// Mã gốc của chuỗi thuê bao ở cửa hàng: một chuỗi chỉ gắn với một khách.
            pub original_transaction_id: String,
            pub transaction_id: String,
            pub product_id: String,
            /// `active`, `expired`, `refunded` hay `revoked`.
            pub state: String,
            pub expires_at: Option<DateTimeUtc>,
            pub sandbox: bool,
            pub created_at: DateTimeUtc,
            pub updated_at: DateTimeUtc,
        }

        impl ActiveModelBehavior for ActiveModel {}
    }

    pub(crate) mod notification {
        use sea_orm::entity::prelude::*;

        #[sea_orm::model]
        #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
        #[sea_orm(table_name = "plans_store_notification")]
        pub struct Model {
            #[sea_orm(primary_key, auto_increment = false)]
            pub id: Uuid,
            pub store: String,
            pub notification_id: String,
            pub received_at: DateTimeUtc,
        }

        impl ActiveModelBehavior for ActiveModel {}
    }
}

use entity::{notification, product, purchase};

/// Bảng của phần này (mặt điều khiển, như mọi bảng `plans_*`).
pub(crate) const TABLES: [&str; 3] =
    ["plans_store_product", "plans_store_purchase", "plans_store_notification"];

fn state_name(state: PurchaseState) -> &'static str {
    match state {
        PurchaseState::Active => "active",
        PurchaseState::Expired => "expired",
        PurchaseState::Refunded => "refunded",
        PurchaseState::Revoked => "revoked",
    }
}

pub(crate) fn unknown_store(store: &str) -> Error {
    Error::not_found(
        "plans.store_unknown",
        "There is no app store `{store}`; use `app_store` or `google_play`.",
    )
    .with_param("store", store)
}

fn store_unavailable(store: AppStore) -> Error {
    Error::unavailable(
        "plans.store_unavailable",
        "Purchases through `{store}` are not set up on this server.",
    )
    .with_param("store", store.as_str())
}

fn product_unknown(store: AppStore, product: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "plans.store_product_unknown",
        "Product `{product}` of `{store}` is not linked to a plan. The operator links it with \
         `PUT /api/v1/plans/store/products/{store}/{product}`.",
    )
    .with_param("store", store.as_str())
    .with_param("product", product)
}

fn purchase_taken() -> Error {
    Error::conflict(
        "plans.store_purchase_taken",
        "This purchase already pays for another tenant's subscription.",
    )
}

fn purchase_inactive() -> Error {
    Error::conflict(
        "plans.store_purchase_inactive",
        "The store says this purchase is no longer active (expired, refunded or revoked).",
    )
}

pub(crate) fn store_billed() -> Error {
    Error::conflict(
        "plans.store_billed",
        "This subscription is paid through an app store: change, cancel or resume it in the store.",
    )
}

impl PlansService {
    fn store(&self, store: AppStore) -> Result<Arc<dyn StorePurchases>, Error> {
        self.stores
            .iter()
            .find(|connector| connector.store() == store)
            .cloned()
            .ok_or_else(|| store_unavailable(store))
    }

    /// Nối sản phẩm `product_id` của cửa hàng `store` với gói chính `plan_id` (thay nối cũ).
    pub(crate) async fn link_store_product(
        &self,
        store: AppStore,
        product_id: &str,
        plan_id: Uuid,
    ) -> Result<product::Model, Error> {
        let product_id = product_id.trim();
        if product_id.is_empty() || product_id.chars().count() > 200 {
            return Err(Error::validation(vec![autoxide_kernel::error::FieldError::new(
                "product_id",
                "length",
                "must be 1 to 200 characters",
            )]));
        }
        let transaction = self.databases.control().begin_write().await?;
        let chosen = plan::Entity::find_by_id(plan_id)
            .one(&transaction)
            .await?
            .ok_or_else(|| plan_not_found(plan_id))?;
        if chosen.kind != "base" {
            return Err(crate::service::addon_not_allowed(&chosen.code));
        }
        let at = now();
        product::Entity::insert(product::ActiveModel {
            id: Set(Uuid::now_v7()),
            store: Set(store.as_str().to_owned()),
            product_id: Set(product_id.to_owned()),
            plan_id: Set(plan_id),
            created_at: Set(at),
            updated_at: Set(at),
        })
        .on_conflict(
            OnConflict::columns([product::Column::Store, product::Column::ProductId])
                .update_columns([product::Column::PlanId, product::Column::UpdatedAt])
                .to_owned(),
        )
        .exec_without_returning(&transaction)
        .await?;
        let row = self.find_product(&transaction, store, product_id).await?;
        transaction.commit().await?;
        row.ok_or_else(|| Error::internal("store product vanished"))
    }

    async fn find_product<C: ConnectionTrait>(
        &self,
        connection: &C,
        store: AppStore,
        product_id: &str,
    ) -> Result<Option<product::Model>, Error> {
        Ok(product::Entity::find()
            .filter(product::Column::Store.eq(store.as_str()))
            .filter(product::Column::ProductId.eq(product_id))
            .one(connection)
            .await?)
    }

    /// Mọi sản phẩm đã nối, theo cửa hàng và mã.
    pub(crate) async fn store_products(&self) -> Result<Vec<product::Model>, Error> {
        Ok(product::Entity::find()
            .order_by_asc(product::Column::Store)
            .order_by_asc(product::Column::ProductId)
            .all(self.databases.control().read())
            .await?)
    }

    /// Bỏ nối sản phẩm (thuê bao đang chạy giữ gói cũ tới hết kỳ).
    pub(crate) async fn unlink_store_product(
        &self,
        store: AppStore,
        product_id: &str,
    ) -> Result<(), Error> {
        product::Entity::delete_many()
            .filter(product::Column::Store.eq(store.as_str()))
            .filter(product::Column::ProductId.eq(product_id))
            .exec(self.databases.control().write())
            .await?;
        Ok(())
    }

    /// Khách `tenant` gửi bằng chứng mua `proof` của cửa hàng `store`: kiểm với cửa hàng rồi đặt
    /// thuê bao. Gửi lại cùng giao dịch không cấp thêm gì.
    pub(crate) async fn claim_store_purchase(
        &self,
        actor: Actor,
        tenant: TenantId,
        store: AppStore,
        proof: &str,
    ) -> Result<(), Error> {
        if tenant == DEFAULT_TENANT {
            return Err(crate::service::platform_exempt());
        }
        // Gọi cửa hàng NGOÀI transaction ghi (luật dữ liệu).
        let purchase = self.store(store)?.verify(proof).await?;
        if purchase.state != PurchaseState::Active
            || purchase.expires_at.is_some_and(|end| end <= now())
        {
            return Err(purchase_inactive());
        }
        self.apply_store_purchase(actor, Some(tenant), &purchase).await
    }

    /// Thông báo máy chủ của cửa hàng `store` (đã kiểm chữ ký): áp trạng thái mới. Thông báo đã xử
    /// lý, hay của giao dịch không gắn với khách nào, thì bỏ qua.
    pub(crate) async fn store_notification(
        &self,
        store: AppStore,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> Result<(), Error> {
        let notification = self.store(store)?.notification(headers, body).await?;
        if notification.purchase.store != store {
            return Err(autoxide_kernel::store::signature_invalid());
        }
        let fresh = notification::Entity::insert(notification::ActiveModel {
            id: Set(Uuid::now_v7()),
            store: Set(store.as_str().to_owned()),
            notification_id: Set(notification.id.clone()),
            received_at: Set(now()),
        })
        .on_conflict(
            OnConflict::columns([
                notification::Column::Store,
                notification::Column::NotificationId,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(self.databases.control().write())
        .await?;
        if fresh == 0 {
            tracing::info!(store = store.as_str(), "store notification received again; skipped");
            return Ok(());
        }
        let applied =
            self.apply_store_purchase(Actor::system(), None, &notification.purchase).await;
        if applied.is_err() {
            // Xử lý hỏng: quên thông báo để cửa hàng gửi lại thì làm lại.
            notification::Entity::delete_many()
                .filter(notification::Column::Store.eq(store.as_str()))
                .filter(notification::Column::NotificationId.eq(notification.id))
                .exec(self.databases.control().write())
                .await?;
        }
        applied
    }

    /// Ghi giao dịch và đặt thuê bao theo nó. `tenant`: khách gửi bằng chứng (lần đầu gắn chuỗi);
    /// `None`: thông báo của cửa hàng — dùng khách đã gắn, chưa gắn ai thì bỏ qua.
    async fn apply_store_purchase(
        &self,
        actor: Actor,
        tenant: Option<TenantId>,
        bought: &StorePurchase,
    ) -> Result<(), Error> {
        let at = now();
        let transaction = self.databases.control().begin_write().await?;
        let existing = rows::lock_one(
            &transaction,
            purchase::Entity::find()
                .filter(purchase::Column::Store.eq(bought.store.as_str()))
                .filter(
                    purchase::Column::OriginalTransactionId.eq(&bought.original_transaction_id),
                ),
        )
        .await?;
        let owner = match (&existing, tenant) {
            (Some(row), Some(tenant)) if row.tenant_id != tenant.as_uuid() => {
                return Err(purchase_taken());
            }
            (Some(row), _) => TenantId::from_uuid(row.tenant_id),
            (None, Some(tenant)) => tenant,
            (None, None) => {
                tracing::info!(
                    store = bought.store.as_str(),
                    "store notification for an unknown purchase; skipped"
                );
                return Ok(());
            }
        };
        let linked = self.find_product(&transaction, bought.store, &bought.product_id).await?;
        let plan_id = match (&linked, bought.state) {
            (Some(row), _) => Some(row.plan_id),
            (None, PurchaseState::Active) => {
                return Err(product_unknown(bought.store, &bought.product_id));
            }
            (None, _) => None,
        };
        let row = purchase::ActiveModel {
            id: Set(existing.as_ref().map_or_else(Uuid::now_v7, |row| row.id)),
            tenant_id: Set(owner.as_uuid()),
            store: Set(bought.store.as_str().to_owned()),
            original_transaction_id: Set(bought.original_transaction_id.clone()),
            transaction_id: Set(bought.transaction_id.clone()),
            product_id: Set(bought.product_id.clone()),
            state: Set(state_name(bought.state).to_owned()),
            expires_at: Set(bought.expires_at),
            sandbox: Set(bought.sandbox),
            created_at: Set(existing.as_ref().map_or(at, |row| row.created_at)),
            updated_at: Set(at),
        };
        if existing.is_some() {
            row.update(&transaction).await?;
        } else {
            row.insert(&transaction).await?;
        }
        self.set_store_subscription(&transaction, actor, owner, bought, plan_id, at).await?;
        transaction.commit().await?;
        self.cache.forget(owner);
        tracing::info!(tenant_id = %owner, store = bought.store.as_str(), state = state_name(bought.state), "store purchase applied");
        Ok(())
    }

    /// Đặt thuê bao của `tenant` theo giao dịch `bought` (trong transaction của bên gọi).
    async fn set_store_subscription<C: ConnectionTrait>(
        &self,
        connection: &C,
        actor: Actor,
        tenant: TenantId,
        bought: &StorePurchase,
        plan_id: Option<Uuid>,
        at: DateTime<Utc>,
    ) -> Result<(), Error> {
        let subject = format!("plans.subscription:{tenant}");
        let current = rows::lock_one(
            connection,
            subscription::Entity::find()
                .filter(subscription::Column::TenantId.eq(tenant.as_uuid())),
        )
        .await?;
        let active = bought.state == PurchaseState::Active;
        let end = bought.expires_at.unwrap_or(at + Duration::days(36500));
        match (current, active) {
            (None, false) => return Ok(()),
            (None, true) => {
                let plan_id =
                    plan_id.ok_or_else(|| Error::internal("active purchase without a plan"))?;
                self.check_store_plan(connection, plan_id).await?;
                let row = subscription::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.as_uuid()),
                    plan_id: Set(plan_id),
                    price_id: Set(None),
                    status: Set("active".to_owned()),
                    current_period_start: Set(at),
                    current_period_end: Set(end),
                    trial_ends_at: Set(None),
                    grace_ends_at: Set(None),
                    suspended_at: Set(None),
                    canceled_at: Set(None),
                    delete_after: Set(None),
                    cancel_at_period_end: Set(true),
                    scheduled_plan_id: Set(None),
                    scheduled_price_id: Set(None),
                    trial_used: Set(false),
                    billing_email: Set(None),
                    created_at: Set(at),
                    updated_at: Set(at),
                }
                .insert(connection)
                .await?;
                let event = events::SubscriptionCreated(json!({
                    "tenant_id": tenant, "subscription": subscription_json(&row), "store": bought.store.as_str(),
                }));
                self.record(connection, actor, subject.clone(), &event).await?;
            }
            (Some(sub), _) if sub.status == "deleted" => {
                return Err(invalid_transition(&sub.status));
            }
            (Some(sub), true) => {
                let plan_id =
                    plan_id.ok_or_else(|| Error::internal("active purchase without a plan"))?;
                if plan_id != sub.plan_id {
                    self.check_store_plan(connection, plan_id).await?;
                }
                let before = subscription_json(&sub);
                let from = sub.status.clone();
                let renewing =
                    sub.status == "active" && sub.price_id.is_none() && sub.cancel_at_period_end;
                let mut row: subscription::ActiveModel = sub.into();
                row.plan_id = Set(plan_id);
                row.price_id = Set(None);
                row.status = Set("active".to_owned());
                if !renewing {
                    row.current_period_start = Set(at);
                }
                row.current_period_end = Set(end);
                row.cancel_at_period_end = Set(true);
                row.scheduled_plan_id = Set(None);
                row.scheduled_price_id = Set(None);
                row.grace_ends_at = Set(None);
                row.suspended_at = Set(None);
                row.canceled_at = Set(None);
                row.delete_after = Set(None);
                row.updated_at = Set(at);
                let row = row.update(connection).await?;
                let event = events::SubscriptionPlanChanged(json!({
                    "tenant_id": tenant, "before": before, "after": subscription_json(&row), "store": bought.store.as_str(),
                }));
                self.record(connection, actor, subject.clone(), &event).await?;
                self.status_event(connection, actor, &subject, tenant, &from, &row.status).await?;
            }
            (Some(sub), false) => {
                let from = sub.status.clone();
                let revoke =
                    matches!(bought.state, PurchaseState::Refunded | PurchaseState::Revoked);
                if from == "canceled" || !(revoke || sub.price_id.is_none()) {
                    return Ok(());
                }
                let retention = plan::Entity::find_by_id(sub.plan_id)
                    .one(connection)
                    .await?
                    .map_or(30, |row| row.retention_days);
                let mut row: subscription::ActiveModel = sub.into();
                if revoke {
                    // Hoàn tiền / thu hồi: thôi quyền dùng ngay (chỉ đọc, giữ dữ liệu).
                    row.status = Set("canceled".to_owned());
                    row.canceled_at = Set(Some(at));
                    row.delete_after = Set(Some(at + Duration::days(i64::from(retention))));
                } else {
                    // Hết hạn, không gia hạn: vòng đời huỷ khi hết kỳ.
                    row.cancel_at_period_end = Set(true);
                    row.current_period_end = Set(end.min(at));
                }
                row.updated_at = Set(at);
                let row = row.update(connection).await?;
                self.status_event(connection, actor, &subject, tenant, &from, &row.status).await?;
            }
        }
        let event = events::EntitlementsChanged(json!({ "tenant_id": tenant }));
        self.record(connection, actor, subject, &event).await
    }

    async fn check_store_plan<C: ConnectionTrait>(
        &self,
        connection: &C,
        plan_id: Uuid,
    ) -> Result<(), Error> {
        let chosen = plan::Entity::find_by_id(plan_id)
            .one(connection)
            .await?
            .ok_or_else(|| plan_not_found(plan_id))?;
        if chosen.status != "active" {
            return Err(plan_not_active(&chosen.code));
        }
        Ok(())
    }

    async fn status_event<C: ConnectionTrait>(
        &self,
        connection: &C,
        actor: Actor,
        subject: &str,
        tenant: TenantId,
        from: &str,
        to: &str,
    ) -> Result<(), Error> {
        if from == to {
            return Ok(());
        }
        let event =
            events::SubscriptionStatusChanged(json!({"tenant_id": tenant, "from": from, "to": to}));
        self.record(connection, actor, subject.to_owned(), &event).await
    }

    /// Khách tự đổi / huỷ / tiếp tục thuê bao: thuê bao đang do cửa hàng thu tiền thì phải làm trong
    /// cửa hàng (`plans.store_billed`) — không thì "tiếp tục" thành gói không ai thu tiền.
    pub(crate) async fn ensure_not_store_billed(&self, tenant: TenantId) -> Result<(), Error> {
        let read = self.databases.control().read();
        let bought = purchase::Entity::find()
            .filter(purchase::Column::TenantId.eq(tenant.as_uuid()))
            .one(read)
            .await?;
        if bought.is_none() {
            return Ok(());
        }
        // Thuê bao không có giá của nhà vận hành mà khách từng mua qua cửa hàng: kể cả khi giao
        // dịch đã hết hạn / hoàn tiền, "tiếp tục" ở đây sẽ thành gói miễn phí — mua lại trong cửa hàng.
        let sub = subscription::Entity::find()
            .filter(subscription::Column::TenantId.eq(tenant.as_uuid()))
            .one(read)
            .await?;
        match sub {
            Some(sub) if sub.price_id.is_none() => Err(store_billed()),
            _ => Ok(()),
        }
    }

    /// Giao dịch cửa hàng của `tenant`, mới trước.
    pub(crate) async fn store_purchases(
        &self,
        tenant: TenantId,
    ) -> Result<Vec<purchase::Model>, Error> {
        Ok(purchase::Entity::find()
            .filter(purchase::Column::TenantId.eq(tenant.as_uuid()))
            .order_by_desc(purchase::Column::UpdatedAt)
            .all(self.databases.control().read())
            .await?)
    }
}

/// Migration của phần mua trong ứng dụng.
pub(crate) fn migration() -> Box<dyn autoxide_kernel::migration::prelude::MigrationTrait> {
    Box::new(CreateStore)
}

struct CreateStore;

mod create {
    use autoxide_kernel::migration::prelude::*;

    impl MigrationName for super::CreateStore {
        fn name(&self) -> &str {
            "m0015_create_store"
        }
    }

    async fn index(
        manager: &SchemaManager<'_>,
        table: &str,
        name: &str,
        columns: &[&str],
        unique: bool,
    ) -> Result<(), DbErr> {
        let mut index = Index::create();
        index.name(name).table(Alias::new(table));
        for column in columns {
            index.col(Alias::new(*column));
        }
        if unique {
            index.unique();
        }
        manager.create_index(index.to_owned()).await
    }

    #[async_trait::async_trait]
    impl MigrationTrait for super::CreateStore {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .create_table(
                    Table::create()
                        .table("plans_store_product")
                        .col(schema::pk_uuid("id"))
                        .col(schema::string_len("store", 16))
                        .col(schema::string_len("product_id", 200))
                        .col(schema::uuid("plan_id"))
                        .col(schema::timestamp_with_time_zone("created_at"))
                        .col(schema::timestamp_with_time_zone("updated_at"))
                        .to_owned(),
                )
                .await?;
            index(
                manager,
                "plans_store_product",
                "plans_store_product_unique",
                &["store", "product_id"],
                true,
            )
            .await?;
            manager
                .create_table(
                    Table::create()
                        .table("plans_store_purchase")
                        .col(schema::pk_uuid("id"))
                        .col(schema::uuid("tenant_id"))
                        .col(schema::string_len("store", 16))
                        .col(schema::string_len("original_transaction_id", 200))
                        .col(schema::string_len("transaction_id", 200))
                        .col(schema::string_len("product_id", 200))
                        .col(schema::string_len("state", 16))
                        .col(schema::timestamp_with_time_zone_null("expires_at"))
                        .col(schema::boolean("sandbox"))
                        .col(schema::timestamp_with_time_zone("created_at"))
                        .col(schema::timestamp_with_time_zone("updated_at"))
                        .to_owned(),
                )
                .await?;
            index(
                manager,
                "plans_store_purchase",
                "plans_store_purchase_unique",
                &["store", "original_transaction_id"],
                true,
            )
            .await?;
            index(
                manager,
                "plans_store_purchase",
                "plans_store_purchase_tenant_idx",
                &["tenant_id", "state"],
                false,
            )
            .await?;
            manager
                .create_table(
                    Table::create()
                        .table("plans_store_notification")
                        .col(schema::pk_uuid("id"))
                        .col(schema::string_len("store", 16))
                        .col(schema::string_len("notification_id", 200))
                        .col(schema::timestamp_with_time_zone("received_at"))
                        .to_owned(),
                )
                .await?;
            index(
                manager,
                "plans_store_notification",
                "plans_store_notification_unique",
                &["store", "notification_id"],
                true,
            )
            .await
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            for table in super::TABLES.iter().rev() {
                manager.drop_table(Table::drop().table(Alias::new(*table)).to_owned()).await?;
            }
            Ok(())
        }
    }
}
