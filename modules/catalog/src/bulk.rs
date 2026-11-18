//! Đổi giá hàng loạt (D-222): tạo (`draft`) → xem trước (`previewed`: số biến thể, 20 dòng mẫu
//! trước / sau, vi phạm sàn giá; không ghi giá) → duyệt (`approved`, permission
//! `catalog.prices.approve`; cài đặt theo nút `catalog.price_change_requires_second_person` bật thì
//! người duyệt khác người tạo) → job ghi dòng giá `source = bulk`, `reference =
//! bulk:<lần đổi>:<biến thể>` theo lô (`applied`). `valid_to` cho giá tạm thời: hết hạn thì giá cũ
//! tự hiệu lực lại. Hoàn tác = một lần đổi mới về giá cũ.

use std::collections::HashMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::money::Currency;
use autoxide_kernel::tenancy::{OrgId, OrgSetting, ScopeExt, TenantContext};
use chrono::{DateTime, Utc};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::api::FloorAction;
use crate::entity::{price, price_change, price_list, product_category, rounding_rule, variant};
use crate::errors;
use crate::permissions::{PRICES_APPROVE, PRICES_READ, PRICES_WRITE};
use crate::rounding::{RoundingMode, round};
use crate::service::{CatalogService, fail_if_any, micros, now, writable};

/// Loại job ghi dòng giá của một lần đổi đã duyệt.
pub(crate) const APPLY_CHANGE: &str = "catalog.apply_price_change";
/// Số biến thể mỗi transaction khi ghi.
const BATCH: usize = 500;
/// Số dòng mẫu của bản xem trước.
const SAMPLES: usize = 20;
/// Số biến thể tối đa một lần đổi.
const MAX_TARGETS: usize = 50_000;

/// Cài đặt kế thừa theo nút: người duyệt lần đổi giá phải khác người tạo.
pub(crate) const SECOND_PERSON: OrgSetting = OrgSetting::new(
    "catalog.price_change_requires_second_person",
    "Bulk price changes must be approved by someone other than the person who made them.",
)
.with_default("false")
.with_schema(r#"{"type":"boolean"}"#);

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ApplyChange {
    pub change: Uuid,
}

impl Job for ApplyChange {
    const KIND: &'static str = APPLY_CHANGE;
}

/// Phạm vi biến thể của lần đổi: hợp của các danh mục (gồm danh mục con), sản phẩm và biến thể
/// nêu. Rỗng hết thì mọi biến thể đang có giá trong bảng.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ChangeFilter {
    #[serde(default)]
    pub category_ids: Vec<Uuid>,
    #[serde(default)]
    pub product_ids: Vec<Uuid>,
    #[serde(default)]
    pub variant_ids: Vec<Uuid>,
}

/// Lần đổi mới.
#[derive(Debug, Clone)]
pub(crate) struct NewChange {
    pub filter: ChangeFilter,
    /// `set` (giá mới = `value`), `percent` (phần vạn, có dấu: −1000 = −10%), `amount` (cộng
    /// `value` đơn vị nhỏ nhất, có dấu).
    pub operation: String,
    pub value: i64,
    pub rounding_rule_id: Option<Uuid>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_to: Option<DateTime<Utc>>,
}

/// Một dòng của bản xem trước.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PreviewRow {
    pub variant_id: Uuid,
    pub before: Option<i64>,
    pub after: i64,
    /// Giá mới dưới sàn: `warn` hay `block`.
    pub floor_violation: Option<String>,
}

fn change_not_found(id: Uuid) -> Error {
    Error::not_found("catalog.price_change_not_found", "There is no price change `{id}`.")
        .with_param("id", id)
}

