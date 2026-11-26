//! Chương trình khuyến mại: tạo (`draft`), đọc, sửa, xoá, bản dịch, mục tiêu, giá flash sale.
//! Sửa sau khi bật bị giới hạn (kéo dài, tăng lượt / ngân sách, sửa bản dịch); mọi lần đổi sau khi
//! bật thêm một dòng `promotions_versions` (lưu vết). Mọi lần đổi tăng `promotions_revisions` trong
//! cùng transaction để bộ nhớ `CompiledPromotions` nạp lại.

pub(crate) mod input;
mod store;

pub(crate) use store::load as store_load;
pub(crate) mod transitions;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::pagination::{Page, PageRequest, SortValue, fetch_page, fetch_page_by};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use sea_orm::{ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::{
    channel, condition, customer_group, flash_item, funding, gift_option, promotion, schedule,
    target, tier, translation, version,
};
use crate::events::{self, Change};
use crate::model::{Role, Status, TargetType};
use crate::permissions::{READ, WRITE};
use crate::service::{PromotionsService, bump_revision, default_org, readable, writable};

pub(crate) use input::PromotionInput;
pub(crate) use store::{input_of, load_children};

/// A product, variant or category a promotion targets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsTargetInput)]
pub(crate) struct TargetInput {
    pub target_type: TargetType,
    pub target_id: Uuid,
    /// `condition` (what must be bought / what is discounted) or `reward` (`buy_x_get_y`
    /// rewards). Defaults to `condition`.
    pub role: Option<Role>,
}

/// The sale price of a variant in a flash sale or the deal price of an add-on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsFlashItemInput)]
pub(crate) struct FlashItemInput {
    pub variant_id: Uuid,
    /// Sale price per unit (minor units of the promotion's currency); or give `value_bp`.
    pub sale_price_minor: Option<i64>,
    /// Percent off in basis points.
    pub value_bp: Option<i32>,
    /// Units sold at this price; none = no limit.
    pub quantity_limit: Option<i64>,
    /// Units one buyer may get at this price.
    pub per_customer_quantity_limit: Option<i64>,
}

/// Một chương trình đã đọc kèm mọi phần con.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PromotionView {
    pub row: promotion::Model,
    pub texts: Vec<translation::Model>,
    pub targets: Vec<target::Model>,
    pub groups: Vec<customer_group::Model>,
    pub channels: Vec<channel::Model>,
    pub conditions: Vec<condition::Model>,
    pub schedules: Vec<schedule::Model>,
    pub tiers: Vec<tier::Model>,
    pub gifts: Vec<gift_option::Model>,
    pub flash: Vec<flash_item::Model>,
    pub funding: Vec<funding::Model>,
    /// Người bán đăng ký tham gia (chương trình của sàn).
    pub participations: Vec<crate::entity::participation::Model>,
}

impl PromotionView {
    pub(crate) fn status(&self) -> Status {
        Status::parse(&self.row.status).unwrap_or(Status::Draft)
    }

    /// Ảnh chụp đủ để dựng lại chương trình (lưu vết, event).
    pub(crate) fn snapshot(&self) -> Value {
        json!({
            "promotion": store::row_json(&self.row),
            "texts": self.texts.iter().map(|text| json!({
                "locale": text.locale, "title": text.title, "terms": text.terms,
            })).collect::<Vec<_>>(),
            "targets": self.targets.iter().map(|found| json!({
                "target_type": found.target_type, "target_id": found.target_id, "role": found.role,
            })).collect::<Vec<_>>(),
            "flash_items": self.flash.iter().map(|item| json!({
                "variant_id": item.variant_id, "sale_price_minor": item.sale_price_minor,
                "value_bp": item.value_bp, "quantity_limit": item.quantity_limit,
            })).collect::<Vec<_>>(),
            "input": serde_json::to_value(store::input_of(self)).unwrap_or(Value::Null),
        })
    }
}

