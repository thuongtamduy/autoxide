//! Service của module `plans`: gói, giá, thuê bao, gói thêm, ghi đè, và quyền dùng gộp lại của mỗi
//! khách. Handler REST, tool MCP và bên cung cấp `dyn Entitlements` chỉ gọi vào đây.
//!
//! Mọi bảng ở database điều khiển (`databases.control()`); event ghi dưới khách nền tảng.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use autoxide_kernel::access::Principal;
use autoxide_kernel::entitlement::{EntitlementsHandle, TenantAccess, TenantEntitlements};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::{Actor, Event, Outbox};
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, DatabaseRegistry, TenantId};
use chrono::{DateTime, Months, SubsecRound, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
    SqlErr,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{
    addon, grant, plan, plan_feature, plan_limit, plan_module, price, settings, subscription,
};
use crate::events;
use crate::merge::{self, Grants, Override, OverrideKind};

/// Giờ hiện tại, cắt ở micro giây (độ chính xác của cột thời gian trên cả hai database).
pub(crate) fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

/// Dữ liệu để tạo hay thay một gói.
#[derive(Debug, Clone, Default)]
pub(crate) struct PlanInput {
    pub code: String,
    pub name: String,
    pub description: String,
    pub kind: String,
    pub status: String,
    pub is_default: bool,
    pub trial_days: i32,
    pub grace_days: i32,
    pub suspend_days: i32,
    pub retention_days: i32,
    pub renewal_lead_days: i32,
    pub placement: String,
    pub requires_kyc: bool,
    pub sample_data: bool,
    pub self_signup: bool,
    pub modules: Vec<String>,
    pub features: Vec<String>,
    pub limits: BTreeMap<String, Option<i64>>,
}

/// Một gói kèm module, tính năng, hạn mức và giá của nó.
#[derive(Debug, Clone)]
pub(crate) struct PlanRow {
    pub plan: plan::Model,
    pub grants: Grants,
    pub prices: Vec<price::Model>,
    /// Số khách đang dùng gói (chỉ cho nhà vận hành; `None` ở catalog công khai).
    pub subscribers: Option<i64>,
}

/// Một giá mới của gói.
#[derive(Debug, Clone)]
pub(crate) struct PriceInput {
    pub kind: String,
    pub interval: String,
    pub interval_count: i32,
    pub amount: Money,
    pub limit_key: Option<String>,
    pub included: i64,
    pub tax_category: Option<String>,
}

/// Gán gói cho một khách (nhà vận hành).
#[derive(Debug, Clone)]
pub(crate) struct AssignInput {
    pub plan_id: Uuid,
    pub price_id: Option<Uuid>,
    /// Cho dùng thử nếu gói có và khách chưa dùng thử lần nào.
    pub trial: bool,
    /// `None`: giữ gói thêm đang có; `Some`: thay hẳn bằng danh sách này.
    pub addons: Option<Vec<(Uuid, i32)>>,
}

/// Thuê bao kèm gói thêm.
#[derive(Debug, Clone)]
pub(crate) struct SubscriptionRow {
    pub subscription: subscription::Model,
    pub addons: Vec<addon::Model>,
    /// Mã và tên của gói chính (kể cả gói đã ngừng bán).
    pub plan: Option<(String, String)>,
}

/// Một ghi đè mới.
#[derive(Debug, Clone)]
pub(crate) struct OverrideInput {
    pub kind: String,
    pub key: String,
    pub enabled: Option<bool>,
    pub max_value: Option<i64>,
    pub reason: String,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Lọc danh sách ghi đè của mọi khách.
#[derive(Debug, Clone, Default)]
pub(crate) struct OverrideFilter {
    pub tenant_id: Option<Uuid>,
    pub kind: Option<String>,
    pub key: Option<String>,
    pub active: Option<bool>,
}

/// Lọc danh sách thuê bao.
#[derive(Debug, Clone, Default)]
pub(crate) struct SubscriptionFilter {
    pub status: Option<String>,
    pub plan_id: Option<Uuid>,
    pub tenant_id: Option<Uuid>,
}

pub(crate) struct PlansService {
    pub databases: DatabaseRegistry,
    pub outbox: Outbox,
    pub entitlements: EntitlementsHandle,
    pub cache: Cache,
    /// Hạn của một chỗ giữ chưa xác nhận.
    pub hold: chrono::Duration,
    /// Sổ khách của kernel: khoá hẳn khách (`suspend`, xoá sau thời hạn lưu).
    pub tenants: Option<Arc<autoxide_kernel::tenancy::TenantAdmin>>,
    /// Gửi mail nhắc thanh toán (module `mail`); `None` thì không nhắc.
    pub mailer: Option<Arc<dyn autoxide_kernel::mail::Mailer>>,
    pub translations: autoxide_kernel::i18n::Translations,
    /// `AUTOXIDE_PLANS_REMINDER_DAYS`: nhắc trước hạn chừng này ngày.
    pub reminder_days: Vec<u32>,
    /// Xuất dữ liệu khách của kernel: tự xin một lượt khi thuê bao bị huỷ (D-328).
    pub exports: Option<Arc<autoxide_kernel::tenancy::TenantExports>>,
    /// Kiểm địa chỉ của hồ sơ xuất hoá đơn (module `address`); `None` thì không nhận địa chỉ.
    pub units: Option<Arc<dyn autoxide_address::api::AdministrativeUnits>>,
    /// Hàng đợi job của module (cấp phát khách chạy nền).
    pub jobs: autoxide_kernel::job::JobQueue,
    /// Mời quản trị viên của khách mới (module `auth`); `None` thì bước mời báo lỗi.
    pub invitations: Option<Arc<dyn autoxide_auth::api::Invitations>>,
    /// Lập hoá đơn thuê bao (module `invoices`); `None` thì kỳ không có hoá đơn.
    pub invoices: Option<Arc<dyn autoxide_invoices::api::Invoices>>,
    /// Thu tiền thuê bao qua cổng thanh toán (module `ledger`); `None` thì chỉ thu tay.
    pub ledger: Option<Arc<dyn autoxide_ledger::api::Ledger>>,
    /// `AUTOXIDE_PLANS_TAX_CATEGORY`: nhóm thuế của dòng thuê bao trên hoá đơn.
    pub tax_category: String,
    /// Thông báo tới quản trị viên khách (module `notifications`); `None` thì không báo.
    pub notifier: Option<Arc<dyn autoxide_notifications::api::Notifier>>,
    /// Định danh người đăng ký khách mới (module `compliance`); `None` thì gói `requires_kyc` không
    /// mở cho tự đăng ký.
    pub kyc: Option<Arc<dyn autoxide_compliance::api::KycGate>>,
    /// Ghi chấp nhận điều khoản của người đăng ký (module `compliance`); `None` thì bỏ qua.
    pub acceptance: Option<Arc<dyn autoxide_compliance::api::AcceptanceGate>>,
    /// Chống spam cho đăng ký công khai (kernel).
    pub guard: Arc<autoxide_kernel::antispam::SpamGuard>,
    /// `AUTOXIDE_PLANS_SIGNUP*`.
    pub signup: crate::config::SignupConfig,
}

/// Bộ nhớ đệm quyền dùng theo khách trong tiến trình (`AUTOXIDE_PLANS_CACHE_SECONDS`): quyền dùng
/// đổi ở máy chủ khác có hiệu lực ở đây sau tối đa chừng này.
pub(crate) struct Cache {
    ttl: Duration,
    // `Mutex` chuẩn: chỉ giữ trong vài phép tính, không giữ qua `.await`.
    entries: Mutex<HashMap<TenantId, (Instant, Arc<TenantEntitlements>)>>,
}

impl Cache {
    pub(crate) fn new(ttl: Duration) -> Self {
        Cache { ttl, entries: Mutex::default() }
    }

    fn get(&self, tenant: TenantId) -> Option<Arc<TenantEntitlements>> {
        if self.ttl.is_zero() {
            return None;
        }
        let entries = self.entries.lock().ok()?;
        entries
            .get(&tenant)
            .filter(|(at, _)| at.elapsed() < self.ttl)
            .map(|(_, value)| value.clone())
    }

    fn put(&self, tenant: TenantId, value: Arc<TenantEntitlements>) {
        if self.ttl.is_zero() {
            return;
        }
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(tenant, (Instant::now(), value));
        }
    }

    pub(crate) fn forget(&self, tenant: TenantId) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(&tenant);
        }
    }

    pub(crate) fn clear(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.clear();
        }
    }
}

