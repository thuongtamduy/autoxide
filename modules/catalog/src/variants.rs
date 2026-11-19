//! Tuỳ chọn (màu, cỡ…), biến thể (SKU, mã vạch, cân nặng, chế độ bán) và thành phần combo.

use std::collections::{BTreeMap, HashMap};

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use chrono::{DateTime, NaiveDate, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, PaginatorTrait,
    QueryFilter, QueryOrder, Set,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{BundleComponent, ProductKind, SaleMode};
use crate::entity::{
    bundle_component, option, option_value, product, product_category, product_media, translation,
    variant,
};
use crate::errors;
use crate::events::{self, Change};
use crate::permissions::{PRODUCTS_READ, PRODUCTS_WRITE};
use crate::products::{ProductDetail, parse_kind};
use crate::service::{
    BUNDLES_FEATURE, CatalogService, VARIANTS_FEATURE, check_length, fail_if_any, now, readable,
    texts_json, texts_of, unique_violation,
};

/// Số tuỳ chọn tối đa của một sản phẩm và số giá trị tối đa của một tuỳ chọn.
const MAX_OPTIONS: usize = 3;
const MAX_VALUES: usize = 100;
/// Số thành phần tối đa của một combo.
const MAX_COMPONENTS: usize = 50;

/// Biến thể mới (hay biến thể đầu tiên của sản phẩm mới).
#[derive(Debug, Clone, Default)]
pub(crate) struct NewVariant {
    pub sku: String,
    pub barcode: Option<String>,
    /// Một giá trị cho mỗi tuỳ chọn của sản phẩm (rỗng khi sản phẩm không có tuỳ chọn).
    pub option_value_ids: Vec<Uuid>,
    pub active: Option<bool>,
    pub sale_mode: Option<SaleMode>,
    pub preorder_available_on: Option<NaiveDate>,
    pub preorder_limit: Option<i64>,
    pub preorder_ends_at: Option<DateTime<Utc>>,
    pub preorder_messages: BTreeMap<String, String>,
    pub weight_grams: Option<i64>,
    pub length_mm: Option<i64>,
    pub width_mm: Option<i64>,
    pub height_mm: Option<i64>,
    pub image_media_id: Option<Uuid>,
}

/// Biến thể đã kiểm và chuẩn hoá.
#[derive(Debug, Clone)]
pub(crate) struct VariantChecked {
    pub sku: String,
    pub sku_key: String,
    pub barcode: Option<String>,
    pub option_value_ids: Vec<Uuid>,
    pub active: bool,
    pub sale_mode: &'static str,
    pub preorder_available_on: Option<NaiveDate>,
    pub preorder_limit: Option<i64>,
    pub preorder_ends_at: Option<DateTime<Utc>>,
    pub preorder_messages: Option<String>,
    pub weight_grams: Option<i64>,
    pub length_mm: Option<i64>,
    pub width_mm: Option<i64>,
    pub height_mm: Option<i64>,
    pub image_media_id: Option<Uuid>,
}

/// Một tuỳ chọn và các giá trị của nó.
#[derive(Debug, Clone)]
pub(crate) struct OptionDetail {
    pub option: option::Model,
    pub values: Vec<option_value::Model>,
}

/// Tuỳ chọn gửi lên: `id` của tuỳ chọn / giá trị đã có thì giữ (biến thể vẫn trỏ được), thiếu
/// `id` là thêm mới.
#[derive(Debug, Clone)]
pub(crate) struct OptionInput {
    pub id: Option<Uuid>,
    pub names: BTreeMap<String, String>,
    pub values: Vec<OptionValueInput>,
}

#[derive(Debug, Clone)]
pub(crate) struct OptionValueInput {
    pub id: Option<Uuid>,
    pub labels: BTreeMap<String, String>,
}

pub(crate) fn sale_mode_str(mode: SaleMode) -> &'static str {
    match mode {
        SaleMode::InStock => "in_stock",
        SaleMode::Backorder => "backorder",
        SaleMode::Preorder => "preorder",
    }
}

pub(crate) fn parse_sale_mode(raw: &str) -> SaleMode {
    match raw {
        "backorder" => SaleMode::Backorder,
        "preorder" => SaleMode::Preorder,
        _ => SaleMode::InStock,
    }
}