/// Bộ lọc danh sách chương trình.
#[derive(Debug, Clone, Default)]
pub(crate) struct PromotionFilter {
    pub status: Option<Status>,
    pub kind: Option<crate::model::Kind>,
    pub org_id: Option<Uuid>,
    pub q: Option<String>,
    /// Chỉ chương trình có thời gian chạy giao với [từ, tới) (lịch), trừ bản nháp và đã huỷ.
    pub window: Option<(Timestamp, Timestamp)>,
}

pub(crate) fn not_found(id: Uuid) -> Error {
    Error::not_found("promotions.not_found", "There is no promotion `{id}` in your scope.")
        .with_param("id", id)
}

pub(crate) fn locked_after_start() -> Error {
    Error::conflict(
        "promotions.locked_after_start",
        "A promotion that was turned on can only be extended, get more uses or budget, be paused \
         or ended, and have its texts changed. Create a new promotion for other changes.",
    )
}

/// Ghi một dòng lưu vết (chỉ thêm) và tăng số phiên bản của chương trình.
pub(crate) async fn record_version<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    view: &PromotionView,
    action: &str,
    actor: &Actor,
) -> Result<i32, Error> {
    use sea_orm::{ActiveModelTrait, Set};
    let next = view.row.version + 1;
    version::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant.id().as_uuid()),
        promotion_id: Set(view.row.id),
        version: Set(next),
        action: Set(action.to_owned()),
        snapshot: Set(view.snapshot().to_string()),
        changed_by: Set(actor.user_id),
        changed_at: Set(crate::service::now()),
    }
    .insert(transaction)
    .await?;
    promotion::Entity::update_many()
        .col_expr(promotion::Column::Version, sea_orm::sea_query::Expr::value(next))
        .filter(promotion::Column::Id.eq(view.row.id))
        .scoped(tenant)
        .exec(transaction)
        .await?;
    Ok(next)
}

impl PromotionsService {
    /// Tạo chương trình ở `draft`.
    pub(crate) async fn create_promotion(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: PromotionInput,
    ) -> Result<PromotionView, Error> {
        let currency = input::check(&input)?;
        let org = match input.org_id {
            Some(org) => OrgId::from_uuid(org),
            None => default_org(tenant, principal, &WRITE)?,
        };
        let id = Uuid::now_v7();
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        principal.require_write_at(&transaction, &WRITE, crate::service::at(org.as_uuid())).await?;
        store::insert_promotion(&transaction, tenant, id, org, currency, &input, &actor).await?;
        let view = store::load(&transaction, tenant, id).await?.ok_or_else(|| not_found(id))?;
        bump_revision(&transaction, tenant).await?;
        self.outbox
            .record(
                &transaction,
                tenant,
                actor,
                events::promotion_subject(id),
                &events::PromotionCreated(Change::after(view.snapshot())),
            )
            .await?;
        transaction.commit().await?;
        self.compiled.invalidate(tenant.id());
        Ok(view)
    }