impl PlansService {
    fn control(&self) -> &autoxide_kernel::database::Database {
        self.databases.control()
    }

    /// Ghi `event` dưới khách nền tảng, trong `connection` (transaction của thay đổi).
    pub(crate) async fn record<C: ConnectionTrait, E: Event>(
        &self,
        connection: &C,
        actor: Actor,
        subject: String,
        event: &E,
    ) -> Result<(), Error> {
        let platform = self.databases.default_tenant();
        self.outbox.record(connection, &platform, actor, Some(subject), event).await
    }

    // ---- Gói -------------------------------------------------------------------------------

    /// Mọi gói (cả nháp và đã ngừng bán), mới trước.
    pub(crate) async fn list_plans(&self, page: &PageRequest) -> Result<Page<PlanRow>, Error> {
        let read = self.control().read();
        let page =
            fetch_page(plan::Entity::find(), plan::Column::Id, |row| row.id, page, read).await?;
        let ids: Vec<Uuid> = page.data.iter().map(|row| row.id).collect();
        let mut details = self.details(&ids, false).await?;
        let counts = self.subscriber_counts(&ids).await?;
        Ok(page.map(|plan| {
            let (grants, prices) = details.remove(&plan.id).unwrap_or_default();
            let subscribers = Some(counts.get(&plan.id).copied().unwrap_or(0));
            PlanRow { plan, grants, prices, subscribers }
        }))
    }

    /// Gói `base` / `addon` đang bán, kèm giá đang dùng — cho trang đăng ký (công khai).
    pub(crate) async fn catalog(&self) -> Result<Vec<PlanRow>, Error> {
        let plans = plan::Entity::find()
            .filter(plan::Column::Status.eq("active"))
            .order_by_asc(plan::Column::Code)
            .all(self.control().read())
            .await?;
        let ids: Vec<Uuid> = plans.iter().map(|row| row.id).collect();
        let mut details = self.details(&ids, true).await?;
        Ok(plans
            .into_iter()
            .map(|plan| {
                let (grants, prices) = details.remove(&plan.id).unwrap_or_default();
                PlanRow { plan, grants, prices, subscribers: None }
            })
            .collect())
    }

    pub(crate) async fn get_plan(&self, id: Uuid) -> Result<PlanRow, Error> {
        let plan = plan::Entity::find_by_id(id)
            .one(self.control().read())
            .await?
            .ok_or_else(|| plan_not_found(id))?;
        let (grants, prices) = self.details(&[id], false).await?.remove(&id).unwrap_or_default();
        let subscribers = Some(self.subscriber_counts(&[id]).await?.get(&id).copied().unwrap_or(0));
        Ok(PlanRow { plan, grants, prices, subscribers })
    }

    /// Số khách đang dùng mỗi gói `ids`: thuê bao chưa huỷ có gói chính là nó, cộng thuê bao chưa
    /// huỷ giữ nó làm gói thêm. Hai câu `GROUP BY`, không phụ thuộc số gói.
    async fn subscriber_counts(&self, ids: &[Uuid]) -> Result<HashMap<Uuid, i64>, Error> {
        use sea_orm::{QuerySelect, QueryTrait};
        let mut counts: HashMap<Uuid, i64> = HashMap::new();
        if ids.is_empty() {
            return Ok(counts);
        }
        let read = self.control().read();
        let live = ["trialing", "active", "past_due", "suspended"];
        let base: Vec<(Uuid, i64)> = subscription::Entity::find()
            .select_only()
            .column(subscription::Column::PlanId)
            .column_as(subscription::Column::Id.count(), "subscribers")
            .filter(subscription::Column::PlanId.is_in(ids.iter().copied()))
            .filter(subscription::Column::Status.is_in(live))
            .group_by(subscription::Column::PlanId)
            .into_tuple()
            .all(read)
            .await?;
        // Gói thêm: chỉ của khách còn thuê bao chưa huỷ.
        let live_tenants = subscription::Entity::find()
            .select_only()
            .column(subscription::Column::TenantId)
            .filter(subscription::Column::Status.is_in(live))
            .into_query();
        let addons: Vec<(Uuid, i64)> = addon::Entity::find()
            .select_only()
            .column(addon::Column::PlanId)
            .column_as(addon::Column::TenantId.count(), "subscribers")
            .filter(addon::Column::PlanId.is_in(ids.iter().copied()))
            .filter(addon::Column::TenantId.in_subquery(live_tenants))
            .group_by(addon::Column::PlanId)
            .into_tuple()
            .all(read)
            .await?;
        for (id, count) in base.into_iter().chain(addons) {
            *counts.entry(id).or_default() += count;
        }
        Ok(counts)
    }