/// SKU dùng để so (duy nhất trong khách): chữ hoa, bỏ mọi khoảng trắng.
pub(crate) fn sku_key(sku: &str) -> String {
    sku.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_uppercase()
}

/// Mã vạch toàn số dài 8 / 12 / 13 / 14 (GTIN) phải đúng chữ số kiểm tra; mã khác không kiểm.
pub(crate) fn gtin_valid(code: &str) -> bool {
    if !code.bytes().all(|b| b.is_ascii_digit()) || ![8, 12, 13, 14].contains(&code.len()) {
        return true;
    }
    let digits: Vec<u32> = code.bytes().map(|b| u32::from(b - b'0')).collect();
    let (body, check) = digits.split_at(digits.len() - 1);
    let sum: u32 = body
        .iter()
        .rev()
        .enumerate()
        .map(|(index, digit)| if index % 2 == 0 { digit * 3 } else { *digit })
        .sum();
    (10 - sum % 10) % 10 == check[0]
}

fn check_measure(problems: &mut Vec<FieldError>, field: &str, value: Option<i64>) {
    if value.is_some_and(|value| !(0..=10_000_000).contains(&value)) {
        problems.push(FieldError::new(field, "range", "must be 0 to 10000000"));
    }
}

/// Kiểm biến thể và gom mọi lỗi (D-083). `kind`: hàng số và dịch vụ không có cân nặng hay đặt
/// trước.
pub(crate) fn check_variant(
    problems: &mut Vec<FieldError>,
    input: &NewVariant,
    kind: ProductKind,
) -> VariantChecked {
    let sku = input.sku.trim().to_owned();
    check_length(problems, "sku", &sku, 1, 64);
    if sku.chars().any(char::is_control) {
        problems.push(FieldError::new("sku", "format", "must not contain control characters"));
    }
    let barcode = input.barcode.as_deref().map(str::trim).filter(|code| !code.is_empty());
    if let Some(code) = barcode {
        if code.len() > 32 || !code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            problems.push(FieldError::new(
                "barcode",
                "format",
                "must be up to 32 letters, digits or `-`",
            ));
        } else if !gtin_valid(code) {
            problems.push(FieldError::new("barcode", "check_digit", "has a wrong check digit"));
        }
    }
    for (field, value) in [
        ("weight_grams", input.weight_grams),
        ("length_mm", input.length_mm),
        ("width_mm", input.width_mm),
        ("height_mm", input.height_mm),
    ] {
        check_measure(problems, field, value);
    }
    let mode = input.sale_mode.unwrap_or(SaleMode::InStock);
    let shipped = matches!(kind, ProductKind::Physical | ProductKind::Bundle);
    if mode == SaleMode::Preorder {
        // Ngày có hàng phải sau hôm nay (D-204). So theo ngày UTC: lệch múi giờ của nút chỉ làm
        // ranh giới sớm / muộn vài giờ, không cho đặt một ngày đã qua.
        let today = Utc::now().date_naive();
        if input.preorder_available_on.is_none_or(|day| day <= today) {
            problems.push(FieldError::new(
                "preorder_available_on",
                "catalog.preorder_date_invalid",
                "must be a date after today",
            ));
        }
        if input.preorder_limit.is_some_and(|limit| limit <= 0) {
            problems.push(FieldError::new(
                "preorder_limit",
                "catalog.preorder_limit_invalid",
                "must be greater than 0",
            ));
        }
    }
    if !shipped && mode != SaleMode::InStock {
        problems.push(FieldError::new(
            "sale_mode",
            "not_shipped",
            "digital products and services are always `in_stock`",
        ));
    }
    for (locale, message) in &input.preorder_messages {
        check_length(problems, &format!("preorder_messages.{locale}"), message, 0, 500);
    }
    let preorder = mode == SaleMode::Preorder;
    VariantChecked {
        sku_key: sku_key(&sku),
        sku,
        barcode: barcode.map(str::to_owned),
        option_value_ids: input.option_value_ids.clone(),
        active: input.active.unwrap_or(true),
        sale_mode: sale_mode_str(mode),
        preorder_available_on: input.preorder_available_on.filter(|_| preorder),
        preorder_limit: input.preorder_limit.filter(|_| preorder),
        preorder_ends_at: input.preorder_ends_at.filter(|_| preorder).map(crate::service::micros),
        preorder_messages: (preorder && !input.preorder_messages.is_empty())
            .then(|| texts_json(&input.preorder_messages)),
        weight_grams: input.weight_grams.filter(|_| shipped),
        length_mm: input.length_mm.filter(|_| shipped),
        width_mm: input.width_mm.filter(|_| shipped),
        height_mm: input.height_mm.filter(|_| shipped),
        image_media_id: input.image_media_id,
    }
}

