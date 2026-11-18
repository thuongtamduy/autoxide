//! `CompiledPromotions` (D-222, "Nhẹ và nhanh"): chương trình đang chạy (và sắp chạy trong 24 giờ)
//! của một khách, đã biên dịch thành [`Program`] cùng chỉ mục biến thể / sản phẩm / danh mục →
//! chương trình. `evaluate` tra chỉ mục cho các dòng của giỏ, không query database ngoài lần nạp.
//!
//! Khoá theo `promotions_revisions` (tăng trong cùng transaction với mọi lần đổi chương trình):
//! máy chủ này xoá ngay khi tự đổi; máy chủ khác kiểm số phiên bản sau `COMPILED_TTL_SECONDS`.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext, TenantId};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::entity::{promotion, revision};
use crate::model::*;
use crate::programs::{PromotionView, load_children};

/// Bộ chương trình đã biên dịch của một khách.
#[derive(Debug, Default)]
pub(crate) struct Compiled {
    pub revision: i64,
    pub programs: Vec<Arc<Program>>,
    /// Chương trình không có mục tiêu (áp mọi dòng).
    untargeted: Vec<usize>,
    by_variant: HashMap<Uuid, Vec<usize>>,
    by_product: HashMap<Uuid, Vec<usize>>,
    by_category: HashMap<Uuid, Vec<usize>>,
}

impl Compiled {
    fn build(revision: i64, programs: Vec<Program>) -> Self {
        let mut compiled = Compiled { revision, ..Compiled::default() };
        for (index, program) in programs.into_iter().enumerate() {
            let targets = [&program.conditions_targets, &program.reward_targets];
            let flash: Vec<Uuid> = program.flash_items.iter().map(|item| item.variant_id).collect();
            if targets.iter().all(|set| set.is_empty()) && flash.is_empty() {
                compiled.untargeted.push(index);
            }
            for set in targets {
                for id in &set.variants {
                    compiled.by_variant.entry(*id).or_default().push(index);
                }
                for id in &set.products {
                    compiled.by_product.entry(*id).or_default().push(index);
                }
                for id in &set.categories {
                    compiled.by_category.entry(*id).or_default().push(index);
                }
            }
            for id in flash {
                compiled.by_variant.entry(id).or_default().push(index);
            }
            compiled.programs.push(Arc::new(program));
        }
        compiled
    }

    /// Chương trình có thể chạm các dòng có (biến thể, sản phẩm, danh mục) này, cùng mọi chương
    /// trình không mục tiêu — không trùng, theo thứ tự nạp.
    pub(crate) fn candidates<'a>(
        &'a self,
        lines: impl Iterator<Item = (Uuid, Option<Uuid>, &'a [Uuid])>,
    ) -> Vec<&'a Program> {
        let mut found: BTreeSet<usize> = self.untargeted.iter().copied().collect();
        for (variant, product, categories) in lines {
            found.extend(self.by_variant.get(&variant).into_iter().flatten());
            if let Some(product) = product {
                found.extend(self.by_product.get(&product).into_iter().flatten());
            }
            for category in categories {
                found.extend(self.by_category.get(category).into_iter().flatten());
            }
        }
        found.into_iter().map(|index| self.programs[index].as_ref()).collect()
    }

    pub(crate) fn get(&self, id: Uuid) -> Option<&Program> {
        self.programs.iter().find(|program| program.id == id).map(AsRef::as_ref)
    }
}

/// Bộ nhớ đệm theo khách.
#[derive(Default)]
pub(crate) struct CompiledCache {
    slots: RwLock<HashMap<TenantId, (Instant, Arc<Compiled>)>>,
}

impl CompiledCache {
    /// Xoá bộ của khách (máy chủ này vừa đổi chương trình).
    pub(crate) fn invalidate(&self, tenant: TenantId) {
        if let Ok(mut slots) = self.slots.write() {
            slots.remove(&tenant);
        }
    }

    fn cached(&self, tenant: TenantId) -> Option<(Instant, Arc<Compiled>)> {
        self.slots.read().ok()?.get(&tenant).cloned()
    }

    fn store(&self, tenant: TenantId, compiled: Arc<Compiled>) {
        if let Ok(mut slots) = self.slots.write() {
            slots.insert(tenant, (Instant::now(), compiled));
        }
    }

    /// Bộ chương trình của khách: dùng bản trong bộ nhớ khi còn hạn; hết hạn thì so số phiên bản
    /// (một câu đọc), khác thì nạp lại.
    pub(crate) async fn get(
        &self,
        tenant: &TenantContext,
        ttl: Duration,
    ) -> Result<Arc<Compiled>, Error> {
        let cached = self.cached(tenant.id());
        if let Some((loaded, compiled)) = &cached
            && loaded.elapsed() < ttl
        {
            return Ok(compiled.clone());
        }
        let read = tenant.database().read();
        let revision = revision::Entity::find()
            .filter(revision::Column::TenantId.eq(tenant.id().as_uuid()))
            .one(read)
            .await?
            .map_or(0, |row| row.revision);
        if let Some((_, compiled)) = cached
            && compiled.revision == revision
        {
            self.store(tenant.id(), compiled.clone());
            return Ok(compiled);
        }
        let horizon = crate::service::now() + chrono::Duration::hours(24);
        let rows = promotion::Entity::find()
            .filter(promotion::Column::Status.is_in(["active", "scheduled"]))
            .filter(promotion::Column::EndsAt.gt(crate::service::now()))
            .filter(promotion::Column::StartsAt.lt(horizon))
            .scoped(tenant)
            .all(read)
            .await?;
        let views = load_children(read, tenant, rows).await?;
        let compiled = Arc::new(Compiled::build(revision, views.iter().map(program_of).collect()));
        self.store(tenant.id(), compiled.clone());
        Ok(compiled)
    }
}

