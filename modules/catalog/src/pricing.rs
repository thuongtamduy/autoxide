//! Bảng giá và dòng giá (D-222).
//!
//! - **Dòng giá chỉ thêm**: không sửa, không xoá. Đổi giá = thêm dòng với `valid_from` mới; giá
//!   hiệu lực lúc `t` = dòng có `valid_from` lớn nhất ≤ `t` mà `valid_to` NULL hay > `t`. Lịch sử
//!   giá là các dòng.
//! - **Thứ tự chọn bảng giá** (một cách duy nhất, tất định): bảng hợp lệ là bảng `active`, cùng
//!   tiền tệ, ở cửa hàng hay một tổ tiên của nó, mỗi cột lọc (`company_id`, `customer_group`,
//!   `channel`, `market`) NULL hay khớp. Xếp: nút gần cửa hàng nhất → khớp `company_id` → khớp
//!   `customer_group` → khớp `channel` → khớp `market` → `priority` lớn hơn → `id`. Mỗi biến thể
//!   lấy giá từ bảng đầu tiên trong thứ tự đó có giá cho nó.
//! - **Đọc giá không tính công thức**: một câu lấy mọi dòng ứng viên của các biến thể rồi chọn
//!   trong Rust.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::reference::{self, Reference, Submitted};
use autoxide_kernel::tenancy::{OrgId, OrgTree, ScopeExt, TenantContext, root_of};
use autoxide_kernel::time::Timestamp;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, IntoActiveModel,
    PaginatorTrait, QueryFilter, QueryOrder, Set,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{FloorAction, MAX_IDS, PriceQuery, PriceSnapshot};
use crate::entity::{price, price_list, variant};
use crate::errors;
use crate::events::{self, Change};
use crate::permissions::{PRICES_READ, PRICES_WRITE};
use crate::service::{
    CatalogService, PRICE_LISTS_FEATURE, check_length, default_org, fail_if_any, micros, now,
    readable, writable,
};

/// Số dòng giá tối đa mỗi lần thêm.
pub(crate) const MAX_PRICES: usize = 500;

/// Bảng giá mới.
#[derive(Debug, Clone)]
pub(crate) struct NewPriceList {
    pub org_id: Option<Uuid>,
    pub name: String,
    pub currency: Currency,
    pub tax_inclusive: bool,
    /// `consumer` hay `business`.
    pub audience: String,
    pub customer_group: Option<String>,
    pub priority: i32,
    /// Bảng gốc của bảng suy ra (D-222); bảng gốc không được là bảng suy ra.
    pub derived_from: Option<Uuid>,
    /// Điều chỉnh theo phần vạn, có dấu (−1500 = −15%).
    pub adjust_bp: i64,
    /// Cộng thêm sau khi nhân, đơn vị nhỏ nhất, có dấu.
    pub adjust_minor: Option<i64>,
    pub rounding_rule_id: Option<Uuid>,
}

/// Thay đổi cho bảng giá; trường `None` giữ nguyên.
#[derive(Debug, Clone, Default)]
pub(crate) struct PriceListChanges {
    pub name: Option<String>,
    pub active: Option<bool>,
    pub priority: Option<i32>,
    pub customer_group: Option<Option<String>>,
}

/// Một dòng giá gửi lên.
#[derive(Debug, Clone)]
pub(crate) struct PriceInput {
    pub variant_id: Uuid,
    pub amount: Money,
    pub compare_at: Option<Money>,
    /// Trống: bây giờ.
    pub valid_from: Option<Timestamp>,
    /// Giá tạm thời: hết hạn thì giá cũ tự hiệu lực lại.
    pub valid_to: Option<Timestamp>,
}

/// Kết quả của một lần thêm giá: các dòng đã ghi và cảnh báo sàn giá (`warn`).
#[derive(Debug, Clone)]
pub(crate) struct AddedPrices {
    pub prices: Vec<price::Model>,
    pub warnings: Vec<PriceWarning>,
}

