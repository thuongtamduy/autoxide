//! Đọc / ghi bảng của chương trình. Chỉ dùng trong `programs` và bộ nạp `compiled`.

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::input::{
    ConditionInput, FundingInput, GiftOptionInput, PromotionInput, TierInput, WindowInput,
    weekday_mask,
};
use super::{FlashItemInput, PromotionView, TargetInput, locked_after_start};
use crate::entity::{
    channel, condition, customer_group, flash_item, funding, gift_option, promotion, schedule,
    target, tier, translation,
};
use crate::model::{
    ClassName, ConditionKind, Exclusivity, GiftPolicy, Kind, Operator, Payer, Period, ProgramKind,
    RewardSelection, Role, Scope, ThresholdKind, Trigger, parse_class,
};
use crate::service::{micros, now};

/// Đọc một chương trình kèm phần con.
pub(crate) async fn load<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<Option<PromotionView>, Error> {
    let Some(row) = promotion::Entity::find_by_id(id).scoped(tenant).one(connection).await? else {
        return Ok(None);
    };
    let mut views = load_children(connection, tenant, vec![row]).await?;
    Ok(views.pop())
}

/// Đọc phần con của nhiều chương trình một lượt (vài câu query, không phụ thuộc số chương trình).
pub(crate) async fn load_children<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    rows: Vec<promotion::Model>,
) -> Result<Vec<PromotionView>, Error> {
    let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
    macro_rules! children {
        ($module:ident) => {
            $module::Entity::find()
                .filter($module::Column::PromotionId.is_in(ids.clone()))
                .scoped(tenant)
                .order_by_asc($module::Column::Id)
                .all(connection)
                .await?
        };
    }
    let texts = children!(translation);
    let targets = children!(target);
    let groups = children!(customer_group);
    let channels = children!(channel);
    let conditions = children!(condition);
    let schedules = children!(schedule);
    let tiers = children!(tier);
    let gifts = children!(gift_option);
    let flash = children!(flash_item);
    let funding = children!(funding);
    macro_rules! of {
        ($list:ident, $id:expr) => {
            $list.iter().filter(|item| item.promotion_id == $id).cloned().collect()
        };
    }
    Ok(rows
        .into_iter()
        .map(|row| {
            let id = row.id;
            PromotionView {
                texts: of!(texts, id),
                targets: of!(targets, id),
                groups: of!(groups, id),
                channels: of!(channels, id),
                conditions: of!(conditions, id),
                schedules: of!(schedules, id),
                tiers: of!(tiers, id),
                gifts: of!(gifts, id),
                flash: of!(flash, id),
                funding: of!(funding, id),
                row,
            }
        })
        .collect())
}

/// Dạng JSON của dòng chương trình (cho ảnh chụp).
pub(crate) fn row_json(row: &promotion::Model) -> Value {
    json!({
        "id": row.id, "org_id": row.org_id, "kind": row.kind, "scope": row.scope,
        "trigger": row.trigger, "status": row.status, "currency": row.currency,
        "starts_at": row.starts_at, "ends_at": row.ends_at, "usage_count": row.usage_count,
        "budget_used_minor": row.budget_used_minor, "issued_count": row.issued_count,
        "version": row.version, "activated_at": row.activated_at,
    })
}

fn money(minor: Option<i64>, currency: &str) -> Option<Money> {
    minor.and_then(|minor| Money::from_columns(minor, currency).ok())
}