    /// Module, tính năng, hạn mức và giá của các gói `ids` (ba + một câu, không phụ thuộc số gói).
    async fn details(
        &self,
        ids: &[Uuid],
        active_prices_only: bool,
    ) -> Result<HashMap<Uuid, (Grants, Vec<price::Model>)>, Error> {
        let mut found: HashMap<Uuid, (Grants, Vec<price::Model>)> = HashMap::new();
        if ids.is_empty() {
            return Ok(found);
        }
        let read = self.control().read();
        let grants = self.grants(ids).await?;
        for (id, grants) in grants {
            found.entry(id).or_default().0 = grants;
        }
        let mut prices =
            price::Entity::find().filter(price::Column::PlanId.is_in(ids.iter().copied()));
        if active_prices_only {
            prices = prices.filter(price::Column::Active.eq(true));
        }
        for row in prices.order_by_asc(price::Column::Id).all(read).await? {
            found.entry(row.plan_id).or_default().1.push(row);
        }
        Ok(found)
    }

    /// Phần quyền dùng của các gói `ids`.
    async fn grants(&self, ids: &[Uuid]) -> Result<HashMap<Uuid, Grants>, Error> {
        let read = self.control().read();
        let mut found: HashMap<Uuid, Grants> =
            ids.iter().map(|id| (*id, Grants::default())).collect();
        let modules = plan_module::Entity::find()
            .filter(plan_module::Column::PlanId.is_in(ids.iter().copied()));
        for row in modules.all(read).await? {
            found.entry(row.plan_id).or_default().modules.insert(row.module);
        }
        let features = plan_feature::Entity::find()
            .filter(plan_feature::Column::PlanId.is_in(ids.iter().copied()));
        for row in features.all(read).await? {
            found.entry(row.plan_id).or_default().features.insert(row.feature);
        }
        let limits = plan_limit::Entity::find()
            .filter(plan_limit::Column::PlanId.is_in(ids.iter().copied()));
        for row in limits.all(read).await? {
            found.entry(row.plan_id).or_default().limits.insert(row.limit_key, row.max_value);
        }
        Ok(found)
    }

    pub(crate) async fn create_plan(
        &self,
        actor: Actor,
        input: PlanInput,
    ) -> Result<PlanRow, Error> {
        let input = self.check_plan(input)?;
        let id = Uuid::now_v7();
        let at = now();
        let transaction = self.control().begin_write().await?;
        if input.is_default {
            self.clear_default(&transaction).await?;
        }
        let row = plan::ActiveModel {
            id: Set(id),
            created_at: Set(at),
            updated_at: Set(at),
            ..active_plan(&input)
        };
        row.insert(&transaction).await.map_err(|error| code_taken(error, &input.code))?;
        write_grants(&transaction, id, &input).await?;
        let after = plan_json(id, &input);
        self.record(
            &transaction,
            actor,
            format!("plans.plan:{id}"),
            &events::PlanCreated(json!({ "plan": after })),
        )
        .await?;
        transaction.commit().await?;
        tracing::info!(plan_id = %id, code = %input.code, "plan created");
        self.get_plan(id).await
    }

    /// Thay toàn bộ gói `id` (không đổi được `kind` khi đã có khách dùng). Tính năng, hạn mức mới
    /// có hiệu lực với mọi khách đang dùng gói sau thời gian của bộ nhớ đệm.
    pub(crate) async fn update_plan(
        &self,
        actor: Actor,
        id: Uuid,
        input: PlanInput,
        expected: Option<DateTime<Utc>>,
    ) -> Result<PlanRow, Error> {
        let input = self.check_plan(input)?;
        let transaction = self.control().begin_write().await?;
        let current = rows::lock_one(&transaction, plan::Entity::find_by_id(id))
            .await?
            .ok_or_else(|| plan_not_found(id))?;
        if expected.is_some_and(|expected| expected != current.updated_at) {
            return Err(autoxide_kernel::version::changed());
        }
        if current.status == "retired" && input.status != "retired" {
            return Err(plan_not_active(&current.code));
        }
        if current.kind != input.kind && self.in_use(&transaction, id).await? {
            return Err(plan_in_use());
        }
        if input.is_default {
            self.clear_default(&transaction).await?;
        }
        let before =
            json!({ "code": current.code, "name": current.name, "status": current.status });
        let row = plan::ActiveModel {
            id: Set(id),
            created_at: Set(current.created_at),
            updated_at: Set(now()),
            ..active_plan(&input)
        };
        row.update(&transaction).await.map_err(|error| code_taken(error, &input.code))?;
        for table in [0, 1, 2] {
            match table {
                0 => {
                    plan_module::Entity::delete_many()
                        .filter(plan_module::Column::PlanId.eq(id))
                        .exec(&transaction)
                        .await?
                }
                1 => {
                    plan_feature::Entity::delete_many()
                        .filter(plan_feature::Column::PlanId.eq(id))
                        .exec(&transaction)
                        .await?
                }
                _ => {
                    plan_limit::Entity::delete_many()
                        .filter(plan_limit::Column::PlanId.eq(id))
                        .exec(&transaction)
                        .await?
                }
            };
        }
        write_grants(&transaction, id, &input).await?;
        let event =
            events::PlanUpdated(json!({ "before": before, "after": plan_json(id, &input) }));
        self.record(&transaction, actor, format!("plans.plan:{id}"), &event).await?;
        transaction.commit().await?;
        self.cache.clear();
        self.get_plan(id).await
    }

    /// Ngừng bán gói `id`: khách đang dùng vẫn giữ; không gán được cho khách mới.
    pub(crate) async fn retire_plan(&self, actor: Actor, id: Uuid) -> Result<PlanRow, Error> {
        let transaction = self.control().begin_write().await?;
        let current = rows::lock_one(&transaction, plan::Entity::find_by_id(id))
            .await?
            .ok_or_else(|| plan_not_found(id))?;
        if current.status != "retired" {
            let mut row: plan::ActiveModel = current.clone().into();
            row.status = Set("retired".to_owned());
            row.is_default = Set(false);
            row.updated_at = Set(now());
            row.update(&transaction).await?;
            let event = events::PlanRetired(json!({ "plan": { "id": id, "code": current.code } }));
            self.record(&transaction, actor, format!("plans.plan:{id}"), &event).await?;
        }
        transaction.commit().await?;
        self.cache.clear();
        self.get_plan(id).await
    }