/// Giá dưới sàn của quy tắc `warn`: vẫn ghi, kèm cảnh báo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PriceWarning {
    pub variant_id: Uuid,
    pub floor: Money,
}

/// Giá giải ra cho một biến thể, kèm lý do (bảng nào thắng) — cho agent kiểm.
#[derive(Debug, Clone)]
pub(crate) struct ResolvedPrice {
    pub price: Option<PriceSnapshot>,
    /// Bảng hợp lệ theo thứ tự chọn, bảng đầu có giá thắng.
    pub candidates: Vec<price_list::Model>,
}

pub(crate) fn snapshot_of(list: &price_list::Model, row: &price::Model) -> Option<PriceSnapshot> {
    let amount = Money::from_columns(row.amount_minor, &row.amount_currency).ok()?;
    let compare_at = row
        .compare_at_minor
        .and_then(|minor| Money::from_columns(minor, &row.amount_currency).ok());
    Some(PriceSnapshot {
        variant_id: row.variant_id,
        price_list_id: list.id,
        price_id: row.id,
        amount,
        tax_inclusive: list.tax_inclusive,
        compare_at,
        valid_from: row.valid_from,
        valid_to: row.valid_to,
    })
}

fn list_snapshot(list: &price_list::Model) -> Value {
    json!({
        "id": list.id,
        "org_id": list.org_id,
        "name": list.name,
        "currency": list.currency,
        "tax_inclusive": list.tax_inclusive,
        "audience": list.audience,
        "customer_group": list.customer_group,
        "priority": list.priority,
        "active": list.active,
    })
}

fn check_group(problems: &mut Vec<FieldError>, group: Option<&str>) {
    if let Some(group) = group {
        let valid = !group.is_empty()
            && group.len() <= 64
            && group
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.');
        if !valid {
            problems.push(FieldError::new(
                "customer_group",
                "format",
                "must be 1 to 64 letters, digits, `_`, `-` or `.`",
            ));
        }
    }
}

/// Chuỗi nút từ cửa hàng `org` lên gốc (gồm chính nó, gần nhất trước). Nút chưa có trong cây
/// (khách chưa dựng cây): chính nó rồi gốc.
pub(crate) async fn ancestors<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    org: OrgId,
) -> Result<Vec<OrgId>, Error> {
    let mut chain = OrgTree::ancestors(connection, tenant.id(), org).await?;
    if chain.is_empty() {
        chain.push(org);
    }
    let root = root_of(tenant.id());
    if !chain.contains(&root) {
        chain.push(root);
    }
    Ok(chain)
}

/// Bảng giá hợp lệ cho `query`, theo thứ tự chọn (D-222).
async fn candidate_lists<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    query: &PriceQuery,
) -> Result<Vec<price_list::Model>, Error> {
    let chain = ancestors(connection, tenant, query.org_id).await?;
    let lists = price_list::Entity::find()
        .filter(price_list::Column::Active.eq(true))
        .filter(price_list::Column::Currency.eq(query.currency.code()))
        .filter(price_list::Column::OrgId.is_in(chain.iter().map(OrgId::as_uuid)))
        .scoped(tenant)
        .all(connection)
        .await?;
    let matches = |column: &Option<String>, wanted: &Option<String>| match column {
        None => Some(false),
        Some(value) => (wanted.as_ref() == Some(value)).then_some(true),
    };
    let mut ranked: Vec<(usize, [bool; 4], i32, Uuid, price_list::Model)> = Vec::new();
    for list in lists {
        let company = match list.company_id {
            None => Some(false),
            Some(id) => (query.company_id == Some(id)).then_some(true),
        };
        let filters = [
            company,
            matches(&list.customer_group, &query.customer_group),
            matches(&list.channel, &query.channel),
            matches(&list.market, &query.market),
        ];
        // Cột lọc đặt mà không khớp: bảng không áp cho câu hỏi này.
        if filters.iter().any(Option::is_none) {
            continue;
        }
        let rank = chain.iter().position(|org| org.as_uuid() == list.org_id).unwrap_or(usize::MAX);
        let matched = filters.map(|value| value.unwrap_or(false));
        ranked.push((rank, matched, list.priority, list.id, list));
    }
    ranked.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| b.2.cmp(&a.2))
            .then_with(|| a.3.cmp(&b.3))
    });
    Ok(ranked.into_iter().map(|entry| entry.4).collect())
}