/// Đầu vào tương ứng với chương trình đã lưu (để sửa bằng cách gộp, và để lưu vết).
pub(crate) fn input_of(view: &PromotionView) -> PromotionInput {
    let row = &view.row;
    let currency = row.currency.as_str();
    let text = view.texts.first();
    PromotionInput {
        org_id: Some(row.org_id),
        kind: Kind::parse(&row.kind).unwrap_or(Kind::PercentOff),
        scope: Scope::parse(&row.scope),
        trigger: Trigger::parse(&row.trigger),
        currency: row.currency.clone(),
        value_bp: row.value_bp,
        value: money(row.value_minor, currency),
        min_order: money(row.min_order_minor, currency),
        min_quantity: row.min_quantity,
        max_discount: money(row.max_discount_minor, currency),
        buy_quantity: row.buy_quantity,
        get_quantity: row.get_quantity,
        reward_selection: row.reward_selection.as_deref().and_then(RewardSelection::parse),
        reward_value_bp: row.reward_value_bp,
        max_applications: row.max_applications,
        bundle_quantity: row.bundle_quantity,
        bundle_price: money(row.bundle_price_minor, currency),
        max_add_on_quantity: row.max_add_on_quantity,
        gift_quantity: row.gift_quantity,
        first_order_only: row.first_order_only,
        starts_at: row.starts_at,
        ends_at: row.ends_at,
        priority: row.priority,
        exclusivity: Exclusivity::parse(&row.exclusivity),
        combines_with: Some(
            row.combines_with.split(',').filter_map(parse_class).map(ClassName::of).collect(),
        ),
        stop_after: row.stop_after,
        usage_limit: row.usage_limit,
        per_customer_limit: row.per_customer_limit,
        per_customer_period: Period::parse(&row.per_customer_period),
        daily_usage_limit: row.daily_usage_limit,
        budget: money(row.budget_minor, currency),
        return_usage_on_cancel: Some(row.return_usage_on_cancel),
        gift_return_policy: GiftPolicy::parse(&row.gift_return_policy),
        program_kind: ProgramKind::parse(&row.program_kind),
        voucher_quota: row.voucher_quota,
        per_user_claim_limit: row.per_user_claim_limit,
        voucher_valid_days: row.voucher_valid_days,
        claimable: row.claimable,
        claim_requires_verified_phone: row.claim_requires_verified_phone,
        public: row.public,
        locale: text.map_or_else(|| "en".to_owned(), |text| text.locale.clone()),
        title: text.map(|text| text.title.clone()).unwrap_or_default(),
        terms: text.map(|text| text.terms.clone()).unwrap_or_default(),
        customer_groups: view.groups.iter().map(|group| group.group_id).collect(),
        channels: view.channels.iter().map(|found| found.channel.clone()).collect(),
        conditions: view
            .conditions
            .iter()
            .filter_map(|found| {
                Some(ConditionInput {
                    kind: ConditionKind::parse(&found.kind)?,
                    operator: Operator::parse(&found.operator)?,
                    values: serde_json::from_str(&found.values).unwrap_or_default(),
                })
            })
            .collect(),
        schedules: view
            .schedules
            .iter()
            .map(|found| WindowInput {
                weekdays: (1..=7u8).filter(|day| found.weekdays & (1 << (day - 1)) != 0).collect(),
                start_minute: found.start_minute,
                end_minute: found.end_minute,
            })
            .collect(),
        tiers: view
            .tiers
            .iter()
            .filter_map(|found| {
                Some(TierInput {
                    threshold_kind: ThresholdKind::parse(&found.threshold_kind)?,
                    threshold: found.threshold,
                    value_bp: found.value_bp,
                    value: money(found.value_minor, currency),
                })
            })
            .collect(),
        gift_options: view
            .gifts
            .iter()
            .map(|found| GiftOptionInput {
                variant_id: found.variant_id,
                stock_limit: found.stock_limit,
            })
            .collect(),
        funding: view
            .funding
            .iter()
            .filter_map(|found| {
                Some(FundingInput {
                    payer_kind: Payer::parse(&found.payer_kind)?,
                    payer_org_id: found.payer_org_id,
                    share_bp: found.share_bp,
                })
            })
            .collect(),
    }
}

