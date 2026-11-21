//! Sản phẩm: tạo (kèm biến thể đầu tiên), đọc, sửa, xoá / lưu trữ, chuyển trạng thái, nội dung
//! theo ngôn ngữ, danh mục và ảnh của sản phẩm.

use autoxide_kernel::time::Timestamp;
use std::collections::HashMap;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::search;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, IntoActiveModel,
    PaginatorTrait, QueryFilter, QueryOrder, Set,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{ProductKind, ProductStatus};
use crate::entity::{product, product_category, product_media, translation, variant};
use crate::errors;
use crate::events::{self, Change};
use crate::permissions::{PRODUCTS_PUBLISH, PRODUCTS_READ, PRODUCTS_WRITE};
use crate::service::{
    CatalogService, PRODUCTS_LIMIT, check_length, default_org, fail_if_any, now, readable,
    unique_violation, writable,
};
use crate::variants::{NewVariant, VariantChecked};

/// Số ký tự tối đa của tên sản phẩm, mô tả ngắn, mô tả.
pub(crate) const NAME_MAX: usize = 200;
pub(crate) const SHORT_DESCRIPTION_MAX: usize = 1_000;
pub(crate) const DESCRIPTION_MAX: usize = 50_000;

/// Nội dung theo một ngôn ngữ.
#[derive(Debug, Clone, Default)]
pub(crate) struct TextInput {
    pub name: String,
    /// Trống: sinh từ tên (`autoxide_kernel::slug`).
    pub slug: Option<String>,
    pub short_description: String,
    /// Chữ thuần, xuống dòng là đoạn; giao diện hiển thị Markdown tối giản (không lưu HTML).
    pub description: String,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    /// Slug sinh từ tên (người dùng không gửi): trùng thì thêm hậu tố `-2`, `-3`… thay vì báo lỗi.
    pub slug_generated: bool,
}

/// Sản phẩm mới, kèm biến thể đầu tiên.
#[derive(Debug, Clone)]
pub(crate) struct NewProduct {
    pub org_id: Option<Uuid>,
    pub kind: ProductKind,
    pub tax_category: String,
    pub locale: String,
    pub text: TextInput,
    pub warranty_months: Option<i32>,
    pub lot_tracked: bool,
    pub serial_tracked: bool,
    pub category_ids: Vec<Uuid>,
    pub variant: NewVariant,
}

/// Thay đổi cho sản phẩm; trường `None` giữ nguyên.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProductChanges {
    pub tax_category: Option<String>,
    /// `Some(None)` bỏ bảo hành.
    pub warranty_months: Option<Option<i32>>,
    pub lot_tracked: Option<bool>,
    pub serial_tracked: Option<bool>,
    pub default_locale: Option<String>,
}

/// Bộ lọc danh sách quản trị.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProductFilter {
    pub status: Option<ProductStatus>,
    pub kind: Option<ProductKind>,
    pub org_id: Option<Uuid>,
    pub category_id: Option<Uuid>,
    pub readiness: Option<String>,
    /// Tìm trong tên, SKU, mô tả ngắn (có dấu hay không đều được).
    pub q: Option<String>,
}

/// Một sản phẩm đầy đủ.
#[derive(Debug, Clone)]
pub(crate) struct ProductDetail {
    pub product: product::Model,
    pub translations: Vec<translation::Model>,
    pub options: Vec<crate::variants::OptionDetail>,
    pub variants: Vec<variant::Model>,
    pub category_ids: Vec<Uuid>,
    pub media: Vec<product_media::Model>,
    /// `alt` theo ngôn ngữ của ảnh (D-202).
    pub media_alts: Vec<crate::entity::media_alt::Model>,
    /// Đoạn đầu của đường dẫn công khai (`AUTOXIDE_CATALOG_PRODUCT_PATH_PREFIX`).
    pub path_prefix: String,
}

/// Một dòng của danh sách: sản phẩm và nội dung bằng ngôn ngữ mặc định của nó.
#[derive(Debug, Clone)]
pub(crate) struct ProductSummary {
    pub product: product::Model,
    pub text: Option<translation::Model>,
    pub variant_count: u64,
}