    /// Một chương trình trong phạm vi người gọi.
    pub(crate) async fn promotion(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<PromotionView, Error> {
        let read = tenant.database().read();
        let view = store::load(read, tenant, id).await?.ok_or_else(|| not_found(id))?;
        readable(principal, read, &READ, view.row.org_id, || not_found(id)).await?;
        Ok(view)
    }

    /// Danh sách chương trình trong phạm vi người gọi, kèm tiêu đề.
    pub(crate) async fn list_promotions(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: &PromotionFilter,
        page: &PageRequest,
    ) -> Result<Page<(promotion::Model, String)>, Error> {
        let read = tenant.database().read();
        let mut condition = Condition::all().add(principal.scope_condition(
            &READ,
            promotion::Column::OrgId,
            None,
        )?);
        if let Some(status) = filter.status {
            condition = condition.add(promotion::Column::Status.eq(status.as_str()));
        }
        if let Some(kind) = filter.kind {
            condition = condition.add(promotion::Column::Kind.eq(kind.as_str()));
        }
        if let Some(org) = filter.org_id {
            condition = condition.add(promotion::Column::OrgId.eq(org));
        }
        if let Some((from, to)) = filter.window {
            condition = condition
                .add(promotion::Column::StartsAt.lt(to))
                .add(promotion::Column::EndsAt.gt(from))
                .add(
                    promotion::Column::Status
                        .is_not_in([Status::Draft.as_str(), Status::Cancelled.as_str()]),
                );
        }
        if let Some(q) = filter.q.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
            let ids: Vec<Uuid> = translation::Entity::find()
                .filter(
                    translation::Column::Title
                        .like(sea_orm::sea_query::LikeExpr::new(contains_pattern(q)).escape('\\')),
                )
                .scoped(tenant)
                .all(read)
                .await?
                .into_iter()
                .map(|row| row.promotion_id)
                .collect();
            condition = condition.add(promotion::Column::Id.is_in(ids));
        }
        let select = promotion::Entity::find().filter(condition).scoped(tenant);
        let page = fetch_page(select, promotion::Column::Id, |row| row.id, page, read).await?;
        let ids: Vec<Uuid> = page.data.iter().map(|row| row.id).collect();
        let texts = translation::Entity::find()
            .filter(translation::Column::PromotionId.is_in(ids))
            .scoped(tenant)
            .order_by_asc(translation::Column::Id)
            .all(read)
            .await?;
        Ok(page.map(|row| {
            let title = texts
                .iter()
                .find(|text| text.promotion_id == row.id)
                .map(|text| text.title.clone())
                .unwrap_or_default();
            (row, title)
        }))
    }

    /// Sửa chương trình: `patch` là object chứa các trường của thân tạo; trường có mặt thay giá
    /// trị hiện có, `null` xoá trường tuỳ chọn. Sau khi bật chỉ được kéo dài, tăng giới hạn, sửa
    /// tiêu đề / điều kiện người mua đọc (`promotions.locked_after_start`).
    pub(crate) async fn update_promotion(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        patch: Value,
    ) -> Result<PromotionView, Error> {
        let Value::Object(changes) = patch else {
            return Err(Error::validation(vec![FieldError::new(
                "body",
                "type",
                "must be an object of the fields to change",
            )]));
        };
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        // Khoá chương trình: trạng thái quyết phần nào còn sửa được (khoá sau khi bắt đầu) — chương
        // trình vừa được kích hoạt thì lần sửa này thấy trạng thái mới.
        let before =
            store::load_locked(&transaction, tenant, id).await?.ok_or_else(|| not_found(id))?;
        writable(principal, &transaction, &WRITE, before.row.org_id, || not_found(id)).await?;
        let current = store::input_of(&before);
        let mut merged = serde_json::to_value(&current).unwrap_or(Value::Null);
        if let Value::Object(fields) = &mut merged {
            for (key, value) in changes {
                fields.insert(key, value);
            }
        }
        let next: PromotionInput = serde_json::from_value(merged).map_err(|error| {
            Error::validation(vec![
                FieldError::new("body", "invalid", "{reason}")
                    .with_param("reason", error.to_string()),
            ])
        })?;
        let currency = input::check(&next)?;
        if next.org_id != current.org_id {
            return Err(Error::validation(vec![FieldError::new(
                "org_id",
                "immutable",
                "a promotion cannot move to another unit",
            )]));
        }
        let status = before.status();
        match status {
            Status::Draft => {}
            Status::Scheduled | Status::Active | Status::Paused => {
                store::check_running_change(&current, &next)?;
            }
            Status::Ended | Status::Cancelled => {
                return Err(transitions::invalid_transition(status, status));
            }
        }
        store::update_promotion(&transaction, tenant, &before, currency, &next).await?;
        let after = store::load(&transaction, tenant, id).await?.ok_or_else(|| not_found(id))?;
        if status != Status::Draft {
            record_version(&transaction, tenant, &after, "updated", &actor).await?;
        }
        bump_revision(&transaction, tenant).await?;
        record_change(&self.outbox, &transaction, tenant, actor, &before, &after).await?;
        // Đang chạy mà đổi (vd kéo dài): hạn của giá giảm tự động đổi theo.
        if status == Status::Active
            && let Some(prices) = events::automatic_prices_changed(&after)
        {
            self.outbox
                .record(&transaction, tenant, actor, events::promotion_subject(id), &prices)
                .await?;
        }
        transaction.commit().await?;
        self.compiled.invalidate(tenant.id());
        self.promotion(tenant, principal, id).await
    }

