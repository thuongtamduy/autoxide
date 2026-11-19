//! Ngoại lệ quyền dùng theo khách và tính quyền dùng gộp (gói + add-on + ngoại lệ).

use super::*;

impl PlansService {
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

    /// Quyền dùng của khách `tenant`, từ bộ nhớ đệm hay đọc lại (một khách: tối đa sáu câu).
    pub(crate) async fn snapshot(
        &self,
        tenant: TenantId,
    ) -> Result<Arc<TenantEntitlements>, Error> {
        if tenant == DEFAULT_TENANT {
            return Ok(Arc::new(TenantEntitlements::unrestricted()));
        }
        // Sandbox (D-232): không có thuê bao riêng, dùng quyền dùng của khách gốc (đếm riêng).
        let tenant = self.entitled_tenant(tenant).await?;
        if let Some(found) = self.cache.get(tenant) {
            return Ok(found);
        }
        let fresh = Arc::new(self.compute(tenant, now()).await?);
        self.cache.put(tenant, fresh.clone());
        Ok(fresh)
    }

    /// Khách mang quyền dùng của `tenant`: khách gốc khi `tenant` là sandbox, còn lại chính nó.
    async fn entitled_tenant(&self, tenant: TenantId) -> Result<TenantId, Error> {
        if self.databases.mode() == autoxide_kernel::tenancy::TenancyMode::Single {
            return Ok(tenant);
        }
        match self.databases.tenant(tenant).await {
            Ok(context) => Ok(context.sandbox_of().unwrap_or(tenant)),
            Err(error) if error.code() == "tenant_not_found" => Ok(tenant),
            Err(error) => Err(error),
        }
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

pub(super) fn override_json(row: &grant::Model) -> Value {
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