pub(crate) fn kind_str(kind: ProductKind) -> &'static str {
    match kind {
        ProductKind::Physical => "physical",
        ProductKind::Digital => "digital",
        ProductKind::Service => "service",
        ProductKind::Bundle => "bundle",
    }
}

pub(crate) fn parse_kind(raw: &str) -> ProductKind {
    match raw {
        "digital" => ProductKind::Digital,
        "service" => ProductKind::Service,
        "bundle" => ProductKind::Bundle,
        _ => ProductKind::Physical,
    }
}

pub(crate) fn status_str(status: ProductStatus) -> &'static str {
    match status {
        ProductStatus::Draft => "draft",
        ProductStatus::Active => "active",
        ProductStatus::Inactive => "inactive",
        ProductStatus::Archived => "archived",
    }
}

pub(crate) fn parse_status(raw: &str) -> ProductStatus {
    match raw {
        "active" => ProductStatus::Active,
        "inactive" => ProductStatus::Inactive,
        "archived" => ProductStatus::Archived,
        _ => ProductStatus::Draft,
    }
}

/// Chuyển trạng thái được phép: nháp → đang bán / lưu trữ; đang bán ↔ ngừng bán; mọi trạng
/// thái → lưu trữ; lưu trữ → nháp (mở lại để sửa rồi bán lại).
fn transition_allowed(from: ProductStatus, to: ProductStatus) -> bool {
    use ProductStatus::{Active, Archived, Draft, Inactive};
    matches!(
        (from, to),
        (Draft, Active)
            | (Draft, Archived)
            | (Active, Inactive)
            | (Active, Archived)
            | (Inactive, Active)
            | (Inactive, Archived)
            | (Archived, Draft)
    )
}

/// Hàng vật lý cần giao và có tồn kho; hàng số, dịch vụ không; combo không tự có tồn (trừ thành
/// phần).
pub(crate) fn tracks_inventory(kind: ProductKind) -> bool {
    kind == ProductKind::Physical
}

/// Mã nhóm thuế dạng `vat.standard`: chữ thường, số, `.`, `_`, 1–64 ký tự.
pub(crate) fn tax_category_valid(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 64
        && code
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'_')
}

fn check_tax_category(problems: &mut Vec<FieldError>, code: &str) {
    if !tax_category_valid(code) {
        problems.push(FieldError::new(
            "tax_category",
            "format",
            "must be a tax category code such as `vat.standard`",
        ));
    }
}

fn check_warranty(problems: &mut Vec<FieldError>, months: Option<i32>) {
    if months.is_some_and(|months| !(1..=600).contains(&months)) {
        problems.push(FieldError::new("warranty_months", "range", "must be 1 to 600 months"));
    }
}

/// Kiểm nội dung theo ngôn ngữ, trả nội dung đã chuẩn hoá (slug sinh khi trống).
pub(crate) fn check_text(
    problems: &mut Vec<FieldError>,
    text: &TextInput,
    product_id: Uuid,
) -> TextInput {
    let name = text.name.trim().to_owned();
    check_length(problems, "name", &name, 1, NAME_MAX);
    check_length(problems, "short_description", &text.short_description, 0, SHORT_DESCRIPTION_MAX);
    check_length(problems, "description", &text.description, 0, DESCRIPTION_MAX);
    if let Some(title) = &text.meta_title {
        check_length(problems, "meta_title", title, 0, 200);
    }
    if let Some(description) = &text.meta_description {
        check_length(problems, "meta_description", description, 0, 500);
    }
    let given = text.slug.as_deref().map(str::trim).filter(|slug| !slug.is_empty());
    let slug = match given {
        Some(slug) => {
            if !autoxide_kernel::slug::is_valid(slug) {
                problems.push(FieldError::new(
                    "slug",
                    "format",
                    "must be lower-case letters, digits and single `-` between them",
                ));
            }
            slug.to_owned()
        }
        None => autoxide_kernel::slug::from_title(&name, product_id),
    };
    TextInput {
        name,
        slug: Some(slug),
        short_description: text.short_description.clone(),
        description: text.description.clone(),
        meta_title: text.meta_title.clone().filter(|value| !value.trim().is_empty()),
        meta_description: text.meta_description.clone().filter(|value| !value.trim().is_empty()),
        slug_generated: given.is_none(),
    }
}

