//! Thuê bao của khách: đọc, liệt kê, gán gói (đổi gói, add-on, kỳ thanh toán).

use super::*;

impl PlansService {
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

pub(crate) fn subscription_json(row: &subscription::Model) -> Value {
    json!({
        "id": row.id,
        "plan_id": row.plan_id,
        "price_id": row.price_id,
        "status": row.status,
        "current_period_end": row.current_period_end,
    })
}