/// Sau khi bật chỉ được: kéo dài `ends_at`, tăng (hay bỏ) giới hạn lượt / ngày / ngân sách /
/// số voucher, sửa tiêu đề và điều kiện người mua đọc.
pub(crate) fn check_running_change(
    current: &PromotionInput,
    next: &PromotionInput,
) -> Result<(), Error> {
    let raised = |old: Option<i64>, new: Option<i64>| match (old, new) {
        (_, None) => true,
        (None, Some(_)) => false,
        (Some(old), Some(new)) => new >= old,
    };
    let minor = |value: &Option<Money>| value.map(Money::minor);
    let mut allowed = current.clone();
    allowed.ends_at = next.ends_at;
    allowed.usage_limit = next.usage_limit;
    allowed.daily_usage_limit = next.daily_usage_limit;
    allowed.voucher_quota = next.voucher_quota;
    allowed.budget = next.budget;
    allowed.title = next.title.clone();
    allowed.terms = next.terms.clone();
    if allowed != *next
        || next.ends_at < current.ends_at
        || !raised(current.usage_limit, next.usage_limit)
        || !raised(current.daily_usage_limit, next.daily_usage_limit)
        || !raised(current.voucher_quota, next.voucher_quota)
        || !raised(minor(&current.budget), minor(&next.budget))
    {
        return Err(locked_after_start());
    }
    Ok(())
}

/// Ghi chương trình mới (dòng chính, bản dịch, phần con).
pub(crate) async fn insert_promotion<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    id: Uuid,
    org: OrgId,
    currency: Currency,
    input: &PromotionInput,
    actor: &Actor,
) -> Result<(), Error> {
    let at = now();
    let mut row = promotion::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant.id().as_uuid()),
        org_id: Set(org.as_uuid()),
        status: Set("draft".to_owned()),
        usage_count: Set(0),
        budget_used_minor: Set(0),
        issued_count: Set(0),
        version: Set(0),
        created_by: Set(actor.user_id),
        created_at: Set(at),
        updated_at: Set(at),
        activated_at: Set(None),
        ..Default::default()
    };
    fill(&mut row, currency, input);
    row.insert(transaction).await?;
    translation::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant.id().as_uuid()),
        promotion_id: Set(id),
        locale: Set(autoxide_kernel::i18n::canonical_locale(&input.locale).unwrap_or_default()),
        title: Set(input.title.trim().to_owned()),
        terms: Set(input.terms.clone()),
    }
    .insert(transaction)
    .await?;
    replace_children(transaction, tenant, id, org, input).await
}

/// Chép các cột lấy từ đầu vào.
fn fill(row: &mut promotion::ActiveModel, currency: Currency, input: &PromotionInput) {
    let minor = |value: &Option<Money>| value.map(Money::minor);
    row.kind = Set(input.kind.as_str().to_owned());
    row.scope = Set(input.scope().as_str().to_owned());
    row.trigger = Set(input.trigger().as_str().to_owned());
    row.currency = Set(currency.code().to_owned());
    row.value_bp = Set(input.value_bp);
    row.value_minor = Set(minor(&input.value));
    row.min_order_minor = Set(minor(&input.min_order));
    row.min_quantity = Set(input.min_quantity);
    row.max_discount_minor = Set(minor(&input.max_discount));
    row.buy_quantity = Set(input.buy_quantity);
    row.get_quantity = Set(input.get_quantity);
    row.reward_selection = Set(input.reward_selection.map(|found| found.as_str().to_owned()));
    row.reward_value_bp = Set(input.reward_value_bp);
    row.max_applications = Set(input.max_applications);
    row.bundle_quantity = Set(input.bundle_quantity);
    row.bundle_price_minor = Set(minor(&input.bundle_price));
    row.max_add_on_quantity = Set(input.max_add_on_quantity);
    row.gift_quantity = Set(input.gift_quantity);
    row.first_order_only = Set(input.first_order_only);
    row.starts_at = Set(micros(input.starts_at));
    row.ends_at = Set(micros(input.ends_at));
    row.priority = Set(input.priority);
    row.exclusivity = Set(input.exclusivity.unwrap_or(Exclusivity::None).as_str().to_owned());
    row.combines_with = Set(input.combines_with());
    row.stop_after = Set(input.stop_after);
    row.usage_limit = Set(input.usage_limit);
    row.per_customer_limit = Set(input.per_customer_limit);
    row.per_customer_period =
        Set(input.per_customer_period.unwrap_or(Period::Lifetime).as_str().to_owned());
    row.daily_usage_limit = Set(input.daily_usage_limit);
    row.budget_minor = Set(minor(&input.budget));
    row.return_usage_on_cancel = Set(input.return_usage_on_cancel.unwrap_or(true));
    row.gift_return_policy =
        Set(input.gift_return_policy.unwrap_or(GiftPolicy::ReturnGift).as_str().to_owned());
    row.program_kind = Set(input.program_kind.unwrap_or(ProgramKind::Regular).as_str().to_owned());
    row.voucher_quota = Set(input.voucher_quota);
    row.per_user_claim_limit = Set(input.per_user_claim_limit);
    row.voucher_valid_days = Set(input.voucher_valid_days);
    row.claimable = Set(input.claimable);
    row.claim_requires_verified_phone = Set(input.claim_requires_verified_phone);
    row.public = Set(input.public);
}