/// Slug còn trống trong ngôn ngữ `locale` của khách: `base`, hay `base-2`, `base-3`… (bỏ qua
/// chính sản phẩm `product_id`).
pub(crate) async fn free_slug<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    locale: &str,
    base: &str,
    product_id: Uuid,
) -> Result<String, Error> {
    let taken: Vec<String> = translation::Entity::find()
        .filter(translation::Column::Locale.eq(locale))
        .filter(translation::Column::ProductId.ne(product_id))
        .filter(Condition::any().add(translation::Column::Slug.eq(base)).add(
            autoxide_kernel::search::starts_with(translation::Column::Slug, &format!("{base}-")),
        ))
        .scoped(tenant)
        .all(connection)
        .await?
        .into_iter()
        .map(|row| row.slug)
        .collect();
    if !taken.iter().any(|slug| slug == base) {
        return Ok(base.to_owned());
    }
    let mut n = 2;
    loop {
        let next = autoxide_kernel::slug::with_suffix(base, n);
        if !taken.contains(&next) {
            return Ok(next);
        }
        n += 1;
    }
}

/// Dòng nội dung theo ngôn ngữ, kèm khoá tìm (`search_key`, `name_key` — D-201).
pub(crate) fn translation_row(
    tenant: &TenantContext,
    product_id: Uuid,
    locale: &str,
    text: &TextInput,
    skus: &[String],
) -> translation::ActiveModel {
    let haystack = format!("{} {} {}", text.name, skus.join(" "), text.short_description);
    translation::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant.id().as_uuid()),
        product_id: Set(product_id),
        locale: Set(locale.to_owned()),
        name: Set(text.name.clone()),
        slug: Set(text.slug.clone().unwrap_or_default()),
        short_description: Set(text.short_description.clone()),
        description: Set(text.description.clone()),
        meta_title: Set(text.meta_title.clone()),
        meta_description: Set(text.meta_description.clone()),
        search_key: Set(search::fold(&haystack)),
        name_key: Set(search::exact_key(&text.name).chars().take(NAME_MAX).collect()),
        updated_at: Set(now()),
    }
}

pub(crate) fn slug_conflict(error: sea_orm::DbErr, slug: &str) -> Error {
    match unique_violation(&error) {
        Some(_) => errors::slug_taken(slug),
        None => Error::from(error),
    }
}

/// Những gì audit cần biết về một sản phẩm.
pub(crate) fn snapshot(product: &product::Model) -> Value {
    json!({
        "id": product.id,
        "org_id": product.org_id,
        "kind": product.kind,
        "status": product.status,
        "tax_category": product.tax_category,
        "warranty_months": product.warranty_months,
        "readiness": product.readiness,
        "default_locale": product.default_locale,
        "updated_at": product.updated_at,
    })
}

fn reference(id: Uuid) -> String {
    format!("catalog.product:{id}")
}