/// Biên dịch một chương trình đã đọc: chữ thành enum, JSON điều kiện thành struct.
pub(crate) fn program_of(view: &PromotionView) -> Program {
    let row = &view.row;
    let mut conditions_targets = TargetSet::default();
    let mut reward_targets = TargetSet::default();
    for found in &view.targets {
        let Some(kind) = TargetType::parse(&found.target_type) else { continue };
        match Role::parse(&found.role) {
            Some(Role::Reward) => reward_targets.insert(kind, found.target_id),
            _ => conditions_targets.insert(kind, found.target_id),
        }
    }
    Program {
        id: row.id,
        org_id: row.org_id,
        kind: Kind::parse(&row.kind).unwrap_or(Kind::PercentOff),
        scope: Scope::parse(&row.scope).unwrap_or(Scope::Items),
        trigger: Trigger::parse(&row.trigger).unwrap_or(Trigger::Automatic),
        status: Status::parse(&row.status).unwrap_or(Status::Draft),
        currency: row.currency.clone(),
        value_bp: row.value_bp,
        value_minor: row.value_minor,
        min_order_minor: row.min_order_minor,
        min_quantity: row.min_quantity,
        max_discount_minor: row.max_discount_minor,
        buy_quantity: row.buy_quantity,
        get_quantity: row.get_quantity,
        reward_selection: row.reward_selection.as_deref().and_then(RewardSelection::parse),
        reward_value_bp: row.reward_value_bp,
        max_applications: row.max_applications,
        bundle_quantity: row.bundle_quantity,
        bundle_price_minor: row.bundle_price_minor,
        max_add_on_quantity: row.max_add_on_quantity,
        gift_quantity: row.gift_quantity,
        first_order_only: row.first_order_only,
        starts_at: row.starts_at,
        ends_at: row.ends_at,
        priority: row.priority,
        exclusivity: Exclusivity::parse(&row.exclusivity).unwrap_or(Exclusivity::None),
        combines_with: row.combines_with.split(',').filter_map(parse_class).collect(),
        stop_after: row.stop_after,
        usage_limit: row.usage_limit,
        usage_count: row.usage_count,
        per_customer_limit: row.per_customer_limit,
        per_customer_period: Period::parse(&row.per_customer_period).unwrap_or(Period::Lifetime),
        daily_usage_limit: row.daily_usage_limit,
        budget_minor: row.budget_minor,
        budget_used_minor: row.budget_used_minor,
        return_usage_on_cancel: row.return_usage_on_cancel,
        gift_return_policy: GiftPolicy::parse(&row.gift_return_policy)
            .unwrap_or(GiftPolicy::ReturnGift),
        program_kind: ProgramKind::parse(&row.program_kind).unwrap_or(ProgramKind::Regular),
        public: row.public,
        texts: view
            .texts
            .iter()
            .map(|text| (text.locale.clone(), text.title.clone(), text.terms.clone()))
            .collect(),
        conditions_targets,
        reward_targets,
        customer_groups: view.groups.iter().map(|group| group.group_id).collect(),
        channels: view.channels.iter().map(|found| found.channel.clone()).collect(),
        conditions: view
            .conditions
            .iter()
            .filter_map(|found| {
                Some(Condition {
                    kind: ConditionKind::parse(&found.kind)?,
                    operator: Operator::parse(&found.operator)?,
                    values: serde_json::from_str(&found.values).unwrap_or_default(),
                })
            })
            .collect(),
        windows: view
            .schedules
            .iter()
            .map(|found| Window {
                weekdays: found.weekdays,
                start_minute: found.start_minute,
                end_minute: found.end_minute,
            })
            .collect(),
        tiers: view
            .tiers
            .iter()
            .filter_map(|found| {
                Some(Tier {
                    threshold_kind: ThresholdKind::parse(&found.threshold_kind)?,
                    threshold: found.threshold,
                    value_bp: found.value_bp,
                    value_minor: found.value_minor,
                })
            })
            .collect(),
        flash_items: view
            .flash
            .iter()
            .map(|item| FlashItem {
                variant_id: item.variant_id,
                sale_price_minor: item.sale_price_minor,
                value_bp: item.value_bp,
                quantity_limit: item.quantity_limit,
                quantity_used: item.quantity_used,
                per_customer_quantity_limit: item.per_customer_quantity_limit,
            })
            .collect(),
        gift_options: view
            .gifts
            .iter()
            .map(|gift| GiftOption {
                variant_id: gift.variant_id,
                stock_limit: gift.stock_limit,
                quantity_used: gift.quantity_used,
            })
            .collect(),
        funding: view
            .funding
            .iter()
            .filter_map(|share| {
                Some(Funding {
                    payer: Payer::parse(&share.payer_kind)?,
                    payer_org_id: share.payer_org_id,
                    share_bp: share.share_bp,
                })
            })
            .collect(),
    }
}