/// Sửa dòng chính và thay phần con theo đầu vào mới.
pub(crate) async fn update_promotion<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    before: &PromotionView,
    currency: Currency,
    input: &PromotionInput,
) -> Result<(), Error> {
    let mut row: promotion::ActiveModel = before.row.clone().into();
    fill(&mut row, currency, input);
    row.updated_at = Set(now());
    row.update(transaction).await?;
    let locale = autoxide_kernel::i18n::canonical_locale(&input.locale).unwrap_or_default();
    put_translation(transaction, tenant, before, &locale, input.title.trim(), &input.terms).await?;
    replace_children(transaction, tenant, before.row.id, OrgId::from_uuid(before.row.org_id), input)
        .await
}

/// Thay nhóm khách, kênh, điều kiện, khung giờ, bậc, quà, người trả tiền.
async fn replace_children<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    id: Uuid,
    org: OrgId,
    input: &PromotionInput,
) -> Result<(), Error> {
    let tenant_id = tenant.id().as_uuid();
    macro_rules! clear {
        ($module:ident) => {
            $module::Entity::delete_many()
                .filter($module::Column::PromotionId.eq(id))
                .scoped(tenant)
                .exec(transaction)
                .await?;
        };
    }
    clear!(customer_group);
    clear!(channel);
    clear!(condition);
    clear!(schedule);
    clear!(tier);
    clear!(funding);
    // Quà: giữ số đã dùng của quà còn trong danh sách.
    let used: Vec<gift_option::Model> = gift_option::Entity::find()
        .filter(gift_option::Column::PromotionId.eq(id))
        .scoped(tenant)
        .all(transaction)
        .await?;
    clear!(gift_option);
    let groups: std::collections::BTreeSet<Uuid> = input.customer_groups.iter().copied().collect();
    for group in groups {
        customer_group::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            promotion_id: Set(id),
            group_id: Set(group),
        }
        .insert(transaction)
        .await?;
    }
    let channels: std::collections::BTreeSet<&String> = input.channels.iter().collect();
    for name in channels {
        channel::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            promotion_id: Set(id),
            channel: Set(name.clone()),
        }
        .insert(transaction)
        .await?;
    }
    for found in &input.conditions {
        condition::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            promotion_id: Set(id),
            kind: Set(found.kind.as_str().to_owned()),
            operator: Set(found.operator.as_str().to_owned()),
            values: Set(serde_json::to_string(&found.values).unwrap_or_else(|_| "[]".to_owned())),
        }
        .insert(transaction)
        .await?;
    }
    for window in &input.schedules {
        schedule::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            promotion_id: Set(id),
            weekdays: Set(weekday_mask(&window.weekdays)),
            start_minute: Set(window.start_minute),
            end_minute: Set(window.end_minute),
        }
        .insert(transaction)
        .await?;
    }
    for found in &input.tiers {
        tier::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            promotion_id: Set(id),
            threshold_kind: Set(found.threshold_kind.as_str().to_owned()),
            threshold: Set(found.threshold),
            value_bp: Set(found.value_bp),
            value_minor: Set(found.value.map(Money::minor)),
        }
        .insert(transaction)
        .await?;
    }
    for gift in &input.gift_options {
        let used = used
            .iter()
            .find(|found| found.variant_id == gift.variant_id)
            .map_or(0, |found| found.quantity_used);
        gift_option::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            promotion_id: Set(id),
            variant_id: Set(gift.variant_id),
            stock_limit: Set(gift.stock_limit),
            quantity_used: Set(used),
        }
        .insert(transaction)
        .await?;
    }
    // Mặc định: nút chạy chương trình trả 100%.
    let shares = if input.funding.is_empty() {
        vec![FundingInput {
            payer_kind: Payer::Seller,
            payer_org_id: Some(org.as_uuid()),
            share_bp: 10_000,
        }]
    } else {
        input.funding.clone()
    };
    for share in shares {
        funding::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            promotion_id: Set(id),
            payer_kind: Set(share.payer_kind.as_str().to_owned()),
            payer_org_id: Set(share.payer_org_id),
            share_bp: Set(share.share_bp),
        }
        .insert(transaction)
        .await?;
    }
    Ok(())
}