    pub(crate) async fn add_price(
        &self,
        actor: Actor,
        plan_id: Uuid,
        input: PriceInput,
    ) -> Result<PlanRow, Error> {
        let mut problems = Vec::new();
        if !["recurring", "metered"].contains(&input.kind.as_str()) {
            problems.push(one_of("kind", &["recurring", "metered"]));
        }
        if !["month", "year"].contains(&input.interval.as_str()) {
            problems.push(one_of("interval", &["month", "year"]));
        }
        if !(1..=12).contains(&input.interval_count) {
            problems.push(range("interval_count", 1, 12));
        }
        if input.amount.is_negative() {
            problems.push(range("amount", 0, i64::MAX));
        }
        if input.included < 0 {
            problems.push(range("included", 0, i64::MAX));
        }
        match (&input.kind[..], &input.limit_key) {
            ("metered", Some(key)) => {
                let metered = self.entitlements.catalog().limit(key).is_some_and(|found| {
                    matches!(
                        found.limit.kind(),
                        autoxide_kernel::entitlement::LimitKind::Metered { .. }
                    )
                });
                if !metered {
                    problems.push(FieldError::new(
                        "limit_key",
                        "unknown",
                        "must be a metered limit a module declares",
                    ));
                }
            }
            ("metered", None) => problems.push(FieldError::new(
                "limit_key",
                "required",
                "a metered price needs `limit_key`",
            )),
            (_, Some(_)) => problems.push(FieldError::new(
                "limit_key",
                "not_allowed",
                "only a metered price has `limit_key`",
            )),
            _ => {}
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let transaction = self.control().begin_write().await?;
        let current = rows::lock_one(&transaction, plan::Entity::find_by_id(plan_id))
            .await?
            .ok_or_else(|| plan_not_found(plan_id))?;
        let id = Uuid::now_v7();
        price::ActiveModel {
            id: Set(id),
            plan_id: Set(plan_id),
            kind: Set(input.kind),
            interval: Set(input.interval),
            interval_count: Set(input.interval_count),
            amount_minor: Set(input.amount.minor()),
            amount_currency: Set(input.amount.currency().code().to_owned()),
            limit_key: Set(input.limit_key),
            included: Set(input.included),
            tax_category: Set(input.tax_category),
            active: Set(true),
            created_at: Set(now()),
        }
        .insert(&transaction)
        .await?;
        touch(&transaction, &current).await?;
        let event = events::PlanUpdated(json!({ "plan_id": plan_id, "price_added": id }));
        self.record(&transaction, actor, format!("plans.plan:{plan_id}"), &event).await?;
        transaction.commit().await?;
        self.get_plan(plan_id).await
    }

    /// Tắt giá `id`: không bán theo giá này nữa; thuê bao đang dùng giữ tới khi đổi gói.
    pub(crate) async fn deactivate_price(&self, actor: Actor, id: Uuid) -> Result<PlanRow, Error> {
        let transaction = self.control().begin_write().await?;
        let row = price::Entity::find_by_id(id)
            .one(&transaction)
            .await?
            .ok_or_else(|| price_not_found(id))?;
        if row.active {
            let plan_id = row.plan_id;
            let mut active: price::ActiveModel = row.into();
            active.active = Set(false);
            active.update(&transaction).await?;
            if let Some(current) = plan::Entity::find_by_id(plan_id).one(&transaction).await? {
                touch(&transaction, &current).await?;
            }
            let event = events::PlanUpdated(json!({ "plan_id": plan_id, "price_deactivated": id }));
            self.record(&transaction, actor, format!("plans.plan:{plan_id}"), &event).await?;
            transaction.commit().await?;
            return self.get_plan(plan_id).await;
        }
        let plan_id = row.plan_id;
        transaction.commit().await?;
        self.get_plan(plan_id).await
    }

    /// Có khách nào đang dùng gói `id` (gói chính hay gói thêm) không.
    async fn in_use<C: ConnectionTrait>(&self, connection: &C, id: Uuid) -> Result<bool, Error> {
        let main = subscription::Entity::find()
            .filter(subscription::Column::PlanId.eq(id))
            .filter(subscription::Column::Status.ne("deleted"))
            .one(connection)
            .await?;
        let extra =
            addon::Entity::find().filter(addon::Column::PlanId.eq(id)).one(connection).await?;
        Ok(main.is_some() || extra.is_some())
    }

    /// Bỏ cờ mặc định của mọi gói, sau khi khoá dòng cố định của `plans_settings`: hai request đặt
    /// gói mặc định cùng lúc nối đuôi nhau, cuối cùng chỉ một gói mang cờ.
    async fn clear_default<C: ConnectionTrait>(&self, transaction: &C) -> Result<(), Error> {
        rows::lock_one(transaction, settings::Entity::find_by_id(1)).await?;
        plan::Entity::update_many()
            .col_expr(plan::Column::IsDefault, sea_orm::sea_query::Expr::value(false))
            .filter(plan::Column::IsDefault.eq(true))
            .exec(transaction)
            .await?;
        settings::Entity::update_many()
            .col_expr(settings::Column::UpdatedAt, sea_orm::sea_query::Expr::value(now()))
            .filter(settings::Column::Id.eq(1))
            .exec(transaction)
            .await?;
        Ok(())
    }

    /// Kiểm và chuẩn hoá đầu vào của gói; gom mọi lỗi để báo một lần (D-083).
    fn check_plan(&self, mut input: PlanInput) -> Result<PlanInput, Error> {
        let catalog = self.entitlements.catalog();
        let mut problems = Vec::new();
        input.code = input.code.trim().to_owned();
        input.name = input.name.trim().to_owned();
        let code_ok = input.code.len() <= 64
            && input.code.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
            && input
                .code
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
        if !code_ok {
            problems.push(FieldError::new("code", "pattern", "must match `[a-z][a-z0-9_-]{0,63}`"));
        }
        if input.name.is_empty() || input.name.chars().count() > 200 {
            problems.push(FieldError::new("name", "length", "must be 1 to 200 characters long"));
        }
        if input.description.chars().count() > 5000 {
            problems.push(FieldError::new(
                "description",
                "length",
                "must be at most 5000 characters long",
            ));
        }
        if !["base", "addon"].contains(&input.kind.as_str()) {
            problems.push(one_of("kind", &["base", "addon"]));
        }
        if !["draft", "active"].contains(&input.status.as_str()) {
            problems.push(one_of("status", &["draft", "active"]));
        }
        if input.is_default && (input.kind != "base" || input.status != "active") {
            problems.push(FieldError::new(
                "is_default",
                "not_allowed",
                "only an active base plan can be the default",
            ));
        }
        if input.self_signup && input.kind != "base" {
            problems.push(FieldError::new(
                "self_signup",
                "not_allowed",
                "only a base plan can be chosen at sign-up",
            ));
        }
        for (field, value) in [
            ("trial_days", input.trial_days),
            ("grace_days", input.grace_days),
            ("suspend_days", input.suspend_days),
            ("retention_days", input.retention_days),
            ("renewal_lead_days", input.renewal_lead_days),
        ] {
            if !(0..=3650).contains(&value) {
                problems.push(range(field, 0, 3650));
            }
        }
        if !["shared", "dedicated"].contains(&input.placement.as_str()) {
            problems.push(one_of("placement", &["shared", "dedicated"]));
        }
        input.modules.sort();
        input.modules.dedup();
        input.features.sort();
        input.features.dedup();
        for (index, module) in input.modules.iter().enumerate() {
            if !catalog.modules().contains(&module.as_str()) {
                problems.push(FieldError::new(
                    format!("modules[{index}]"),
                    "unknown",
                    "no installed module has this name",
                ));
            }
        }
        for (index, feature) in input.features.iter().enumerate() {
            if catalog.feature(feature).is_none() {
                problems.push(FieldError::new(
                    format!("features[{index}]"),
                    "unknown",
                    "no module declares this feature",
                ));
            }
        }
        for (key, value) in &input.limits {
            if catalog.limit(key).is_none() {
                problems.push(FieldError::new(
                    format!("limits.{key}"),
                    "unknown",
                    "no module declares this limit",
                ));
            } else if value.is_some_and(|value| value < 0) {
                problems.push(range(&format!("limits.{key}"), 0, i64::MAX));
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        // Gói chính bật module nào thì phải bật cả module nó cần (gói thêm đi kèm một gói chính).
        if input.kind == "base" {
            for module in &input.modules {
                for needed in catalog.requires(module) {
                    if !catalog.is_always_enabled(needed)
                        && !input.modules.iter().any(|m| m == needed)
                    {
                        return Err(Error::new(
                            ErrorKind::Validation,
                            "plans.module_dependency_missing",
                            "The module `{module}` needs the module `{requires}` in the same plan.",
                        )
                        .with_param("module", module.clone())
                        .with_param("requires", needed));
                    }
                }
            }
        }
        Ok(input)
    }

    // ---- Thuê bao -------------------------------------------------------------------------

    pub(crate) async fn subscription(
        &self,
        tenant: TenantId,
    ) -> Result<Option<SubscriptionRow>, Error> {
        let read = self.control().read();
        let Some(subscription) = subscription::Entity::find()
            .filter(subscription::Column::TenantId.eq(tenant.as_uuid()))
            .one(read)
            .await?
        else {
            return Ok(None);
        };
        let addons = addon::Entity::find()
            .filter(addon::Column::TenantId.eq(tenant.as_uuid()))
            .order_by_asc(addon::Column::PlanId)
            .all(read)
            .await?;
        let plan = plan::Entity::find_by_id(subscription.plan_id)
            .one(read)
            .await?
            .map(|row| (row.code, row.name));
        Ok(Some(SubscriptionRow { subscription, addons, plan }))
    }

    pub(crate) async fn list_subscriptions(
        &self,
        filter: &SubscriptionFilter,
        page: &PageRequest,
    ) -> Result<Page<SubscriptionRow>, Error> {
        let mut select = subscription::Entity::find();
        if let Some(status) = &filter.status {
            select = select.filter(subscription::Column::Status.eq(status.clone()));
        }
        if let Some(plan_id) = filter.plan_id {
            select = select.filter(subscription::Column::PlanId.eq(plan_id));
        }
        if let Some(tenant_id) = filter.tenant_id {
            select = select.filter(subscription::Column::TenantId.eq(tenant_id));
        }
        let read = self.control().read();
        let page = fetch_page(select, subscription::Column::Id, |row| row.id, page, read).await?;
        let names = self.plan_names(page.data.iter().map(|row| row.plan_id)).await?;
        Ok(page.map(|subscription| {
            let plan = names.get(&subscription.plan_id).cloned();
            SubscriptionRow { subscription, addons: Vec::new(), plan }
        }))
    }

    /// Thuê bao (không kèm gói thêm) của các khách `tenants`, kèm mã và tên gói (hai câu).
    pub(crate) async fn subscriptions_of(
        &self,
        tenants: &[TenantId],
    ) -> Result<Vec<SubscriptionRow>, Error> {
        if tenants.is_empty() {
            return Ok(Vec::new());
        }
        let rows = subscription::Entity::find()
            .filter(subscription::Column::TenantId.is_in(tenants.iter().map(|id| id.as_uuid())))
            .all(self.control().read())
            .await?;
        let names = self.plan_names(rows.iter().map(|row| row.plan_id)).await?;
        Ok(rows
            .into_iter()
            .map(|subscription| {
                let plan = names.get(&subscription.plan_id).cloned();
                SubscriptionRow { subscription, addons: Vec::new(), plan }
            })
            .collect())
    }

    /// Mã và tên của các gói `ids` (một câu).
    pub(crate) async fn plan_names(
        &self,
        ids: impl IntoIterator<Item = Uuid>,
    ) -> Result<HashMap<Uuid, (String, String)>, Error> {
        let ids: std::collections::BTreeSet<Uuid> = ids.into_iter().collect();
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        Ok(plan::Entity::find()
            .filter(plan::Column::Id.is_in(ids))
            .all(self.control().read())
            .await?
            .into_iter()
            .map(|row| (row.id, (row.code, row.name)))
            .collect())
    }

    /// Nhà vận hành gán gói cho khách `tenant`: lần đầu tạo thuê bao (dùng thử khi gói có và khách
    /// chưa dùng thử), sau đó đổi gói / giá ngay. Khách nền tảng không bao giờ có thuê bao.
    pub(crate) async fn assign(
        &self,
        actor: Actor,
        tenant: TenantId,
        input: AssignInput,
    ) -> Result<SubscriptionRow, Error> {
        if tenant == DEFAULT_TENANT {
            return Err(platform_exempt());
        }
        // Khách phải tồn tại (lỗi 404 của kernel khi không có).
        self.databases.tenant(tenant).await?;
        let at = now();
        let transaction = self.control().begin_write().await?;
        let chosen = plan::Entity::find_by_id(input.plan_id)
            .one(&transaction)
            .await?
            .ok_or_else(|| plan_not_found(input.plan_id))?;
        if chosen.kind != "base" {
            return Err(addon_not_allowed(&chosen.code));
        }
        if chosen.status != "active" {
            return Err(plan_not_active(&chosen.code));
        }
        let chosen_price = match input.price_id {
            Some(price_id) => {
                let found = price::Entity::find_by_id(price_id)
                    .one(&transaction)
                    .await?
                    .filter(|row| row.plan_id == chosen.id && row.active && row.kind == "recurring")
                    .ok_or_else(|| price_not_found(price_id))?;
                Some(found)
            }
            None => None,
        };
        if let Some(addons) = &input.addons {
            for (plan_id, quantity) in addons {
                let extra = plan::Entity::find_by_id(*plan_id)
                    .one(&transaction)
                    .await?
                    .ok_or_else(|| plan_not_found(*plan_id))?;
                if extra.kind != "addon" {
                    return Err(addon_not_allowed(&extra.code));
                }
                if extra.status != "active" {
                    return Err(plan_not_active(&extra.code));
                }
                if !(1..=1000).contains(quantity) {
                    return Err(Error::validation(vec![range("addons.quantity", 1, 1000)]));
                }
            }
        }
        let current = rows::lock_one(
            &transaction,
            subscription::Entity::find()
                .filter(subscription::Column::TenantId.eq(tenant.as_uuid())),
        )
        .await?;
        let period_end = period_end(at, chosen_price.as_ref())?;
        let subject = format!("plans.subscription:{tenant}");
        let row = match current {
            None => {
                let trial = input.trial && chosen.trial_days > 0;
                let trial_ends =
                    trial.then(|| at + chrono::Duration::days(i64::from(chosen.trial_days)));
                let row = subscription::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.as_uuid()),
                    plan_id: Set(chosen.id),
                    price_id: Set(chosen_price.as_ref().map(|row| row.id)),
                    status: Set(if trial { "trialing" } else { "active" }.to_owned()),
                    current_period_start: Set(at),
                    current_period_end: Set(trial_ends.unwrap_or(period_end)),
                    trial_ends_at: Set(trial_ends),
                    grace_ends_at: Set(None),
                    suspended_at: Set(None),
                    canceled_at: Set(None),
                    delete_after: Set(None),
                    cancel_at_period_end: Set(false),
                    scheduled_plan_id: Set(None),
                    scheduled_price_id: Set(None),
                    trial_used: Set(trial),
                    billing_email: Set(None),
                    created_at: Set(at),
                    updated_at: Set(at),
                }
                .insert(&transaction)
                .await?;
                let event = events::SubscriptionCreated(json!({
                    "tenant_id": tenant,
                    "subscription": subscription_json(&row),
                }));
                self.record(&transaction, actor, subject.clone(), &event).await?;
                row
            }
            Some(current) if current.status == "deleted" => {
                return Err(invalid_transition(&current.status));
            }
            Some(current) => {
                let before = subscription_json(&current);
                let from_status = current.status.clone();
                let mut row: subscription::ActiveModel = current.into();
                row.plan_id = Set(chosen.id);
                row.price_id = Set(chosen_price.as_ref().map(|row| row.id));
                row.scheduled_plan_id = Set(None);
                row.scheduled_price_id = Set(None);
                // Đã huỷ mà gán lại: đăng ký lại, kỳ mới bắt đầu từ bây giờ.
                if from_status == "canceled" {
                    row.status = Set("active".to_owned());
                    row.current_period_start = Set(at);
                    row.current_period_end = Set(period_end);
                    row.canceled_at = Set(None);
                    row.delete_after = Set(None);
                    row.cancel_at_period_end = Set(false);
                }
                row.updated_at = Set(at);
                let row = row.update(&transaction).await?;
                let event = events::SubscriptionPlanChanged(json!({
                    "tenant_id": tenant,
                    "before": before,
                    "after": subscription_json(&row),
                }));
                self.record(&transaction, actor, subject.clone(), &event).await?;
                if from_status != row.status {
                    let event = events::SubscriptionStatusChanged(json!({
                        "tenant_id": tenant, "from": from_status, "to": row.status,
                    }));
                    self.record(&transaction, actor, subject.clone(), &event).await?;
                }
                row
            }
        };
        if let Some(addons) = input.addons {
            addon::Entity::delete_many()
                .filter(addon::Column::TenantId.eq(tenant.as_uuid()))
                .exec(&transaction)
                .await?;
            for (plan_id, quantity) in addons {
                addon::ActiveModel {
                    tenant_id: Set(tenant.as_uuid()),
                    plan_id: Set(plan_id),
                    quantity: Set(quantity),
                    created_at: Set(at),
                }
                .insert(&transaction)
                .await?;
            }
        }
        let event = events::EntitlementsChanged(json!({ "tenant_id": tenant }));
        self.record(&transaction, actor, subject, &event).await?;
        transaction.commit().await?;
        self.cache.forget(tenant);
        tracing::info!(tenant_id = %tenant, plan_id = %row.plan_id, status = %row.status, "subscription assigned");
        self.subscription(tenant).await?.ok_or_else(subscription_not_found)
    }

    // ---- Ghi đè ---------------------------------------------------------------------------

    pub(crate) async fn overrides(&self, tenant: TenantId) -> Result<Vec<grant::Model>, Error> {
        Ok(grant::Entity::find()
            .filter(grant::Column::TenantId.eq(tenant.as_uuid()))
            .order_by_asc(grant::Column::Id)
            .all(self.control().read())
            .await?)
    }

    /// Ghi đè của mọi khách (nhà vận hành), theo trang; `active`: chỉ cái còn hiệu lực lúc `at`
    /// (`true`) hay chỉ cái đã hết hạn (`false`).
    pub(crate) async fn all_overrides(
        &self,
        filter: &OverrideFilter,
        at: DateTime<Utc>,
        page: &PageRequest,
    ) -> Result<Page<grant::Model>, Error> {
        let mut select = grant::Entity::find();
        if let Some(tenant_id) = filter.tenant_id {
            select = select.filter(grant::Column::TenantId.eq(tenant_id));
        }
        if let Some(kind) = &filter.kind {
            select = select.filter(grant::Column::Kind.eq(kind.clone()));
        }
        if let Some(key) = &filter.key {
            select = select.filter(grant::Column::Key.eq(key.clone()));
        }
        match filter.active {
            Some(true) => {
                select = select.filter(
                    sea_orm::Condition::any()
                        .add(grant::Column::ExpiresAt.is_null())
                        .add(grant::Column::ExpiresAt.gt(at)),
                );
            }
            Some(false) => select = select.filter(grant::Column::ExpiresAt.lte(at)),
            None => {}
        }
        fetch_page(select, grant::Column::Id, |row| row.id, page, self.control().read()).await
    }

    /// Đặt (hay thay) ghi đè `kind` / `key` cho khách `tenant`.
    pub(crate) async fn set_override(
        &self,
        principal: &Principal,
        tenant: TenantId,
        input: OverrideInput,
    ) -> Result<grant::Model, Error> {
        if tenant == DEFAULT_TENANT {
            return Err(platform_exempt());
        }
        self.databases.tenant(tenant).await?;
        let catalog = self.entitlements.catalog();
        let mut problems = Vec::new();
        let kind = OverrideKind::parse(&input.kind);
        match kind {
            None => problems.push(one_of("kind", &["module", "feature", "limit"])),
            Some(OverrideKind::Module) if !catalog.modules().contains(&input.key.as_str()) => {
                problems.push(FieldError::new(
                    "key",
                    "unknown",
                    "no installed module has this name",
                ));
            }
            Some(OverrideKind::Feature) if catalog.feature(&input.key).is_none() => {
                problems.push(FieldError::new("key", "unknown", "no module declares this feature"));
            }
            Some(OverrideKind::Limit) if catalog.limit(&input.key).is_none() => {
                problems.push(FieldError::new("key", "unknown", "no module declares this limit"));
            }
            _ => {}
        }
        match kind {
            Some(OverrideKind::Module | OverrideKind::Feature) if input.enabled.is_none() => {
                problems.push(FieldError::new(
                    "enabled",
                    "required",
                    "a module or feature override needs `enabled`",
                ));
            }
            Some(OverrideKind::Limit) if input.max_value.is_some_and(|value| value < 0) => {
                problems.push(range("max_value", 0, i64::MAX));
            }
            _ => {}
        }
        let reason = input.reason.trim().to_owned();
        if reason.is_empty() || reason.chars().count() > 500 {
            problems.push(FieldError::new("reason", "length", "must be 1 to 500 characters long"));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let at = now();
        let transaction = self.control().begin_write().await?;
        grant::Entity::delete_many()
            .filter(grant::Column::TenantId.eq(tenant.as_uuid()))
            .filter(grant::Column::Kind.eq(input.kind.clone()))
            .filter(grant::Column::Key.eq(input.key.clone()))
            .exec(&transaction)
            .await?;
        let is_limit = kind == Some(OverrideKind::Limit);
        let row = grant::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.as_uuid()),
            kind: Set(input.kind),
            key: Set(input.key),
            enabled: Set(if is_limit { None } else { input.enabled }),
            max_value: Set(if is_limit { input.max_value } else { None }),
            reason: Set(reason),
            expires_at: Set(input.expires_at.map(|at| at.trunc_subsecs(6))),
            created_by: Set(Some(principal.user_id())),
            created_at: Set(at),
        }
        .insert(&transaction)
        .await?;
        let actor = Actor::from(principal);
        let subject = format!("plans.subscription:{tenant}");
        let event =
            events::OverrideSet(json!({ "tenant_id": tenant, "override": override_json(&row) }));
        self.record(&transaction, actor, subject.clone(), &event).await?;
        let event = events::EntitlementsChanged(json!({ "tenant_id": tenant }));
        self.record(&transaction, actor, subject, &event).await?;
        transaction.commit().await?;
        self.cache.forget(tenant);
        Ok(row)
    }