/// `option_key`: ID giá trị tuỳ chọn sắp tăng, nối `,`.
fn option_key(ids: &[Uuid]) -> String {
    let mut ids: Vec<String> = ids.iter().map(Uuid::to_string).collect();
    ids.sort();
    ids.join(",")
}

/// Vi phạm unique khi ghi biến thể thành lỗi 409 dễ hiểu.
fn variant_conflict(error: sea_orm::DbErr, checked: &VariantChecked) -> Error {
    match unique_violation(&error) {
        Some(message) if message.contains("barcode") => {
            errors::barcode_taken(checked.barcode.as_deref().unwrap_or_default())
        }
        Some(message) if message.contains("option") => errors::option_combination_taken(),
        Some(_) => errors::sku_taken(&checked.sku),
        None => Error::from(error),
    }
}

pub(crate) async fn insert_variant<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    product_id: Uuid,
    checked: &VariantChecked,
    position: i32,
) -> Result<variant::Model, Error> {
    let now = now();
    variant::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant.id().as_uuid()),
        product_id: Set(product_id),
        sku: Set(checked.sku.clone()),
        sku_key: Set(checked.sku_key.clone()),
        barcode: Set(checked.barcode.clone()),
        option_key: Set(option_key(&checked.option_value_ids)),
        position: Set(position),
        active: Set(checked.active),
        sale_mode: Set(checked.sale_mode.to_owned()),
        preorder_available_on: Set(checked.preorder_available_on),
        preorder_limit: Set(checked.preorder_limit),
        preorder_ends_at: Set(checked.preorder_ends_at),
        preorder_messages: Set(checked.preorder_messages.clone()),
        weight_grams: Set(checked.weight_grams),
        length_mm: Set(checked.length_mm),
        width_mm: Set(checked.width_mm),
        height_mm: Set(checked.height_mm),
        image_media_id: Set(checked.image_media_id),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(connection)
    .await
    .map_err(|error| variant_conflict(error, checked))
}

/// Những gì audit (và module nghe event) cần biết về một biến thể.
pub(crate) fn snapshot(row: &variant::Model) -> Value {
    json!({
        "id": row.id,
        "product_id": row.product_id,
        "sku": row.sku,
        "barcode": row.barcode,
        "active": row.active,
        "sale_mode": row.sale_mode,
        "preorder_available_on": row.preorder_available_on,
        "preorder_limit": row.preorder_limit,
        "weight_grams": row.weight_grams,
        "updated_at": row.updated_at,
    })
}

/// Tuỳ chọn của sản phẩm, theo thứ tự.
pub(crate) async fn options_of<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    product_id: Uuid,
) -> Result<Vec<OptionDetail>, Error> {
    let options = option::Entity::find()
        .filter(option::Column::ProductId.eq(product_id))
        .scoped(tenant)
        .order_by_asc(option::Column::Position)
        .all(connection)
        .await?;
    let values = option_value::Entity::find()
        .filter(option_value::Column::ProductId.eq(product_id))
        .scoped(tenant)
        .order_by_asc(option_value::Column::Position)
        .all(connection)
        .await?;
    Ok(options
        .into_iter()
        .map(|option| {
            let values =
                values.iter().filter(|value| value.option_id == option.id).cloned().collect();
            OptionDetail { option, values }
        })
        .collect())
}

/// Mỗi tuỳ chọn có đúng một giá trị trong `ids`.
fn check_combination(options: &[OptionDetail], ids: &[Uuid]) -> Result<(), Error> {
    let complete = ids.len() == options.len()
        && options.iter().all(|option| {
            option.values.iter().filter(|value| ids.contains(&value.id)).count() == 1
        });
    if complete {
        Ok(())
    } else {
        Err(Error::validation(vec![FieldError::new(
            "option_value_ids",
            "combination",
            "must hold exactly one value of each option of the product",
        )]))
    }
}