/// Giá mới theo phép đổi, làm tròn; `None` khi phép đổi cần giá cũ mà biến thể chưa có giá.
fn apply(
    operation: &str,
    value: i64,
    before: Option<i64>,
    rule: Option<&rounding_rule::Model>,
) -> Option<i64> {
    let raw = match operation {
        "set" => value,
        "percent" => {
            let before = i128::from(before?);
            let scaled = (before * i128::from(10_000 + value) + 5_000).div_euclid(10_000);
            i64::try_from(scaled.max(0)).unwrap_or(i64::MAX)
        }
        _ => before?.saturating_add(value).max(0),
    };
    Some(match rule {
        Some(rule) => round(
            raw,
            rule.step_minor,
            rule.ending_minor.unwrap_or(0),
            RoundingMode::parse(&rule.mode).unwrap_or(RoundingMode::Nearest),
        ),
        None => raw,
    })
}

impl CatalogService {
    /// Tạo lần đổi giá hàng loạt (nháp) cho bảng giá `list_id`.
    pub(crate) async fn create_change(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        list_id: Uuid,
        input: NewChange,
    ) -> Result<price_change::Model, Error> {
        let mut problems = Vec::new();
        if !matches!(input.operation.as_str(), "set" | "percent" | "amount") {
            problems.push(FieldError::new(
                "operation",
                "one_of",
                "must be `set`, `percent` or `amount`",
            ));
        }
        if input.operation == "set" && input.value < 0 {
            problems.push(FieldError::new("value", "range", "must not be negative"));
        }
        if input.operation == "percent" && !(-10_000..=100_000).contains(&input.value) {
            problems.push(FieldError::new(
                "value",
                "range",
                "must be -10000 to 100000 basis points",
            ));
        }
        let valid_from = micros(input.valid_from.unwrap_or_else(now));
        let valid_to = input.valid_to.map(micros);
        if valid_to.is_some_and(|to| to <= valid_from) {
            problems.push(FieldError::new("valid_to", "range", "must be after `valid_from`"));
        }
        fail_if_any(problems)?;
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        let list = price_list::Entity::find_by_id(list_id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| errors::price_list_not_found(list_id))?;
        writable(principal, &transaction, &PRICES_READ, &PRICES_WRITE, list.org_id, || {
            errors::price_list_not_found(list_id)
        })
        .await?;
        if let Some(rule) = input.rounding_rule_id {
            rounding_rule::Entity::find_by_id(rule)
                .scoped(tenant)
                .one(&transaction)
                .await?
                .ok_or_else(|| rounding_rule_not_found(rule))?;
        }
        let now = now();
        let row = price_change::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            price_list_id: Set(list_id),
            filter: Set(serde_json::to_string(&input.filter).unwrap_or_else(|_| "{}".to_owned())),
            operation: Set(input.operation),
            value: Set(input.value),
            rounding_rule_id: Set(input.rounding_rule_id),
            valid_from: Set(valid_from),
            valid_to: Set(valid_to),
            status: Set("draft".to_owned()),
            affected: Set(0),
            preview: Set(None),
            created_by: Set(actor.user_id),
            approved_by: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&transaction)
        .await?;
        transaction.commit().await?;
        Ok(row)
    }

    /// Lần đổi `id` (kiểm quyền đọc giá ở nút của bảng giá).
    pub(crate) async fn get_change(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        list_id: Uuid,
        id: Uuid,
    ) -> Result<price_change::Model, Error> {
        self.get_price_list(tenant, principal, list_id).await?;
        price_change::Entity::find_by_id(id)
            .filter(price_change::Column::PriceListId.eq(list_id))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| change_not_found(id))
    }

    /// Biến thể trong phạm vi lần đổi và giá mới của từng biến thể (đọc, không ghi).
    async fn plan(
        &self,
        tenant: &TenantContext,
        change: &price_change::Model,
        list: &price_list::Model,
    ) -> Result<Vec<PreviewRow>, Error> {
        let read = tenant.database().read();
        let filter: ChangeFilter = serde_json::from_str(&change.filter).unwrap_or_default();
        let mut variant_ids: Vec<Uuid> = filter.variant_ids.clone();
        let mut product_ids = filter.product_ids.clone();
        for category in &filter.category_ids {
            let below = self.descendants(read, tenant, *category).await?;
            product_ids.extend(
                product_category::Entity::find()
                    .filter(product_category::Column::CategoryId.is_in(below))
                    .scoped(tenant)
                    .all(read)
                    .await?
                    .into_iter()
                    .map(|link| link.product_id),
            );
        }
        if !product_ids.is_empty() {
            variant_ids.extend(
                variant::Entity::find()
                    .filter(variant::Column::ProductId.is_in(product_ids))
                    .scoped(tenant)
                    .all(read)
                    .await?
                    .into_iter()
                    .map(|row| row.id),
            );
        }
        if filter == ChangeFilter::default() {
            variant_ids = price::Entity::find()
                .filter(price::Column::PriceListId.eq(list.id))
                .scoped(tenant)
                .all(read)
                .await?
                .into_iter()
                .map(|row| row.variant_id)
                .collect();
        }
        variant_ids.sort();
        variant_ids.dedup();
        if variant_ids.len() > MAX_TARGETS {
            return Err(Error::validation(vec![FieldError::new(
                "filter",
                "too_many",
                "matches more than 50000 variants; narrow it",
            )]));
        }
        // Giá hiện tại của chính bảng này lúc `valid_from` (không theo thứ tự chọn bảng).
        let rows = price::Entity::find()
            .filter(price::Column::PriceListId.eq(list.id))
            .filter(price::Column::VariantId.is_in(variant_ids.clone()))
            .filter(price::Column::ValidFrom.lte(change.valid_from))
            .scoped(tenant)
            .all(read)
            .await?;
        let mut current: HashMap<Uuid, &price::Model> = HashMap::new();
        for row in rows.iter().filter(|row| row.valid_to.is_none_or(|to| to > change.valid_from)) {
            let better = current
                .get(&row.variant_id)
                .is_none_or(|best| (row.valid_from, row.id) > (best.valid_from, best.id));
            if better {
                current.insert(row.variant_id, row);
            }
        }
        let rule = match change.rounding_rule_id {
            Some(id) => rounding_rule::Entity::find_by_id(id).scoped(tenant).one(read).await?,
            None => None,
        };
        let currency = Currency::parse(&list.currency)?;
        let mut floors = Vec::new();
        for chunk in variant_ids.chunks(crate::api::MAX_IDS) {
            floors
                .extend(self.floors(tenant, OrgId::from_uuid(list.org_id), currency, chunk).await?);
        }
        let mut out = Vec::new();
        for variant_id in variant_ids {
            let before = current.get(&variant_id).map(|row| row.amount_minor);
            let Some(after) = apply(&change.operation, change.value, before, rule.as_ref()) else {
                continue;
            };
            let floor_violation =
                floors.iter().find(|floor| floor.variant_id == variant_id).and_then(|floor| {
                    let minimum = floor.floor?;
                    (after < minimum.minor()).then(|| match floor.action {
                        FloorAction::Block => "block".to_owned(),
                        FloorAction::Warn => "warn".to_owned(),
                    })
                });
            out.push(PreviewRow { variant_id, before, after, floor_violation });
        }
        Ok(out)
    }

    /// Xem trước lần đổi: số biến thể, 20 dòng mẫu, vi phạm sàn giá. Không ghi giá.
    pub(crate) async fn preview_change(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        list_id: Uuid,
        id: Uuid,
    ) -> Result<price_change::Model, Error> {
        let change = self.get_change(tenant, principal, list_id, id).await?;
        if !matches!(change.status.as_str(), "draft" | "previewed") {
            return Err(change_state(&change.status));
        }
        let list = self.get_price_list(tenant, principal, list_id).await?;
        let plan = self.plan(tenant, &change, &list).await?;
        let violations: Vec<&PreviewRow> =
            plan.iter().filter(|row| row.floor_violation.is_some()).collect();
        let preview = json!({
            "samples": plan.iter().take(SAMPLES).collect::<Vec<_>>(),
            "floor_violations": violations.iter().take(SAMPLES).collect::<Vec<_>>(),
            "blocked": violations.iter().filter(|row| row.floor_violation.as_deref() == Some("block")).count(),
            "warned": violations.iter().filter(|row| row.floor_violation.as_deref() == Some("warn")).count(),
        });
        let transaction = tenant.database().begin_write().await?;
        writable(principal, &transaction, &PRICES_READ, &PRICES_WRITE, list.org_id, || {
            errors::price_list_not_found(list_id)
        })
        .await?;
        let mut row = change.into_active_model();
        row.status = Set("previewed".to_owned());
        row.affected = Set(i64::try_from(plan.len()).unwrap_or(i64::MAX));
        row.preview = Set(Some(preview.to_string()));
        row.updated_at = Set(now());
        let saved = row.update(&transaction).await?;
        transaction.commit().await?;
        Ok(saved)
    }

    /// Duyệt lần đổi đã xem trước; xếp job ghi giá.
    pub(crate) async fn approve_change(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        list_id: Uuid,
        id: Uuid,
    ) -> Result<price_change::Model, Error> {
        let change = self.get_change(tenant, principal, list_id, id).await?;
        let list = self.get_price_list(tenant, principal, list_id).await?;
        if change.status == "draft" {
            return Err(Error::conflict(
                "catalog.price_change_not_previewed",
                "Preview the price change before approving it.",
            ));
        }
        if change.status != "previewed" {
            return Err(change_state(&change.status));
        }
        let second = self.second_person_required(tenant, OrgId::from_uuid(list.org_id)).await?;
        let actor = Actor::from(principal);
        if second && actor.user_id.is_some() && actor.user_id == change.created_by {
            return Err(Error::forbidden(
                "catalog.self_approval",
                "The person who made a price change cannot approve it.",
            ));
        }
        let transaction = tenant.database().begin_write().await?;
        principal
            .require_write_at(&transaction, &PRICES_APPROVE, crate::service::at(list.org_id))
            .await?;
        let mut row = change.into_active_model();
        row.status = Set("approved".to_owned());
        row.approved_by = Set(actor.user_id);
        row.updated_at = Set(now());
        let saved = row.update(&transaction).await?;
        self.jobs
            .enqueue(&transaction, tenant, &ApplyChange { change: id }, EnqueueOptions::default())
            .await?;
        transaction.commit().await?;
        Ok(saved)
    }

    async fn second_person_required(
        &self,
        tenant: &TenantContext,
        org: OrgId,
    ) -> Result<bool, Error> {
        let Some(directory) = &self.organizations else { return Ok(false) };
        let setting = directory.setting(tenant, org, SECOND_PERSON.key()).await?;
        // Giá trị lưu dạng JSON: `true` hay chuỗi `"true"`.
        Ok(setting
            .and_then(|setting| setting.value)
            .is_some_and(|value| value.as_bool().unwrap_or_else(|| value.as_str() == Some("true"))))
    }

    /// Ghi dòng giá của lần đổi đã duyệt, theo lô; dòng đã ghi (cùng `reference`) bỏ qua.
    pub(crate) async fn apply_change(&self, tenant: &TenantContext, id: Uuid) -> Result<(), Error> {
        let read = tenant.database().read();
        let Some(change) = price_change::Entity::find_by_id(id).scoped(tenant).one(read).await?
        else {
            return Ok(());
        };
        if change.status != "approved" {
            return Ok(());
        }
        let Some(list) =
            price_list::Entity::find_by_id(change.price_list_id).scoped(tenant).one(read).await?
        else {
            return Ok(());
        };
        let plan = self.plan(tenant, &change, &list).await?;
        for chunk in plan.chunks(BATCH) {
            let transaction = tenant.database().begin_write().await?;
            let references: Vec<String> =
                chunk.iter().map(|row| format!("bulk:{id}:{}", row.variant_id)).collect();
            let done: Vec<String> = price::Entity::find()
                .filter(price::Column::PriceListId.eq(list.id))
                .filter(price::Column::Reference.is_in(references))
                .scoped(tenant)
                .all(&transaction)
                .await?
                .into_iter()
                .filter_map(|row| row.reference)
                .collect();
            let mut changes = Vec::new();
            for row in chunk {
                let reference = format!("bulk:{id}:{}", row.variant_id);
                // Giá dưới sàn `block` không ghi (bản xem trước đã báo).
                if done.contains(&reference) || row.floor_violation.as_deref() == Some("block") {
                    continue;
                }
                let written = price::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    price_list_id: Set(list.id),
                    variant_id: Set(row.variant_id),
                    amount_minor: Set(row.after),
                    amount_currency: Set(list.currency.clone()),
                    compare_at_minor: Set(None),
                    valid_from: Set(change.valid_from),
                    valid_to: Set(change.valid_to),
                    source: Set("bulk".to_owned()),
                    reference: Set(Some(reference)),
                    reference_fingerprint: Set(None),
                    created_by: Set(change.created_by),
                    created_at: Set(now()),
                }
                .insert(&transaction)
                .await?;
                changes.push(json!({
                    "variant_id": written.variant_id,
                    "price_id": written.id,
                    "amount": written.amount_minor.to_string(),
                    "currency": written.amount_currency,
                    "valid_from": written.valid_from,
                    "valid_to": written.valid_to,
                }));
            }
            if !changes.is_empty() {
                let event = crate::events::PriceChanged(crate::events::Change {
                    before: None,
                    after: Some(
                        json!({"price_list_id": list.id, "org_id": list.org_id, "change_id": id, "prices": changes}),
                    ),
                });
                self.outbox
                    .record(
                        &transaction,
                        tenant,
                        autoxide_kernel::event::Actor::system(),
                        crate::events::price_list_subject(list.id),
                        &event,
                    )
                    .await?;
                self.queue_derivation(&transaction, tenant, list.id, None).await?;
            }
            transaction.commit().await?;
        }
        let write = tenant.database().write();
        let mut row = change.into_active_model();
        row.status = Set("applied".to_owned());
        row.updated_at = Set(now());
        row.update(write).await?;
        Ok(())
    }
}