    pub(crate) async fn remove_override(
        &self,
        actor: Actor,
        tenant: TenantId,
        id: Uuid,
    ) -> Result<(), Error> {
        let transaction = self.control().begin_write().await?;
        let row = grant::Entity::find_by_id(id)
            .filter(grant::Column::TenantId.eq(tenant.as_uuid()))
            .one(&transaction)
            .await?
            .ok_or_else(|| override_not_found(id))?;
        grant::Entity::delete_by_id(id).exec(&transaction).await?;
        let subject = format!("plans.subscription:{tenant}");
        let event = events::OverrideRemoved(
            json!({ "tenant_id": tenant, "override": override_json(&row) }),
        );
        self.record(&transaction, actor, subject.clone(), &event).await?;
        let event = events::EntitlementsChanged(json!({ "tenant_id": tenant }));
        self.record(&transaction, actor, subject, &event).await?;
        transaction.commit().await?;
        self.cache.forget(tenant);
        Ok(())
    }

    // ---- Quyền dùng -----------------------------------------------------------------------

    /// Quyền dùng của khách `tenant`, từ bộ nhớ đệm hay đọc lại (một khách: tối đa sáu câu).
    pub(crate) async fn snapshot(
        &self,
        tenant: TenantId,
    ) -> Result<Arc<TenantEntitlements>, Error> {
        if tenant == DEFAULT_TENANT {
            return Ok(Arc::new(TenantEntitlements::unrestricted()));
        }
        if let Some(found) = self.cache.get(tenant) {
            return Ok(found);
        }
        let fresh = Arc::new(self.compute(tenant, now()).await?);
        self.cache.put(tenant, fresh.clone());
        Ok(fresh)
    }