impl CatalogService {
    /// Tạo sản phẩm (trạng thái nháp) kèm biến thể đầu tiên. Giữ chỗ hạn mức `catalog.products`
    /// trước transaction, xác nhận khi xong, trả lại khi lỗi.
    pub(crate) async fn create_product(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewProduct,
    ) -> Result<ProductDetail, Error> {
        let id = Uuid::now_v7();
        let locale = self.locale(&input.locale)?;
        let mut problems = Vec::new();
        check_tax_category(&mut problems, &input.tax_category);
        check_warranty(&mut problems, input.warranty_months);
        let text = check_text(&mut problems, &input.text, id);
        let variant = crate::variants::check_variant(&mut problems, &input.variant, input.kind);
        fail_if_any(problems)?;
        let org = match input.org_id {
            Some(org) => OrgId::from_uuid(org),
            None => default_org(tenant, principal, &PRODUCTS_WRITE)?,
        };
        let key = reference(id);
        self.entitlements.reserve(tenant.id(), &PRODUCTS_LIMIT, 1, &key).await?;
        let created =
            self.insert_product(tenant, principal, id, org, &locale, &input, &text, variant).await;
        match &created {
            Ok(_) => self.entitlements.confirm(tenant.id(), &PRODUCTS_LIMIT, &key).await?,
            Err(_) => self.entitlements.release(tenant.id(), &PRODUCTS_LIMIT, &key).await?,
        }
        created
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert_product(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        org: OrgId,
        locale: &str,
        input: &NewProduct,
        text: &TextInput,
        variant: VariantChecked,
    ) -> Result<ProductDetail, Error> {
        let actor = Actor::from(principal);
        let now = now();
        let transaction = tenant.database().begin_write().await?;
        principal
            .require_write_at(&transaction, &PRODUCTS_WRITE, crate::service::at(org.as_uuid()))
            .await?;
        let categories =
            self.existing_categories(&transaction, tenant, &input.category_ids).await?;
        let row = product::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(org.as_uuid()),
            kind: Set(kind_str(input.kind).to_owned()),
            status: Set("draft".to_owned()),
            tax_category: Set(input.tax_category.clone()),
            primary_category_id: Set(categories.first().copied()),
            track_inventory: Set(tracks_inventory(input.kind)),
            allow_backorder: Set(variant.sale_mode == "backorder"),
            lot_tracked: Set(input.lot_tracked),
            serial_tracked: Set(input.serial_tracked),
            warranty_months: Set(input.warranty_months),
            readiness: Set("ready".to_owned()),
            default_locale: Set(locale.to_owned()),
            ai_assisted: Set(false),
            created_by: Set(actor.user_id),
            created_at: Set(now),
            updated_at: Set(now),
            activated_at: Set(None),
            ever_active: Set(false),
        }
        .insert(&transaction)
        .await?;
        let mut text = text.clone();
        if text.slug_generated {
            let base = text.slug.clone().unwrap_or_default();
            text.slug = Some(free_slug(&transaction, tenant, locale, &base, id).await?);
        }
        let slug = text.slug.clone().unwrap_or_default();
        translation_row(tenant, id, locale, &text, std::slice::from_ref(&variant.sku))
            .insert(&transaction)
            .await
            .map_err(|error| slug_conflict(error, &slug))?;
        let first = crate::variants::insert_variant(&transaction, tenant, id, &variant, 0).await?;
        for category in &categories {
            product_category::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                product_id: Set(id),
                category_id: Set(*category),
            }
            .insert(&transaction)
            .await?;
        }
        self.refresh_readiness(&transaction, tenant, &[id]).await?;
        let event = events::ProductCreated(Change { before: None, after: Some(snapshot(&row)) });
        self.outbox
            .record(&transaction, tenant, actor, events::product_subject(id), &event)
            .await?;
        let event = events::VariantCreated(Change {
            before: None,
            after: Some(crate::variants::snapshot(&first)),
        });
        self.outbox
            .record(&transaction, tenant, actor, events::product_subject(id), &event)
            .await?;
        self.queue_products(&transaction, tenant, vec![id]).await?;
        transaction.commit().await?;
        tracing::info!(product_id = %id, "catalog product created");
        self.get_row(tenant, id).await
    }

    /// Một trang sản phẩm trong phạm vi `catalog.products.read`, mới tạo trước (mặc định).
    pub(crate) async fn list_products(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: &ProductFilter,
        page: &PageRequest,
    ) -> Result<Page<ProductSummary>, Error> {
        let read = tenant.database().read();
        let mut condition = Condition::all().add(principal.scope_condition(
            &PRODUCTS_READ,
            product::Column::OrgId,
            None,
        )?);
        if let Some(status) = filter.status {
            condition = condition.add(product::Column::Status.eq(status_str(status)));
        }
        if let Some(kind) = filter.kind {
            condition = condition.add(product::Column::Kind.eq(kind_str(kind)));
        }
        if let Some(org) = filter.org_id {
            condition = condition.add(product::Column::OrgId.eq(org));
        }
        if let Some(readiness) = &filter.readiness {
            condition = condition.add(product::Column::Readiness.eq(readiness.as_str()));
        }
        if let Some(category) = filter.category_id {
            // Danh mục và mọi danh mục con của nó.
            let below = self.descendants(read, tenant, category).await?;
            let ids: Vec<Uuid> = product_category::Entity::find()
                .filter(product_category::Column::CategoryId.is_in(below))
                .scoped(tenant)
                .all(read)
                .await?
                .into_iter()
                .map(|row| row.product_id)
                .collect();
            condition = condition.add(product::Column::Id.is_in(ids));
        }
        if let Some(q) = filter.q.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
            let ids: Vec<Uuid> = translation::Entity::find()
                .filter(search::contains(translation::Column::SearchKey, q))
                .scoped(tenant)
                .all(read)
                .await?
                .into_iter()
                .map(|row| row.product_id)
                .collect();
            condition = condition.add(product::Column::Id.is_in(ids));
        }
        let select = product::Entity::find().filter(condition).scoped(tenant);
        let page = fetch_page(select, product::Column::Id, |row| row.id, page, read).await?;
        let ids: Vec<Uuid> = page.data.iter().map(|row| row.id).collect();
        let texts = translation::Entity::find()
            .filter(translation::Column::ProductId.is_in(ids.clone()))
            .scoped(tenant)
            .all(read)
            .await?;
        let variants = variant::Entity::find()
            .filter(variant::Column::ProductId.is_in(ids))
            .scoped(tenant)
            .all(read)
            .await?;
        let mut counts: HashMap<Uuid, u64> = HashMap::new();
        for row in &variants {
            *counts.entry(row.product_id).or_default() += 1;
        }
        Ok(page.map(|product| {
            let text = texts
                .iter()
                .find(|text| text.product_id == product.id && text.locale == product.default_locale)
                .or_else(|| texts.iter().find(|text| text.product_id == product.id))
                .cloned();
            let variant_count = counts.get(&product.id).copied().unwrap_or_default();
            ProductSummary { product, text, variant_count }
        }))
    }

    /// Sản phẩm `id` đầy đủ. Ngoài phạm vi đọc báo như không có.
    pub(crate) async fn get_product(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<ProductDetail, Error> {
        let read = tenant.database().read();
        let row = product::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| errors::product_not_found(id))?;
        readable(principal, read, &PRODUCTS_READ, row.org_id, || errors::product_not_found(id))
            .await?;
        self.detail(tenant, row).await
    }

    /// Sản phẩm `id` đầy đủ, không kiểm quyền (dùng ngay sau khi chính service đã ghi).
    pub(crate) async fn get_row(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<ProductDetail, Error> {
        let row = product::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().write())
            .await?
            .ok_or_else(|| errors::product_not_found(id))?;
        self.detail(tenant, row).await
    }

    /// Đọc đủ phần của sản phẩm `row` (nội dung, tuỳ chọn, biến thể, danh mục, ảnh).
    pub(crate) async fn detail(
        &self,
        tenant: &TenantContext,
        row: product::Model,
    ) -> Result<ProductDetail, Error> {
        let read = tenant.database().read();
        let id = row.id;
        let translations = translation::Entity::find()
            .filter(translation::Column::ProductId.eq(id))
            .scoped(tenant)
            .order_by_asc(translation::Column::Locale)
            .all(read)
            .await?;
        let variants = variant::Entity::find()
            .filter(variant::Column::ProductId.eq(id))
            .scoped(tenant)
            .order_by_asc(variant::Column::Position)
            .order_by_asc(variant::Column::Id)
            .all(read)
            .await?;
        let category_ids = product_category::Entity::find()
            .filter(product_category::Column::ProductId.eq(id))
            .scoped(tenant)
            .order_by_asc(product_category::Column::Id)
            .all(read)
            .await?
            .into_iter()
            .map(|row| row.category_id)
            .collect();
        let media = product_media::Entity::find()
            .filter(product_media::Column::ProductId.eq(id))
            .scoped(tenant)
            .order_by_asc(product_media::Column::Position)
            .all(read)
            .await?;
        let options = crate::variants::options_of(read, tenant, id).await?;
        let media_alts = crate::entity::media_alt::Entity::find()
            .filter(crate::entity::media_alt::Column::ProductId.eq(id))
            .scoped(tenant)
            .order_by_asc(crate::entity::media_alt::Column::MediaId)
            .order_by_asc(crate::entity::media_alt::Column::Locale)
            .all(read)
            .await?;
        Ok(ProductDetail {
            product: row,
            translations,
            options,
            variants,
            category_ids,
            media,
            media_alts,
            path_prefix: self.config.product_path_prefix.clone(),
        })
    }

    /// Đọc sản phẩm `id` trong transaction ghi và kiểm quyền ghi `permission` của người gọi.
    pub(crate) async fn product_for_write<C: ConnectionTrait>(
        &self,
        transaction: &C,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        permission: &autoxide_kernel::access::Permission,
    ) -> Result<product::Model, Error> {
        // Khoá dòng sản phẩm tới hết transaction ghi (D-181): mọi lệnh sửa sản phẩm, biến thể, giấy
        // tờ đi qua đây nên nối đuôi nhau trên Postgres — mở bán (kiểm giấy tờ bắt buộc) không chen
        // giữa lúc giấy tờ đang bị xoá, hai lần sửa không cùng xuất phát từ bản cũ rồi phát event
        // trùng. Ngoài transaction (kết nối ghi trần) khoá nhả ngay sau câu lệnh, không hại gì.
        let row = autoxide_kernel::rows::lock_one(
            transaction,
            product::Entity::find_by_id(id).scoped(tenant),
        )
        .await?
        .ok_or_else(|| errors::product_not_found(id))?;
        writable(principal, transaction, &PRODUCTS_READ, permission, row.org_id, || {
            errors::product_not_found(id)
        })
        .await?;
        Ok(row)
    }

    /// Ghi `updated_at` mới (phiên bản) của sản phẩm và event `catalog.product_updated`.
    pub(crate) async fn touch<C: ConnectionTrait>(
        &self,
        transaction: &C,
        tenant: &TenantContext,
        actor: Actor,
        before: &product::Model,
        expected: Option<Timestamp>,
    ) -> Result<product::Model, Error> {
        let mut row = before.clone().into_active_model();
        row.updated_at = Set(now());
        self.save_product(transaction, tenant, actor, before, row, expected).await
    }

    pub(crate) async fn save_product<C: ConnectionTrait>(
        &self,
        transaction: &C,
        tenant: &TenantContext,
        actor: Actor,
        before: &product::Model,
        mut row: product::ActiveModel,
        expected: Option<Timestamp>,
    ) -> Result<product::Model, Error> {
        row.updated_at = Set(now());
        let mut update = product::Entity::update(row).validate()?.scoped(tenant);
        if let Some(expected) = expected {
            // Kiểm phiên bản ngay trong câu `UPDATE`: người khác vừa sửa thì 0 dòng (D-084).
            update = update.filter(product::Column::UpdatedAt.eq(expected));
        }
        let after = update.exec(transaction).await.map_err(|error| match error {
            sea_orm::DbErr::RecordNotUpdated => autoxide_kernel::version::changed(),
            error => Error::from(error),
        })?;
        let event = events::ProductUpdated(Change {
            before: Some(snapshot(before)),
            after: Some(snapshot(&after)),
        });
        self.outbox
            .record(transaction, tenant, actor, events::product_subject(after.id), &event)
            .await?;
        self.queue_products(transaction, tenant, vec![after.id]).await?;
        Ok(after)
    }

    /// Sửa thông tin chung của sản phẩm.
    pub(crate) async fn update_product(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        changes: ProductChanges,
        expected: Option<Timestamp>,
    ) -> Result<ProductDetail, Error> {
        let mut problems = Vec::new();
        if let Some(code) = &changes.tax_category {
            check_tax_category(&mut problems, code);
        }
        if let Some(months) = changes.warranty_months {
            check_warranty(&mut problems, months);
        }
        let default_locale = match &changes.default_locale {
            Some(locale) => Some(self.locale(locale)?),
            None => None,
        };
        fail_if_any(problems)?;
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        let before =
            self.product_for_write(&transaction, tenant, principal, id, &PRODUCTS_WRITE).await?;
        let mut row = before.clone().into_active_model();
        if let Some(code) = changes.tax_category {
            row.tax_category = Set(code);
        }
        if let Some(months) = changes.warranty_months {
            row.warranty_months = Set(months);
        }
        if let Some(value) = changes.lot_tracked {
            row.lot_tracked = Set(value);
        }
        if let Some(value) = changes.serial_tracked {
            row.serial_tracked = Set(value);
        }
        if let Some(locale) = default_locale {
            // Ngôn ngữ mặc định phải đã có nội dung.
            let has = translation::Entity::find()
                .filter(translation::Column::ProductId.eq(id))
                .filter(translation::Column::Locale.eq(locale.as_str()))
                .scoped(tenant)
                .count(&transaction)
                .await?;
            if has == 0 {
                return Err(Error::validation(vec![FieldError::new(
                    "default_locale",
                    "missing_translation",
                    "add the texts in this language first",
                )]));
            }
            row.default_locale = Set(locale);
        }
        let after = self.save_product(&transaction, tenant, actor, &before, row, expected).await?;
        transaction.commit().await?;
        self.detail(tenant, after).await
    }

    /// Xoá sản phẩm chưa từng bán (kèm mọi phần của nó); đã từng bán thì
    /// `catalog.product_not_deletable` — hãy lưu trữ. Trả lại hạn mức.
    pub(crate) async fn delete_product(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        let row =
            self.product_for_write(&transaction, tenant, principal, id, &PRODUCTS_WRITE).await?;
        if row.ever_active {
            return Err(errors::product_not_deletable());
        }
        crate::variants::delete_product_parts(&transaction, tenant, id).await?;
        let deleted = product::Entity::delete_many()
            .filter(product::Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if deleted.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(errors::product_not_found(id));
        }
        let event = events::ProductDeleted(Change { before: Some(snapshot(&row)), after: None });
        self.outbox
            .record(&transaction, tenant, actor, events::product_subject(id), &event)
            .await?;
        self.queue_products(&transaction, tenant, vec![id]).await?;
        transaction.commit().await?;
        self.entitlements.release(tenant.id(), &PRODUCTS_LIMIT, &reference(id)).await?;
        Ok(())
    }

    /// Chuyển trạng thái bán của sản phẩm. Mở bán kiểm: có nhóm thuế, có nội dung, có biến thể
    /// đang bật, `readiness = ready`.
    /// Hỏi điều kiện mở bán đã cắm (nếu có); từ chối → 422 với mã của module cắm vào.
    async fn check_listing_gate(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        let Some(gate) = self.listing_gate.get() else { return Ok(()) };
        let detail = self.get_product(tenant, principal, id).await?;
        let candidate = crate::api::ListingCandidate::new(
            id,
            autoxide_kernel::tenancy::OrgId::from_uuid(detail.product.org_id),
            detail.category_ids.clone(),
            parse_kind(&detail.product.kind),
        );
        match gate.can_publish(tenant, &candidate).await? {
            crate::api::ListingDecision::Allow => Ok(()),
            crate::api::ListingDecision::Deny { code, message } => {
                Err(errors::listing_denied(&code, &message))
            }
        }
    }

    pub(crate) async fn set_status(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        to: ProductStatus,
        expected: Option<Timestamp>,
    ) -> Result<ProductDetail, Error> {
        let actor = Actor::from(principal);
        if to == ProductStatus::Active {
            // Điều kiện mở bán của module khác (bước 46) — có thể đọc database của nó, nên hỏi
            // TRƯỚC transaction ghi. Đọc qua kiểm quyền đọc để người ngoài phạm vi không biết lý do.
            self.check_listing_gate(tenant, principal, id).await?;
        }
        let transaction = tenant.database().begin_write().await?;
        let before =
            self.product_for_write(&transaction, tenant, principal, id, &PRODUCTS_PUBLISH).await?;
        let from = parse_status(&before.status);
        if from == to {
            transaction.rollback().await?;
            return self.detail(tenant, before).await;
        }
        if !transition_allowed(from, to) {
            return Err(errors::invalid_status_transition(status_str(from), status_str(to)));
        }
        if to == ProductStatus::Active {
            self.check_sellable(&transaction, tenant, &before).await?;
        }
        let mut row = before.clone().into_active_model();
        row.status = Set(status_str(to).to_owned());
        if to == ProductStatus::Active {
            row.ever_active = Set(true);
            if before.activated_at.is_none() {
                row.activated_at = Set(Some(now()));
            }
        }
        row.updated_at = Set(now());
        let mut update = product::Entity::update(row).validate()?.scoped(tenant);
        if let Some(expected) = expected {
            update = update.filter(product::Column::UpdatedAt.eq(expected));
        }
        let after = update.exec(&transaction).await.map_err(|error| match error {
            sea_orm::DbErr::RecordNotUpdated => autoxide_kernel::version::changed(),
            error => Error::from(error),
        })?;
        let change = || Change { before: Some(snapshot(&before)), after: Some(snapshot(&after)) };
        let subject = events::product_subject(id);
        let event = events::ProductStatusChanged(change());
        self.outbox.record(&transaction, tenant, actor, subject.clone(), &event).await?;
        if to == ProductStatus::Archived {
            let event = events::ProductArchived(change());
            self.outbox.record(&transaction, tenant, actor, subject, &event).await?;
        }
        self.queue_products(&transaction, tenant, vec![id]).await?;
        transaction.commit().await?;
        tracing::info!(product_id = %id, status = status_str(to), "catalog product status changed");
        self.detail(tenant, after).await
    }

    /// Điều kiện mở bán; lỗi đầu tiên gặp được trả về.
    async fn check_sellable<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        product: &product::Model,
    ) -> Result<(), Error> {
        let mut problems = Vec::new();
        if product.tax_category.is_empty() {
            problems.push(FieldError::new("tax_category", "required", "set a tax category"));
        }
        let texts = translation::Entity::find()
            .filter(translation::Column::ProductId.eq(product.id))
            .scoped(tenant)
            .count(connection)
            .await?;
        if texts == 0 {
            problems.push(FieldError::new("translations", "required", "add a name"));
        }
        let active = variant::Entity::find()
            .filter(variant::Column::ProductId.eq(product.id))
            .filter(variant::Column::Active.eq(true))
            .scoped(tenant)
            .count(connection)
            .await?;
        if active == 0 {
            problems.push(FieldError::new("variants", "required", "turn on at least one variant"));
        }
        fail_if_any(problems)?;
        if parse_kind(&product.kind) == ProductKind::Digital {
            crate::digital::require_assets(connection, tenant, product.id).await?;
        }
        self.check_tax_category_known(tenant, product).await?;
        // Tính lại tại chỗ (không tin cột dẫn xuất): báo đúng thuộc tính / giấy tờ còn thiếu.
        self.readiness_of(connection, tenant, product).await?.require()
    }

    /// Nhóm thuế của sản phẩm phải có trong engine thuế (bước 36) ở vùng thuế của nút bán. Chưa
    /// cài `tax` hay nút chưa có hồ sơ thuế thì không kiểm được — bỏ qua (D-20261118-step41-readiness).
    async fn check_tax_category_known(
        &self,
        tenant: &TenantContext,
        product: &product::Model,
    ) -> Result<(), Error> {
        use autoxide_tax::api::{PriceMode, TaxQuoteRequest, TaxableLine};
        let Some(tax) = &self.tax else { return Ok(()) };
        let org = OrgId::from_uuid(product.org_id);
        let Some(profile) = tax.profile(tenant, org).await? else { return Ok(()) };
        let currency = autoxide_kernel::money::Currency::VND;
        let line = TaxableLine::new(
            "catalog-check",
            product.tax_category.clone(),
            autoxide_kernel::money::Money::zero(currency),
        );
        let request = TaxQuoteRequest::new(
            profile.jurisdiction,
            Timestamp::now().date_utc(),
            org,
            PriceMode::Inclusive,
            currency,
            vec![line],
        );
        match tax.quote(tenant, request).await {
            Err(error) if error.code() == "tax.category_unknown" => Err(Error::new(
                autoxide_kernel::error::ErrorKind::Validation,
                "catalog.tax_category_unknown",
                "The tax category `{code}` is not in the tax engine.",
            )
            .with_param("code", &product.tax_category)),
            // Thiếu thuế suất hay lỗi khác của engine không chặn việc mở bán ở đây: chứng từ sẽ
            // báo khi tính thuế thật.
            _ => Ok(()),
        }
    }
}