/// Xoá mọi phần của sản phẩm `product_id` (khi xoá sản phẩm).
pub(crate) async fn delete_product_parts<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    product_id: Uuid,
) -> Result<(), Error> {
    let variant_ids: Vec<Uuid> = variant::Entity::find()
        .filter(variant::Column::ProductId.eq(product_id))
        .scoped(tenant)
        .all(connection)
        .await?
        .into_iter()
        .map(|row| row.id)
        .collect();
    bundle_component::Entity::delete_many()
        .filter(bundle_component::Column::BundleVariantId.is_in(variant_ids.clone()))
        .scoped(tenant)
        .exec(connection)
        .await?;
    variant::Entity::delete_many()
        .filter(variant::Column::ProductId.eq(product_id))
        .scoped(tenant)
        .exec(connection)
        .await?;
    option_value::Entity::delete_many()
        .filter(option_value::Column::ProductId.eq(product_id))
        .scoped(tenant)
        .exec(connection)
        .await?;
    option::Entity::delete_many()
        .filter(option::Column::ProductId.eq(product_id))
        .scoped(tenant)
        .exec(connection)
        .await?;
    translation::Entity::delete_many()
        .filter(translation::Column::ProductId.eq(product_id))
        .scoped(tenant)
        .exec(connection)
        .await?;
    product_category::Entity::delete_many()
        .filter(product_category::Column::ProductId.eq(product_id))
        .scoped(tenant)
        .exec(connection)
        .await?;
    product_media::Entity::delete_many()
        .filter(product_media::Column::ProductId.eq(product_id))
        .scoped(tenant)
        .exec(connection)
        .await?;
    Ok(())
}

/// Thay đổi cho biến thể; trường `None` giữ nguyên, `Some(None)` xoá giá trị.
#[derive(Debug, Clone, Default)]
pub(crate) struct VariantChanges {
    pub sku: Option<String>,
    pub barcode: Option<Option<String>>,
    pub option_value_ids: Option<Vec<Uuid>>,
    pub active: Option<bool>,
    pub sale_mode: Option<SaleMode>,
    pub preorder_available_on: Option<Option<NaiveDate>>,
    pub preorder_limit: Option<Option<i64>>,
    pub preorder_ends_at: Option<Option<DateTime<Utc>>>,
    pub preorder_messages: Option<BTreeMap<String, String>>,
    pub weight_grams: Option<Option<i64>>,
    pub length_mm: Option<Option<i64>>,
    pub width_mm: Option<Option<i64>>,
    pub height_mm: Option<Option<i64>>,
    pub image_media_id: Option<Option<Uuid>>,
    pub position: Option<i32>,
}

impl VariantChanges {
    /// Biến thể sau khi áp thay đổi, để kiểm lại như biến thể mới.
    fn apply(self, row: &variant::Model) -> NewVariant {
        NewVariant {
            sku: self.sku.unwrap_or_else(|| row.sku.clone()),
            barcode: self.barcode.unwrap_or_else(|| row.barcode.clone()),
            option_value_ids: self.option_value_ids.unwrap_or_else(|| {
                row.option_key.split(',').filter_map(|id| Uuid::parse_str(id).ok()).collect()
            }),
            active: Some(self.active.unwrap_or(row.active)),
            sale_mode: Some(self.sale_mode.unwrap_or_else(|| parse_sale_mode(&row.sale_mode))),
            preorder_available_on: self.preorder_available_on.unwrap_or(row.preorder_available_on),
            preorder_limit: self.preorder_limit.unwrap_or(row.preorder_limit),
            preorder_ends_at: self.preorder_ends_at.unwrap_or(row.preorder_ends_at),
            preorder_messages: self.preorder_messages.unwrap_or_else(|| {
                row.preorder_messages.as_deref().map(texts_of).unwrap_or_default()
            }),
            weight_grams: self.weight_grams.unwrap_or(row.weight_grams),
            length_mm: self.length_mm.unwrap_or(row.length_mm),
            width_mm: self.width_mm.unwrap_or(row.width_mm),
            height_mm: self.height_mm.unwrap_or(row.height_mm),
            image_media_id: self.image_media_id.unwrap_or(row.image_media_id),
        }
    }
}