    /// Đọc và gộp quyền dùng của khách `tenant` tại `now`, không qua bộ nhớ đệm.
    pub(crate) async fn compute(
        &self,
        tenant: TenantId,
        now: DateTime<Utc>,
    ) -> Result<TenantEntitlements, Error> {
        let read = self.control().read();
        let current = subscription::Entity::find()
            .filter(subscription::Column::TenantId.eq(tenant.as_uuid()))
            .one(read)
            .await?;
        let (base_id, access, addons) = match &current {
            Some(row) => {
                let access = match row.status.as_str() {
                    "suspended" | "canceled" | "deleted" => TenantAccess::ReadOnly,
                    _ => TenantAccess::Full,
                };
                let addons = addon::Entity::find()
                    .filter(addon::Column::TenantId.eq(tenant.as_uuid()))
                    .all(read)
                    .await?;
                (Some(row.plan_id), access, addons)
            }
            None => {
                let default = plan::Entity::find()
                    .filter(plan::Column::IsDefault.eq(true))
                    .filter(plan::Column::Status.eq("active"))
                    .one(read)
                    .await?;
                (default.map(|row| row.id), TenantAccess::Full, Vec::new())
            }
        };
        let Some(base_id) = base_id else {
            return Ok(TenantEntitlements::unrestricted());
        };
        let mut ids: Vec<Uuid> = vec![base_id];
        ids.extend(addons.iter().map(|row| row.plan_id));
        let mut grants = self.grants(&ids).await?;
        let base = grants.remove(&base_id).unwrap_or_default();
        let extras: Vec<(Grants, i64)> = addons
            .iter()
            .map(|row| {
                (grants.get(&row.plan_id).cloned().unwrap_or_default(), i64::from(row.quantity))
            })
            .collect();
        let overrides: Vec<Override> = grant::Entity::find()
            .filter(grant::Column::TenantId.eq(tenant.as_uuid()))
            .all(read)
            .await?
            .into_iter()
            .filter_map(|row| {
                Some(Override {
                    kind: OverrideKind::parse(&row.kind)?,
                    key: row.key,
                    enabled: row.enabled.unwrap_or(false),
                    max_value: row.max_value,
                    expires_at: row.expires_at,
                })
            })
            .collect();
        Ok(merge::merge(Some(&base), &extras, &overrides, access, now))
    }
}