    /// Xoá chương trình `draft`.
    pub(crate) async fn delete_promotion(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let view =
            store::load_locked(&transaction, tenant, id).await?.ok_or_else(|| not_found(id))?;
        writable(principal, &transaction, &WRITE, view.row.org_id, || not_found(id)).await?;
        if view.status() != Status::Draft {
            return Err(Error::new(
                ErrorKind::Conflict,
                "promotions.not_deletable",
                "Only a draft promotion can be deleted; end or cancel it instead.",
            ));
        }
        store::delete_promotion(&transaction, tenant, id).await?;
        bump_revision(&transaction, tenant).await?;
        transaction.commit().await?;
        self.compiled.invalidate(tenant.id());
        Ok(())
    }

    /// Đặt tiêu đề và điều kiện theo một ngôn ngữ (được cả sau khi bật — không đổi điều kiện).
    pub(crate) async fn put_translation(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        locale: &str,
        title: &str,
        terms: &str,
    ) -> Result<PromotionView, Error> {
        let locale = autoxide_kernel::i18n::canonical_locale(locale).ok_or_else(|| {
            Error::validation(vec![FieldError::new(
                "locale",
                "invalid",
                "must be a language code like `vi`",
            )])
        })?;
        let mut problems = Vec::new();
        if title.trim().is_empty() || title.chars().count() > 200 {
            problems.push(FieldError::new("title", "length", "must be 1 to 200 characters long"));
        }
        if terms.chars().count() > 5000 {
            problems.push(FieldError::new(
                "terms",
                "length",
                "must be at most 5000 characters long",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let change = ChildChange::Translation {
            locale,
            title: title.trim().to_owned(),
            terms: terms.to_owned(),
        };
        self.change(tenant, principal, id, change).await
    }

    /// Thay mục tiêu (chỉ `draft`).
    pub(crate) async fn put_targets(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        targets: Vec<TargetInput>,
    ) -> Result<PromotionView, Error> {
        if targets.len() > 1000 {
            return Err(Error::validation(vec![FieldError::new(
                "targets",
                "length",
                "at most 1000 targets",
            )]));
        }
        self.change(tenant, principal, id, ChildChange::Targets(targets)).await
    }

    /// Thay giá flash sale / giá mua kèm (chỉ `draft`).
    pub(crate) async fn put_flash_items(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        items: Vec<FlashItemInput>,
    ) -> Result<PromotionView, Error> {
        let mut problems = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let one = item.sale_price_minor.is_some() != item.value_bp.is_some();
            if !one
                || item.sale_price_minor.is_some_and(|price| price < 0)
                || item.value_bp.is_some_and(|bp| !(1..=10_000).contains(&bp))
                || item.quantity_limit.is_some_and(|limit| limit < 1)
                || item.per_customer_quantity_limit.is_some_and(|limit| limit < 1)
            {
                problems.push(FieldError::new(
                    format!("items[{index}]"),
                    "invalid",
                    "exactly one of `sale_price_minor` / `value_bp`; limits at least 1",
                ));
            }
        }
        if items.len() > 1000 {
            problems.push(FieldError::new("items", "length", "at most 1000 items"));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        self.change(tenant, principal, id, ChildChange::FlashItems(items)).await
    }

    /// Khung chung của một thay đổi phần con: khoá theo trạng thái, ghi, lưu vết, event.
    async fn change(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        change: ChildChange,
    ) -> Result<PromotionView, Error> {
        let (action, after_start) = match &change {
            ChildChange::Translation { .. } => ("translation", true),
            ChildChange::Targets(_) => ("targets", false),
            ChildChange::FlashItems(_) => ("flash_items", false),
        };
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        // Khoá chương trình: trạng thái quyết phần nào còn sửa được (khoá sau khi bắt đầu) — chương
        // trình vừa được kích hoạt thì lần sửa này thấy trạng thái mới.
        let before =
            store::load_locked(&transaction, tenant, id).await?.ok_or_else(|| not_found(id))?;
        writable(principal, &transaction, &WRITE, before.row.org_id, || not_found(id)).await?;
        let status = before.status();
        if matches!(status, Status::Ended | Status::Cancelled) {
            return Err(transitions::invalid_transition(status, status));
        }
        if status != Status::Draft && !after_start {
            return Err(locked_after_start());
        }
        match &change {
            ChildChange::Translation { locale, title, terms } => {
                store::put_translation(&transaction, tenant, &before, locale, title, terms).await?;
            }
            ChildChange::Targets(targets) => {
                store::put_targets(&transaction, tenant, &before, targets).await?;
            }
            ChildChange::FlashItems(items) => {
                store::put_flash_items(&transaction, tenant, &before, items).await?;
            }
        }
        let after = store::load(&transaction, tenant, id).await?.ok_or_else(|| not_found(id))?;
        if status != Status::Draft {
            record_version(&transaction, tenant, &after, action, &actor).await?;
        }
        bump_revision(&transaction, tenant).await?;
        record_change(&self.outbox, &transaction, tenant, actor, &before, &after).await?;
        transaction.commit().await?;
        self.compiled.invalidate(tenant.id());
        self.promotion(tenant, principal, id).await
    }

    /// Lưu vết của chương trình, cũ trước.
    pub(crate) async fn versions(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<version::Model>, Error> {
        let view = self.promotion(tenant, principal, id).await?;
        // Keyset trong database theo (`version`, id); `version` duy nhất trong một chương trình nên index unique
        // `promotions_versions_unique` (tenant_id, promotion_id, version) đã đủ cho trang.
        let select = version::Entity::find()
            .filter(version::Column::PromotionId.eq(view.row.id))
            .scoped(tenant);
        fetch_page_by(
            select,
            version::Column::Version,
            |row| SortValue::Integer(i64::from(row.version)),
            version::Column::Id,
            |row| row.id,
            page,
            tenant.database().read(),
        )
        .await
    }
}

/// Một thay đổi phần con của chương trình.
enum ChildChange {
    Translation { locale: String, title: String, terms: String },
    Targets(Vec<TargetInput>),
    FlashItems(Vec<FlashItemInput>),
}

async fn record_change<C: ConnectionTrait>(
    outbox: &Outbox,
    transaction: &C,
    tenant: &TenantContext,
    actor: Actor,
    before: &PromotionView,
    after: &PromotionView,
) -> Result<(), Error> {
    outbox
        .record(
            transaction,
            tenant,
            actor,
            events::promotion_subject(after.row.id),
            &events::PromotionUpdated(Change {
                before: Some(before.snapshot()),
                after: Some(after.snapshot()),
            }),
        )
        .await
}

/// Mẫu `LIKE … ESCAPE '\'` "chứa `text`": `%`, `_`, `\` người dùng gõ là chữ thường, không phải ký tự
/// đại diện.
fn contains_pattern(text: &str) -> String {
    let mut pattern = String::with_capacity(text.len() + 2);
    pattern.push('%');
    for character in text.chars() {
        if matches!(character, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(character);
    }
    pattern.push('%');
    pattern
}