/// Xoá chương trình `draft` và mọi phần con.
pub(crate) async fn delete_promotion<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<(), Error> {
    macro_rules! clear {
        ($module:ident) => {
            $module::Entity::delete_many()
                .filter($module::Column::PromotionId.eq(id))
                .scoped(tenant)
                .exec(transaction)
                .await?;
        };
    }
    clear!(translation);
    clear!(target);
    clear!(customer_group);
    clear!(channel);
    clear!(condition);
    clear!(schedule);
    clear!(tier);
    clear!(gift_option);
    clear!(flash_item);
    clear!(funding);
    use crate::entity::code;
    clear!(code);
    promotion::Entity::delete_many()
        .filter(promotion::Column::Id.eq(id))
        .scoped(tenant)
        .exec(transaction)
        .await?;
    Ok(())
}

/// Đặt bản dịch của `locale`.
pub(crate) async fn put_translation<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    view: &PromotionView,
    locale: &str,
    title: &str,
    terms: &str,
) -> Result<(), Error> {
    match view.texts.iter().find(|text| text.locale == locale) {
        Some(found) => {
            let mut row: translation::ActiveModel = found.clone().into();
            row.title = Set(title.to_owned());
            row.terms = Set(terms.to_owned());
            row.update(transaction).await?;
        }
        None => {
            translation::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                promotion_id: Set(view.row.id),
                locale: Set(locale.to_owned()),
                title: Set(title.to_owned()),
                terms: Set(terms.to_owned()),
            }
            .insert(transaction)
            .await?;
        }
    }
    Ok(())
}

/// Thay mục tiêu.
pub(crate) async fn put_targets<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    view: &PromotionView,
    targets: &[TargetInput],
) -> Result<(), Error> {
    target::Entity::delete_many()
        .filter(target::Column::PromotionId.eq(view.row.id))
        .scoped(tenant)
        .exec(transaction)
        .await?;
    let mut seen = std::collections::BTreeSet::new();
    for found in targets {
        let role = found.role.unwrap_or(Role::Condition);
        if !seen.insert((found.target_type, found.target_id, role)) {
            continue;
        }
        target::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            promotion_id: Set(view.row.id),
            target_type: Set(found.target_type.as_str().to_owned()),
            target_id: Set(found.target_id),
            role: Set(role.as_str().to_owned()),
        }
        .insert(transaction)
        .await?;
    }
    Ok(())
}

/// Thay giá flash sale / giá mua kèm.
pub(crate) async fn put_flash_items<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    view: &PromotionView,
    items: &[FlashItemInput],
) -> Result<(), Error> {
    flash_item::Entity::delete_many()
        .filter(flash_item::Column::PromotionId.eq(view.row.id))
        .scoped(tenant)
        .exec(transaction)
        .await?;
    let mut seen = std::collections::BTreeSet::new();
    for item in items {
        if !seen.insert(item.variant_id) {
            continue;
        }
        flash_item::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            promotion_id: Set(view.row.id),
            variant_id: Set(item.variant_id),
            sale_price_minor: Set(item.sale_price_minor),
            value_bp: Set(item.value_bp),
            quantity_limit: Set(item.quantity_limit),
            quantity_used: Set(0),
            per_customer_quantity_limit: Set(item.per_customer_quantity_limit),
        }
        .insert(transaction)
        .await?;
    }
    Ok(())
}