impl CatalogService {
    /// Thay cả tập tuỳ chọn của sản phẩm. Giá trị đang được biến thể dùng mà bị bỏ thì
    /// `catalog.option_value_in_use`.
    pub(crate) async fn put_options(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        options: Vec<OptionInput>,
    ) -> Result<ProductDetail, Error> {
        let mut problems = Vec::new();
        if options.len() > MAX_OPTIONS {
            problems.push(
                FieldError::new("options", "length", "must have at most {max} options")
                    .with_param("max", MAX_OPTIONS),
            );
        }
        for (index, option) in options.iter().enumerate() {
            if option.names.is_empty() {
                problems.push(FieldError::new(
                    format!("options[{index}].names"),
                    "required",
                    "give the option a name in at least one language",
                ));
            }
            if option.values.is_empty() || option.values.len() > MAX_VALUES {
                problems.push(
                    FieldError::new(
                        format!("options[{index}].values"),
                        "length",
                        "must have 1 to {max} values",
                    )
                    .with_param("max", MAX_VALUES),
                );
            }
            for (locale, name) in &option.names {
                self.locale(locale)?;
                check_length(
                    &mut problems,
                    &format!("options[{index}].names.{locale}"),
                    name,
                    1,
                    100,
                );
            }
            for (value_index, value) in option.values.iter().enumerate() {
                for (locale, label) in &value.labels {
                    self.locale(locale)?;
                    let field = format!("options[{index}].values[{value_index}].labels.{locale}");
                    check_length(&mut problems, &field, label, 1, 100);
                }
            }
        }
        fail_if_any(problems)?;
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        let before =
            self.product_for_write(&transaction, tenant, principal, id, &PRODUCTS_WRITE).await?;
        let existing = options_of(&transaction, tenant, id).await?;
        option_value::Entity::delete_many()
            .filter(option_value::Column::ProductId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        option::Entity::delete_many()
            .filter(option::Column::ProductId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        for (position, input) in options.iter().enumerate() {
            // Giữ ID đã có (nếu là của sản phẩm này) để biến thể vẫn trỏ đúng.
            let option_id = input
                .id
                .filter(|id| existing.iter().any(|option| option.option.id == *id))
                .unwrap_or_else(Uuid::now_v7);
            option::ActiveModel {
                id: Set(option_id),
                tenant_id: Set(tenant.id().as_uuid()),
                product_id: Set(id),
                position: Set(i32::try_from(position).unwrap_or(i32::MAX)),
                names: Set(texts_json(&input.names)),
            }
            .insert(&transaction)
            .await?;
            for (value_position, value) in input.values.iter().enumerate() {
                let value_id = value
                    .id
                    .filter(|id| {
                        existing.iter().flat_map(|option| &option.values).any(|v| v.id == *id)
                    })
                    .unwrap_or_else(Uuid::now_v7);
                option_value::ActiveModel {
                    id: Set(value_id),
                    tenant_id: Set(tenant.id().as_uuid()),
                    product_id: Set(id),
                    option_id: Set(option_id),
                    position: Set(i32::try_from(value_position).unwrap_or(i32::MAX)),
                    labels: Set(texts_json(&value.labels)),
                }
                .insert(&transaction)
                .await?;
            }
        }
        // Biến thể giữ giá trị còn tồn tại; tuỳ chọn mới mà biến thể chưa có giá trị thì nhận giá
        // trị đầu của tuỳ chọn đó (như các nền tảng lớn). Giá trị bị bỏ mà biến thể còn dùng thì
        // chặn: người dùng phải đổi biến thể trước.
        let new_options = options_of(&transaction, tenant, id).await?;
        let variants = variant::Entity::find()
            .filter(variant::Column::ProductId.eq(id))
            .scoped(tenant)
            .all(&transaction)
            .await?;
        let all_values: Vec<Uuid> =
            new_options.iter().flat_map(|option| option.values.iter().map(|v| v.id)).collect();
        let old_values: Vec<Uuid> =
            existing.iter().flat_map(|option| option.values.iter().map(|v| v.id)).collect();
        for row in &variants {
            let mut ids: Vec<Uuid> =
                row.option_key.split(',').filter_map(|id| Uuid::parse_str(id).ok()).collect();
            if ids.iter().any(|id| old_values.contains(id) && !all_values.contains(id)) {
                return Err(Error::conflict(
                    "catalog.option_value_in_use",
                    "Variant `{sku}` still uses an option value you removed. Change the variant \
                     first.",
                )
                .with_param("sku", &row.sku));
            }
            ids.retain(|id| all_values.contains(id));
            for option in &new_options {
                let has = option.values.iter().any(|value| ids.contains(&value.id));
                if let (false, Some(first)) = (has, option.values.first()) {
                    ids.push(first.id);
                }
            }
            let key = option_key(&ids);
            if key != row.option_key {
                let mut changed = row.clone().into_active_model();
                changed.option_key = Set(key);
                changed.updated_at = Set(now());
                changed.update(&transaction).await.map_err(|error| {
                    if unique_violation(&error).is_some() {
                        errors::option_combination_taken()
                    } else {
                        Error::from(error)
                    }
                })?;
            }
        }
        let after = self.touch(&transaction, tenant, actor, &before, None).await?;
        transaction.commit().await?;
        self.detail(tenant, after).await
    }

    /// Thêm biến thể cho sản phẩm. Quá một biến thể cần tính năng `catalog.variants`.
    pub(crate) async fn create_variant(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        product_id: Uuid,
        input: NewVariant,
    ) -> Result<variant::Model, Error> {
        self.require_feature(tenant, &VARIANTS_FEATURE).await?;
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        let product = self
            .product_for_write(&transaction, tenant, principal, product_id, &PRODUCTS_WRITE)
            .await?;
        let mut problems = Vec::new();
        let checked = check_variant(&mut problems, &input, parse_kind(&product.kind));
        fail_if_any(problems)?;
        let options = options_of(&transaction, tenant, product_id).await?;
        check_combination(&options, &checked.option_value_ids)?;
        let count = variant::Entity::find()
            .filter(variant::Column::ProductId.eq(product_id))
            .scoped(tenant)
            .count(&transaction)
            .await?;
        let position = i32::try_from(count).unwrap_or(i32::MAX);
        let row = insert_variant(&transaction, tenant, product_id, &checked, position).await?;
        let event = events::VariantCreated(Change { before: None, after: Some(snapshot(&row)) });
        self.outbox
            .record(&transaction, tenant, actor, events::product_subject(product_id), &event)
            .await?;
        self.touch(&transaction, tenant, actor, &product, None).await?;
        transaction.commit().await?;
        Ok(row)
    }

    /// Biến thể `id` cùng sản phẩm của nó, trong transaction ghi, đã kiểm quyền ghi.
    async fn variant_for_write<C: ConnectionTrait>(
        &self,
        transaction: &C,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(variant::Model, product::Model), Error> {
        let row = variant::Entity::find_by_id(id)
            .scoped(tenant)
            .one(transaction)
            .await?
            .ok_or_else(|| errors::variant_not_found(id))?;
        let product = self
            .product_for_write(transaction, tenant, principal, row.product_id, &PRODUCTS_WRITE)
            .await
            .map_err(|error| {
                if error.code() == "catalog.product_not_found" {
                    errors::variant_not_found(id)
                } else {
                    error
                }
            })?;
        Ok((row, product))
    }

    /// Sửa biến thể `id`. Ngày đặt trước đổi thì phát thêm `catalog.preorder_date_changed`.
    pub(crate) async fn update_variant(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        changes: VariantChanges,
        expected: Option<DateTime<Utc>>,
    ) -> Result<variant::Model, Error> {
        let actor = Actor::from(principal);
        let position = changes.position;
        let transaction = tenant.database().begin_write().await?;
        let (before, product) = self.variant_for_write(&transaction, tenant, principal, id).await?;
        let input = changes.apply(&before);
        let mut problems = Vec::new();
        let checked = check_variant(&mut problems, &input, parse_kind(&product.kind));
        fail_if_any(problems)?;
        let options = options_of(&transaction, tenant, product.id).await?;
        check_combination(&options, &checked.option_value_ids)?;
        let mut row = before.clone().into_active_model();
        row.sku = Set(checked.sku.clone());
        row.sku_key = Set(checked.sku_key.clone());
        row.barcode = Set(checked.barcode.clone());
        row.option_key = Set(option_key(&checked.option_value_ids));
        row.active = Set(checked.active);
        row.sale_mode = Set(checked.sale_mode.to_owned());
        row.preorder_available_on = Set(checked.preorder_available_on);
        row.preorder_limit = Set(checked.preorder_limit);
        row.preorder_ends_at = Set(checked.preorder_ends_at);
        row.preorder_messages = Set(checked.preorder_messages.clone());
        row.weight_grams = Set(checked.weight_grams);
        row.length_mm = Set(checked.length_mm);
        row.width_mm = Set(checked.width_mm);
        row.height_mm = Set(checked.height_mm);
        row.image_media_id = Set(checked.image_media_id);
        if let Some(position) = position {
            row.position = Set(position);
        }
        row.updated_at = Set(now());
        let mut update = variant::Entity::update(row).validate()?.scoped(tenant);
        if let Some(expected) = expected {
            update = update.filter(variant::Column::UpdatedAt.eq(expected));
        }
        let after = update.exec(&transaction).await.map_err(|error| match error {
            sea_orm::DbErr::RecordNotUpdated => autoxide_kernel::version::changed(),
            error => variant_conflict(error, &checked),
        })?;
        let subject = events::product_subject(product.id);
        let event = events::VariantUpdated(Change {
            before: Some(snapshot(&before)),
            after: Some(snapshot(&after)),
        });
        self.outbox.record(&transaction, tenant, actor, subject.clone(), &event).await?;
        if before.preorder_available_on != after.preorder_available_on
            && before.sale_mode == "preorder"
        {
            let event = events::PreorderDateChanged(Change {
                before: Some(snapshot(&before)),
                after: Some(snapshot(&after)),
            });
            self.outbox.record(&transaction, tenant, actor, subject, &event).await?;
        }
        self.queue_products(&transaction, tenant, vec![product.id]).await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Xoá biến thể `id`; sản phẩm phải còn ít nhất một biến thể.
    pub(crate) async fn delete_variant(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        let (row, product) = self.variant_for_write(&transaction, tenant, principal, id).await?;
        let count = variant::Entity::find()
            .filter(variant::Column::ProductId.eq(product.id))
            .scoped(tenant)
            .count(&transaction)
            .await?;
        if count <= 1 {
            return Err(errors::variant_required());
        }
        bundle_component::Entity::delete_many()
            .filter(bundle_component::Column::BundleVariantId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        variant::Entity::delete_many()
            .filter(variant::Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        product_media::Entity::update_many()
            .col_expr(
                product_media::Column::VariantId,
                sea_orm::sea_query::Expr::value(None::<Uuid>),
            )
            .filter(product_media::Column::VariantId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let event = events::VariantDeleted(Change { before: Some(snapshot(&row)), after: None });
        self.outbox
            .record(&transaction, tenant, actor, events::product_subject(product.id), &event)
            .await?;
        self.touch(&transaction, tenant, actor, &product, None).await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Biến thể theo SKU hay mã vạch, trong phạm vi đọc của người gọi.
    pub(crate) async fn find_variants_by_code(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        code: &str,
    ) -> Result<Vec<variant::Model>, Error> {
        let read = tenant.database().read();
        let code = code.trim();
        let rows = variant::Entity::find()
            .filter(
                sea_orm::Condition::any()
                    .add(variant::Column::SkuKey.eq(sku_key(code)))
                    .add(variant::Column::Barcode.eq(code)),
            )
            .scoped(tenant)
            .all(read)
            .await?;
        let mut found = Vec::new();
        for row in rows {
            let Some(product) =
                product::Entity::find_by_id(row.product_id).scoped(tenant).one(read).await?
            else {
                continue;
            };
            if readable(principal, read, &PRODUCTS_READ, product.org_id, || {
                errors::variant_not_found(row.id)
            })
            .await
            .is_ok()
            {
                found.push(row);
            }
        }
        Ok(found)
    }

    /// Thay cả tập thành phần của biến thể combo `id`. Combo không chứa combo khác, chính nó hay
    /// dịch vụ (`catalog.bundle_invalid`).
    pub(crate) async fn put_components(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        components: Vec<BundleComponent>,
    ) -> Result<Vec<BundleComponent>, Error> {
        self.require_feature(tenant, &BUNDLES_FEATURE).await?;
        let mut problems = Vec::new();
        if components.is_empty() || components.len() > MAX_COMPONENTS {
            problems.push(
                FieldError::new("components", "length", "must have 1 to {max} components")
                    .with_param("max", MAX_COMPONENTS),
            );
        }
        for (index, component) in components.iter().enumerate() {
            if !(1..=10_000).contains(&component.quantity) {
                problems.push(FieldError::new(
                    format!("components[{index}].quantity"),
                    "range",
                    "must be 1 to 10000",
                ));
            }
        }
        let mut seen = std::collections::HashSet::new();
        if components.iter().any(|component| !seen.insert(component.variant_id)) {
            problems.push(FieldError::new("components", "duplicate", "list each variant once"));
        }
        fail_if_any(problems)?;
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        let (row, product) = self.variant_for_write(&transaction, tenant, principal, id).await?;
        if parse_kind(&product.kind) != ProductKind::Bundle {
            return Err(errors::bundle_invalid());
        }
        let ids: Vec<Uuid> = components.iter().map(|component| component.variant_id).collect();
        if ids.contains(&row.id) {
            return Err(errors::bundle_invalid());
        }
        let parts = variant::Entity::find()
            .filter(variant::Column::Id.is_in(ids.clone()))
            .scoped(tenant)
            .all(&transaction)
            .await?;
        if let Some(missing) = ids.iter().find(|id| !parts.iter().any(|part| part.id == **id)) {
            return Err(errors::variant_not_found(*missing));
        }
        let product_ids: Vec<Uuid> = parts.iter().map(|part| part.product_id).collect();
        let kinds: HashMap<Uuid, String> = product::Entity::find()
            .filter(product::Column::Id.is_in(product_ids))
            .scoped(tenant)
            .all(&transaction)
            .await?
            .into_iter()
            .map(|product| (product.id, product.kind))
            .collect();
        let bad = parts.iter().any(|part| {
            matches!(
                kinds.get(&part.product_id).map(String::as_str),
                Some("bundle" | "service") | None
            )
        });
        if bad {
            return Err(errors::bundle_invalid());
        }
        bundle_component::Entity::delete_many()
            .filter(bundle_component::Column::BundleVariantId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        for component in &components {
            bundle_component::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                bundle_variant_id: Set(id),
                component_variant_id: Set(component.variant_id),
                quantity: Set(component.quantity),
            }
            .insert(&transaction)
            .await?;
        }
        self.touch(&transaction, tenant, actor, &product, None).await?;
        transaction.commit().await?;
        Ok(components)
    }

    /// Thành phần của các combo `variant_ids` (đọc, không kiểm quyền — dùng sau khi đã kiểm).
    pub(crate) async fn components_of(
        &self,
        tenant: &TenantContext,
        variant_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, Vec<BundleComponent>>, Error> {
        let rows = bundle_component::Entity::find()
            .filter(bundle_component::Column::BundleVariantId.is_in(variant_ids.iter().copied()))
            .scoped(tenant)
            .order_by_asc(bundle_component::Column::Id)
            .all(tenant.database().read())
            .await?;
        let mut out: HashMap<Uuid, Vec<BundleComponent>> = HashMap::new();
        for row in rows {
            out.entry(row.bundle_variant_id).or_default().push(BundleComponent {
                variant_id: row.component_variant_id,
                quantity: row.quantity,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::{gtin_valid, sku_key};

    #[test]
    fn kiem_chu_so_gtin() {
        assert!(gtin_valid("8934563138165"));
        assert!(!gtin_valid("8934563138166"));
        assert!(gtin_valid("ABC-123"));
        assert!(gtin_valid("96385074"));
    }

    #[test]
    fn sku_so_khong_phan_biet_hoa_thuong_va_khoang_trang() {
        assert_eq!(sku_key(" ao-thun m "), "AO-THUNM");
    }
}
