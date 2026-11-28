//! Listing tenants: filters, `expand`, and the keyset page.
//! Moved out of `admin.rs` unchanged (D-20261118-file-length).

use super::*;

impl TenantAdmin {
    /// One page of tenants, newest first (unless `order=asc`). Two queries for the whole page (the small
    /// placement table, then the filtered page of tenants), not one call per row.
    pub async fn list(
        &self,
        filter: &TenantFilter,
        page: &PageRequest,
    ) -> Result<Page<Tenant>, Error> {
        let mut problems = Vec::new();
        let mut select = tenant::Entity::find();
        if let Some(status) = &filter.status {
            if TenantStatus::parse(status).is_none() {
                problems.push(FieldError::new(
                    "status",
                    "invalid",
                    "must be one of provisioning, active, suspended, maintenance, moving, deleted",
                ));
            }
            select = select.filter(tenant::Column::Status.eq(status.as_str()));
        }
        if let Some(parent) = filter.parent_id {
            select = select.filter(tenant::Column::ParentId.eq(parent));
        }
        if let Some(read_only) = filter.read_only {
            select = select.filter(tenant::Column::ReadOnly.eq(read_only));
        }
        if filter.placement_kind.as_deref().is_some_and(|kind| PlacementKind::parse(kind).is_none())
        {
            problems.push(FieldError::new(
                "placement_kind",
                "invalid",
                "must be `shared` or `dedicated`",
            ));
        }
        if filter.schema_state.as_deref().is_some_and(|state| SchemaState::parse(state).is_none()) {
            problems.push(FieldError::new(
                "schema_state",
                "invalid",
                "must be one of ready, behind, migrating",
            ));
        }
        if filter
            .subscription_status
            .as_deref()
            .is_some_and(|state| !SUBSCRIPTION_STATES.contains(&state))
        {
            problems.push(FieldError::new(
                "subscription_status",
                "invalid",
                "must be one of trialing, active, past_due, suspended, canceled, deleted",
            ));
        }
        let mut expand = Vec::new();
        for item in filter.expand.as_deref().unwrap_or_default().split(',').map(str::trim) {
            if item.is_empty() || expand.contains(&item) {
                continue;
            }
            if EXPANSIONS.contains(&item) {
                expand.push(item);
            } else {
                problems.push(FieldError::new(
                    "expand",
                    "invalid",
                    "can only list placement, subscription and usage",
                ));
                break;
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let placements = self.placement_names().await?;
        if filter.placement_kind.is_some() || filter.schema_state.is_some() {
            let ids: Vec<Uuid> = placements
                .iter()
                .filter(|placement| {
                    filter.placement_kind.as_deref().is_none_or(|kind| placement.kind == kind)
                        && filter
                            .schema_state
                            .as_deref()
                            .is_none_or(|state| placement.schema_state == state)
                })
                .map(|placement| placement.id)
                .collect();
            select = select.filter(tenant::Column::PlacementId.is_in(ids));
        }
        // Thuê bao và hạn mức là của module `plans` (bước 35), hỏi qua tay cầm quyền dùng: chưa cài
        // `plans` thì không khách nào có thuê bao hay gần hạn mức — trang rỗng.
        if let Some(status) = &filter.subscription_status {
            let ids = self.entitlements.tenants_with_status(status).await?;
            select =
                select.filter(tenant::Column::Id.is_in(ids.into_iter().map(|id| id.as_uuid())));
        }
        if filter.near_limit == Some(true) {
            let ids = self.entitlements.tenants_near_limit().await?;
            select =
                select.filter(tenant::Column::Id.is_in(ids.into_iter().map(|id| id.as_uuid())));
        }
        let rows =
            fetch_page(select, tenant::Column::Id, |row| row.id, page, self.control().read())
                .await?;
        // Placement của trang, mô tả một lần mỗi cái (đọc nơi lưu riêng cần mở bản mã hoá).
        let mut described: HashMap<Uuid, Placement> = HashMap::new();
        if expand.contains(&"placement") {
            for row in &rows.data {
                if described.contains_key(&row.placement_id) {
                    continue;
                }
                if let Some(placement) =
                    placements.iter().find(|placement| placement.id == row.placement_id)
                {
                    let placement = describe_placement(&self.registry, placement.clone()).await;
                    described.insert(row.placement_id, placement);
                }
            }
        }
        // Thuê bao và lượng dùng: hỏi module `plans` (qua tay cầm quyền dùng) cho cả trang một lần.
        let ids: Vec<TenantId> = rows.data.iter().map(|row| TenantId::from_uuid(row.id)).collect();
        let mut plans = self
            .entitlements
            .describe_tenants(&ids, expand.contains(&"subscription"), expand.contains(&"usage"))
            .await?;
        Ok(rows.map(|row| {
            let placement = described.get(&row.placement_id).cloned();
            let view = plans.remove(&TenantId::from_uuid(row.id)).unwrap_or_default();
            let mut tenant = describe(row, &placements);
            if !expand.is_empty() {
                tenant.expanded = Some(TenantExpanded {
                    placement,
                    subscription: view.subscription,
                    usage: view.usage,
                });
            }
            tenant
        }))
    }
}