/// Mốc hết kỳ đầu từ `start` theo giá `price` (lịch dương; ngày 31 → cuối tháng sau); không có giá
/// thì một tháng.
pub(crate) fn period_end(
    start: DateTime<Utc>,
    price: Option<&price::Model>,
) -> Result<DateTime<Utc>, Error> {
    let months = match price {
        Some(row) if row.interval == "year" => row.interval_count.saturating_mul(12),
        Some(row) => row.interval_count,
        None => 1,
    };
    let months = u32::try_from(months.max(1)).map_err(Error::internal)?;
    start
        .checked_add_months(Months::new(months))
        .ok_or_else(|| Error::internal("subscription period end is out of range"))
}

fn active_plan(input: &PlanInput) -> plan::ActiveModel {
    plan::ActiveModel {
        code: Set(input.code.clone()),
        name: Set(input.name.clone()),
        description: Set(input.description.clone()),
        kind: Set(input.kind.clone()),
        status: Set(input.status.clone()),
        is_default: Set(input.is_default),
        trial_days: Set(input.trial_days),
        grace_days: Set(input.grace_days),
        suspend_days: Set(input.suspend_days),
        retention_days: Set(input.retention_days),
        renewal_lead_days: Set(input.renewal_lead_days),
        placement: Set(input.placement.clone()),
        requires_kyc: Set(input.requires_kyc),
        sample_data: Set(input.sample_data),
        self_signup: Set(input.self_signup),
        ..Default::default()
    }
}