/// Dòng hiệu lực lúc `at`: `valid_from` lớn nhất (≤ `at`, hay < `at` khi `before`) mà chưa hết
/// hạn. Hai dòng cùng `valid_from`: dòng thêm sau (ID UUIDv7 lớn hơn) thắng.
fn effective<'a>(
    rows: impl Iterator<Item = &'a price::Model>,
    at: Timestamp,
    before: bool,
) -> Option<&'a price::Model> {
    rows.filter(|row| if before { row.valid_from < at } else { row.valid_from <= at })
        .filter(|row| match row.valid_to {
            None => true,
            // Ngay trước `at`: dòng hết hạn đúng lúc `at` vẫn còn hiệu lực.
            Some(to) => {
                if before {
                    to >= at
                } else {
                    to > at
                }
            }
        })
        .max_by(|a, b| a.valid_from.cmp(&b.valid_from).then_with(|| a.id.cmp(&b.id)))
}

impl CatalogService {
    /// Giá của `query` (hay ngay trước `query.at` khi `before`). Một câu cho bảng, một câu cho
    /// dòng giá, không phụ thuộc số biến thể.
    pub(crate) async fn resolve_prices(
        &self,
        tenant: &TenantContext,
        query: &PriceQuery,
        before: bool,
    ) -> Result<Vec<PriceSnapshot>, Error> {
        if query.variant_ids.len() > MAX_IDS {
            return Err(errors::too_many_ids(MAX_IDS));
        }
        if query.variant_ids.is_empty() {
            return Ok(Vec::new());
        }
        let read = tenant.database().read();
        let lists = candidate_lists(read, tenant, query).await?;
        let rows = self.price_rows(read, tenant, &lists, &query.variant_ids, query.at).await?;
        let mut out = Vec::new();
        for variant_id in &query.variant_ids {
            for list in &lists {
                let found = effective(
                    rows.iter().filter(|row| {
                        row.price_list_id == list.id && row.variant_id == *variant_id
                    }),
                    micros(query.at),
                    before,
                );
                if let Some(row) = found {
                    if let Some(snapshot) = snapshot_of(list, row) {
                        out.push(snapshot);
                    }
                    break;
                }
            }
        }
        Ok(out)
    }