fn rounding_rule_not_found(id: Uuid) -> Error {
    Error::not_found("catalog.rounding_rule_not_found", "There is no rounding rule `{id}`.")
        .with_param("id", id)
}

fn change_state(status: &str) -> Error {
    Error::conflict(
        "catalog.price_change_state",
        "The price change is `{status}` and cannot do this any more.",
    )
    .with_param("status", status)
}

/// Bộ chạy job [`ApplyChange`].
pub(crate) struct Applier(pub(crate) Arc<CatalogService>);

#[async_trait::async_trait]
impl JobHandler for Applier {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let Some(tenant) = &job.tenant else { return Ok(()) };
        let input: ApplyChange = serde_json::from_value(job.payload.clone())
            .map_err(|error| Error::internal(format!("bad apply_price_change payload: {error}")))?;
        self.0.apply_change(tenant, input.change).await
    }
}

#[cfg(test)]
mod tests {
    use super::apply;

    #[test]
    fn phep_doi_gia() {
        assert_eq!(apply("set", 99_000, None, None), Some(99_000));
        assert_eq!(apply("percent", -1_000, Some(200_000), None), Some(180_000));
        assert_eq!(apply("percent", -1_000, None, None), None);
        assert_eq!(apply("amount", -5_000, Some(3_000), None), Some(0));
    }
}