async fn write_grants<C: ConnectionTrait>(
    connection: &C,
    id: Uuid,
    input: &PlanInput,
) -> Result<(), Error> {
    for module in &input.modules {
        plan_module::ActiveModel { plan_id: Set(id), module: Set(module.clone()) }
            .insert(connection)
            .await?;
    }
    for feature in &input.features {
        plan_feature::ActiveModel { plan_id: Set(id), feature: Set(feature.clone()) }
            .insert(connection)
            .await?;
    }
    for (key, value) in &input.limits {
        plan_limit::ActiveModel {
            plan_id: Set(id),
            limit_key: Set(key.clone()),
            max_value: Set(*value),
        }
        .insert(connection)
        .await?;
    }
    Ok(())
}

/// Đổi `updated_at` của gói (giá đổi là gói đổi: `ETag` mới).
async fn touch<C: ConnectionTrait>(connection: &C, current: &plan::Model) -> Result<(), Error> {
    let mut row: plan::ActiveModel = current.clone().into();
    row.updated_at = Set(now());
    row.update(connection).await?;
    Ok(())
}

fn plan_json(id: Uuid, input: &PlanInput) -> Value {
    json!({
        "id": id,
        "code": input.code,
        "name": input.name,
        "kind": input.kind,
        "status": input.status,
        "is_default": input.is_default,
        "modules": input.modules,
        "features": input.features,
        "limits": input.limits,
    })
}

pub(crate) fn subscription_json(row: &subscription::Model) -> Value {
    json!({
        "id": row.id,
        "plan_id": row.plan_id,
        "price_id": row.price_id,
        "status": row.status,
        "current_period_end": row.current_period_end,
    })
}

fn override_json(row: &grant::Model) -> Value {
    json!({
        "id": row.id,
        "kind": row.kind,
        "key": row.key,
        "enabled": row.enabled,
        "max_value": row.max_value,
        "reason": row.reason,
        "expires_at": row.expires_at,
    })
}

fn code_taken(error: sea_orm::DbErr, code: &str) -> Error {
    if matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) {
        Error::conflict("plans.plan_code_taken", "The plan code `{code}` is already used.")
            .with_param("code", code.to_owned())
    } else {
        error.into()
    }
}

fn one_of(field: &str, values: &[&str]) -> FieldError {
    let listed: Vec<String> = values.iter().map(|value| format!("`{value}`")).collect();
    FieldError::new(field, "one_of", "must be one of {values}")
        .with_param("values", listed.join(", "))
}

fn range(field: &str, min: i64, max: i64) -> FieldError {
    FieldError::new(field, "range", "must be between {min} and {max}")
        .with_param("min", min)
        .with_param("max", max)
}

pub(crate) fn plan_not_found(id: Uuid) -> Error {
    Error::not_found("plans.plan_not_found", "There is no plan `{id}`.").with_param("id", id)
}

pub(crate) fn plan_not_active(code: &str) -> Error {
    Error::conflict("plans.plan_not_active", "The plan `{code}` is no longer sold.")
        .with_param("code", code.to_owned())
}

fn plan_in_use() -> Error {
    Error::conflict(
        "plans.plan_in_use",
        "Tenants use this plan: it can only be retired, not deleted or turned into another kind.",
    )
}

pub(crate) fn price_not_found(id: Uuid) -> Error {
    Error::not_found("plans.price_not_found", "There is no price `{id}` of this plan.")
        .with_param("id", id)
}

pub(crate) fn subscription_not_found() -> Error {
    Error::not_found("plans.subscription_not_found", "This tenant has no subscription.")
}

pub(crate) fn invalid_transition(status: &str) -> Error {
    Error::conflict(
        "plans.invalid_transition",
        "The subscription is `{status}`, so this cannot be done.",
    )
    .with_param("status", status.to_owned())
}

fn addon_not_allowed(code: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "plans.addon_not_allowed",
        "`{code}` is not the right kind of plan here: a subscription has one base plan and any number of add-ons.",
    )
    .with_param("code", code.to_owned())
}

fn override_not_found(id: Uuid) -> Error {
    Error::not_found("plans.override_not_found", "This tenant has no override `{id}`.")
        .with_param("id", id)
}

fn platform_exempt() -> Error {
    Error::new(
        ErrorKind::Validation,
        "plans.platform_exempt",
        "The platform tenant is never limited by a plan: it has no subscription or overrides.",
    )
}