    async fn price_rows<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        lists: &[price_list::Model],
        variant_ids: &[Uuid],
        at: Timestamp,
    ) -> Result<Vec<price::Model>, Error> {
        if lists.is_empty() {
            return Ok(Vec::new());
        }
        let at = micros(at);
        Ok(price::Entity::find()
            .filter(price::Column::PriceListId.is_in(lists.iter().map(|list| list.id)))
            .filter(price::Column::VariantId.is_in(variant_ids.iter().copied()))
            .filter(price::Column::ValidFrom.lte(at))
            .filter(
                Condition::any()
                    .add(price::Column::ValidTo.is_null())
                    .add(price::Column::ValidTo.gte(at)),
            )
            .scoped(tenant)
            .all(connection)
            .await?)
    }

    /// Giá của biến thể `variant_id` cho `query` kèm các bảng ứng viên (đã kiểm quyền đọc giá ở
    /// cửa hàng).
    pub(crate) async fn explain_price(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        query: &PriceQuery,
    ) -> Result<ResolvedPrice, Error> {
        let read = tenant.database().read();
        principal
            .require_at(read, &PRICES_READ, crate::service::at(query.org_id.as_uuid()))
            .await?;
        let candidates = candidate_lists(read, tenant, query).await?;
        let price = self.resolve_prices(tenant, query, false).await?.into_iter().next();
        Ok(ResolvedPrice { price, candidates })
    }

    /// Tạo bảng giá. Bảng thứ hai trở đi của khách cần tính năng `catalog.price_lists`. Bảng
    /// bán cho người tiêu dùng ở nút có hồ sơ thuế Việt Nam phải bằng VND và gồm thuế (Luật Giá
    /// 16/2023 — cần đối chiếu).
    pub(crate) async fn create_price_list(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewPriceList,
    ) -> Result<price_list::Model, Error> {
        let mut problems = Vec::new();
        let name = input.name.trim().to_owned();
        check_length(&mut problems, "name", &name, 1, 200);
        check_group(&mut problems, input.customer_group.as_deref());
        if !matches!(input.audience.as_str(), "consumer" | "business") {
            problems.push(FieldError::new(
                "audience",
                "one_of",
                "must be `consumer` or `business`",
            ));
        }
        fail_if_any(problems)?;
        let org = match input.org_id {
            Some(org) => OrgId::from_uuid(org),
            None => default_org(tenant, principal, &PRICES_WRITE)?,
        };
        if input.audience == "consumer" {
            self.check_consumer_rules(tenant, org, input.currency, input.tax_inclusive).await?;
        }
        let existing =
            price_list::Entity::find().scoped(tenant).count(tenant.database().read()).await?;
        if existing >= 1 {
            self.require_feature(tenant, &PRICE_LISTS_FEATURE).await?;
        }
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        principal
            .require_write_at(&transaction, &PRICES_WRITE, crate::service::at(org.as_uuid()))
            .await?;
        if let Some(source) = input.derived_from {
            let source = price_list::Entity::find_by_id(source)
                .scoped(tenant)
                .one(&transaction)
                .await?
                .ok_or_else(|| errors::price_list_not_found(source))?;
            if source.derived_from.is_some() {
                return Err(Error::new(
                    autoxide_kernel::error::ErrorKind::Validation,
                    "catalog.derived_chain",
                    "The source price list of a derived price list cannot itself be derived.",
                ));
            }
            if source.currency != input.currency.code() {
                return Err(errors::currency_mismatch());
            }
        }
        if let Some(rule) = input.rounding_rule_id {
            crate::entity::rounding_rule::Entity::find_by_id(rule)
                .scoped(tenant)
                .one(&transaction)
                .await?
                .ok_or_else(|| {
                    Error::not_found(
                        "catalog.rounding_rule_not_found",
                        "There is no rounding rule `{id}`.",
                    )
                    .with_param("id", rule)
                })?;
        }
        let now = now();
        let row = price_list::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(org.as_uuid()),
            name: Set(name),
            currency: Set(input.currency.code().to_owned()),
            tax_inclusive: Set(input.tax_inclusive),
            audience: Set(input.audience),
            company_id: Set(None),
            customer_group: Set(input.customer_group),
            channel: Set(None),
            market: Set(None),
            derived_from: Set(input.derived_from),
            adjust_bp: Set(input.adjust_bp),
            adjust_minor: Set(input.adjust_minor),
            rounding_rule_id: Set(input.rounding_rule_id),
            priority: Set(input.priority),
            active: Set(true),
            created_by: Set(actor.user_id),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&transaction)
        .await?;
        let event =
            events::PriceListChanged(Change { before: None, after: Some(list_snapshot(&row)) });
        self.outbox
            .record(&transaction, tenant, actor, events::price_list_subject(row.id), &event)
            .await?;
        if let Some(source) = row.derived_from {
            // Bảng suy ra mới: ghi giá từ mọi dòng hiện có của bảng gốc.
            self.queue_derivation(&transaction, tenant, source, Some(row.id)).await?;
        }
        self.queue_all(&transaction, tenant).await?;
        transaction.commit().await?;
        Ok(row)
    }

    /// Luật "VND + gồm thuế" chỉ áp khi nút có hồ sơ thuế Việt Nam (engine thuế đã cài).
    async fn check_consumer_rules(
        &self,
        tenant: &TenantContext,
        org: OrgId,
        currency: Currency,
        tax_inclusive: bool,
    ) -> Result<(), Error> {
        let Some(tax) = &self.tax else {
            return Ok(());
        };
        let vietnam =
            tax.profile(tenant, org).await?.is_some_and(|profile| profile.jurisdiction == "VN");
        if vietnam && (currency.code() != "VND" || !tax_inclusive) {
            return Err(errors::consumer_price_rules());
        }
        Ok(())
    }

    /// Một trang bảng giá trong phạm vi `catalog.prices.read`.
    pub(crate) async fn list_price_lists(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<price_list::Model>, Error> {
        let scope = principal.scope_condition(&PRICES_READ, price_list::Column::OrgId, None)?;
        let select = price_list::Entity::find().filter(scope).scoped(tenant);
        fetch_page(select, price_list::Column::Id, |row| row.id, page, tenant.database().read())
            .await
    }

    /// Bảng giá `id` trong phạm vi đọc.
    pub(crate) async fn get_price_list(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<price_list::Model, Error> {
        let read = tenant.database().read();
        let row = price_list::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| errors::price_list_not_found(id))?;
        readable(principal, read, &PRICES_READ, row.org_id, || errors::price_list_not_found(id))
            .await?;
        Ok(row)
    }

    async fn price_list_for_write<C: ConnectionTrait>(
        &self,
        transaction: &C,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<price_list::Model, Error> {
        let row = price_list::Entity::find_by_id(id)
            .scoped(tenant)
            .one(transaction)
            .await?
            .ok_or_else(|| errors::price_list_not_found(id))?;
        writable(principal, transaction, &PRICES_READ, &PRICES_WRITE, row.org_id, || {
            errors::price_list_not_found(id)
        })
        .await?;
        Ok(row)
    }

    /// Sửa tên, bật / tắt, ưu tiên hay nhóm khách của bảng giá.
    pub(crate) async fn update_price_list(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        changes: PriceListChanges,
        expected: Option<Timestamp>,
    ) -> Result<price_list::Model, Error> {
        let mut problems = Vec::new();
        let name = changes.name.map(|name| name.trim().to_owned());
        if let Some(name) = &name {
            check_length(&mut problems, "name", name, 1, 200);
        }
        if let Some(group) = &changes.customer_group {
            check_group(&mut problems, group.as_deref());
        }
        fail_if_any(problems)?;
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        let before = self.price_list_for_write(&transaction, tenant, principal, id).await?;
        let mut row = before.clone().into_active_model();
        if let Some(name) = name {
            row.name = Set(name);
        }
        if let Some(active) = changes.active {
            row.active = Set(active);
        }
        if let Some(priority) = changes.priority {
            row.priority = Set(priority);
        }
        if let Some(group) = changes.customer_group {
            row.customer_group = Set(group);
        }
        row.updated_at = Set(now());
        let mut update = price_list::Entity::update(row).validate()?.scoped(tenant);
        if let Some(expected) = expected {
            update = update.filter(price_list::Column::UpdatedAt.eq(expected));
        }
        let after = update.exec(&transaction).await.map_err(|error| match error {
            sea_orm::DbErr::RecordNotUpdated => autoxide_kernel::version::changed(),
            error => Error::from(error),
        })?;
        let event = events::PriceListChanged(Change {
            before: Some(list_snapshot(&before)),
            after: Some(list_snapshot(&after)),
        });
        self.outbox
            .record(&transaction, tenant, actor, events::price_list_subject(id), &event)
            .await?;
        self.queue_all(&transaction, tenant).await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Thêm tối đa [`MAX_PRICES`] dòng giá vào bảng giá `id`, đúng một lần cho mỗi `reference`
    /// (gửi lại cùng nội dung trả lại các dòng đã ghi). Giá dưới sàn: quy tắc `block` → 422
    /// `catalog.price_below_floor`; `warn` → ghi, kèm cảnh báo.
    pub(crate) async fn add_prices(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        reference: Reference,
        items: Vec<PriceInput>,
    ) -> Result<AddedPrices, Error> {
        let mut problems = Vec::new();
        if items.is_empty() || items.len() > MAX_PRICES {
            problems.push(
                FieldError::new("prices", "length", "must have 1 to {max} prices")
                    .with_param("max", MAX_PRICES),
            );
        }
        // Dấu vân tay tính trên nội dung client gửi (trước khi điền `valid_from` = bây giờ):
        // gửi lại đúng request thì khớp.
        let fingerprint = reference::fingerprint(&items_fingerprint(&items))?;
        let now = now();
        let items: Vec<PriceInput> = items
            .into_iter()
            .map(|item| PriceInput {
                valid_from: Some(micros(item.valid_from.unwrap_or(now))),
                valid_to: item.valid_to.map(micros),
                ..item
            })
            .collect();
        for (index, item) in items.iter().enumerate() {
            if item.amount.is_negative() {
                problems.push(FieldError::new(
                    format!("prices[{index}].amount"),
                    "range",
                    "must not be negative",
                ));
            }
            if let Some(compare_at) = item.compare_at
                && compare_at.compare(item.amount).is_ok_and(|order| order.is_le())
            {
                problems.push(FieldError::new(
                    format!("prices[{index}].compare_at"),
                    "range",
                    "must be higher than the price",
                ));
            }
            if let (Some(from), Some(to)) = (item.valid_from, item.valid_to)
                && to <= from
            {
                problems.push(FieldError::new(
                    format!("prices[{index}].valid_to"),
                    "range",
                    "must be after `valid_from`",
                ));
            }
        }
        fail_if_any(problems)?;
        let list = self.get_price_list(tenant, principal, id).await?;
        if items.iter().any(|item| {
            item.amount.currency().code() != list.currency
                || item.compare_at.is_some_and(|money| money.currency().code() != list.currency)
        }) {
            return Err(errors::currency_mismatch());
        }
        // Sàn giá đọc TRƯỚC transaction ghi.
        let currency = Currency::parse(&list.currency)?;
        let variant_ids: Vec<Uuid> = items.iter().map(|item| item.variant_id).collect();
        let floors =
            self.floors(tenant, OrgId::from_uuid(list.org_id), currency, &variant_ids).await?;
        let mut warnings = Vec::new();
        for item in &items {
            let Some(rule) = floors.iter().find(|floor| floor.variant_id == item.variant_id) else {
                continue;
            };
            let Some(floor) = rule.floor else { continue };
            if item.amount.minor() < floor.minor() {
                match rule.action {
                    FloorAction::Block => {
                        return Err(errors::price_below_floor(item.variant_id, floor.minor()));
                    }
                    FloorAction::Warn => {
                        warnings.push(PriceWarning { variant_id: item.variant_id, floor });
                    }
                }
            }
        }
        let actor = Actor::from(principal);
        let (database, items, reference, fingerprint) =
            (tenant.database(), &items, &reference, &fingerprint);
        let submitted = reference::submit_once(database, move || async move {
            let transaction = database.begin_write().await?;
            let list = self.price_list_for_write(&transaction, tenant, principal, id).await?;
            let first = format!("{}#0", reference.as_str());
            let stored = price::Entity::find()
                .filter(price::Column::PriceListId.eq(id))
                .filter(price::Column::Reference.eq(first))
                .scoped(tenant)
                .one(&transaction)
                .await?
                .map(|row| (row.clone(), row.reference_fingerprint.clone().unwrap_or_default()));
            if reference::replayed(stored, reference, fingerprint)?.is_some() {
                let prefix = format!("{}#", reference.as_str());
                let rows = price::Entity::find()
                    .filter(price::Column::PriceListId.eq(id))
                    .filter(price::Column::Reference.starts_with(prefix))
                    .scoped(tenant)
                    .order_by_asc(price::Column::Id)
                    .all(&transaction)
                    .await?;
                return Ok(Submitted::Replayed(rows));
            }
            let found: Vec<Uuid> = variant::Entity::find()
                .filter(variant::Column::Id.is_in(items.iter().map(|item| item.variant_id)))
                .scoped(tenant)
                .all(&transaction)
                .await?
                .into_iter()
                .map(|row| row.id)
                .collect();
            if let Some(missing) = items.iter().find(|item| !found.contains(&item.variant_id)) {
                return Err(errors::variant_not_found(missing.variant_id));
            }
            let mut rows = Vec::new();
            let mut changes = Vec::new();
            for (index, item) in items.iter().enumerate() {
                let row = price::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    price_list_id: Set(id),
                    variant_id: Set(item.variant_id),
                    amount_minor: Set(item.amount.minor()),
                    amount_currency: Set(list.currency.clone()),
                    compare_at_minor: Set(item.compare_at.map(Money::minor)),
                    valid_from: Set(item.valid_from.unwrap_or(now)),
                    valid_to: Set(item.valid_to),
                    source: Set("manual".to_owned()),
                    reference: Set(Some(format!("{}#{index}", reference.as_str()))),
                    reference_fingerprint: Set(Some(fingerprint.clone())),
                    created_by: Set(actor.user_id),
                    created_at: Set(now),
                }
                .insert(&transaction)
                .await?;
                changes.push(json!({
                    "variant_id": row.variant_id,
                    "price_id": row.id,
                    "amount": row.amount_minor.to_string(),
                    "currency": row.amount_currency,
                    "valid_from": row.valid_from,
                    "valid_to": row.valid_to,
                }));
                rows.push(row);
            }
            let event = events::PriceChanged(Change {
                before: None,
                after: Some(json!({"price_list_id": id, "org_id": list.org_id, "prices": changes})),
            });
            self.outbox
                .record(&transaction, tenant, actor, events::price_list_subject(id), &event)
                .await?;
            // Bảng suy ra từ bảng này nhận dòng giá mới qua job, cùng transaction.
            self.queue_derivation(&transaction, tenant, id, None).await?;
            let variants = rows.iter().map(|row| row.variant_id).collect();
            self.queue_variants(&transaction, tenant, variants).await?;
            transaction.commit().await?;
            Ok(Submitted::Created(rows))
        })
        .await?;
        let prices = match submitted {
            Submitted::Created(rows) | Submitted::Replayed(rows) => rows,
        };
        Ok(AddedPrices { prices, warnings })
    }

    /// Lịch sử giá của bảng `id` (lọc theo biến thể nếu nêu), mới thêm trước.
    pub(crate) async fn price_history(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        variant_id: Option<Uuid>,
        page: &PageRequest,
    ) -> Result<Page<price::Model>, Error> {
        self.get_price_list(tenant, principal, id).await?;
        let mut select = price::Entity::find().filter(price::Column::PriceListId.eq(id));
        if let Some(variant_id) = variant_id {
            select = select.filter(price::Column::VariantId.eq(variant_id));
        }
        fetch_page(
            select.scoped(tenant),
            price::Column::Id,
            |row| row.id,
            page,
            tenant.database().read(),
        )
        .await
    }
}

/// Nội dung đem lấy dấu vân tay của một lần thêm giá.
fn items_fingerprint(items: &[PriceInput]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|item| {
                json!({
                    "variant_id": item.variant_id,
                    "amount": item.amount.minor(),
                    "currency": item.amount.currency().code(),
                    "compare_at": item.compare_at.map(Money::minor),
                    "valid_from": item.valid_from,
                    "valid_to": item.valid_to,
                })
            })
            .collect(),
    )
}
