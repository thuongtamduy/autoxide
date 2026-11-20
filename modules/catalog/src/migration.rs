//! Migration của module `catalog`. Đã commit thì không sửa — thêm migration mới vào cuối [`all`].
//!
//! Mọi bảng là cấp khách hàng: có `tenant_id`, unique và index bắt đầu bằng `tenant_id` (bước 33).
//! Kiểu cột dùng được trên cả SQLite và Postgres: tiền là `*_minor` (`big_integer`) + mã tiền tệ,
//! JSON lưu trong `text`, không `jsonb`, mảng hay enum của Postgres.

use autoxide_kernel::migration::prelude::*;

mod collection_ai;
mod collections;
mod media_alts;
mod net_content;
mod recommend;
mod search;
mod suggestions;
mod transfers;
mod variant_attributes;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(CreateCatalog),
        // Bảng `catalog_events` (transactional outbox) và `catalog_jobs`, định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "catalog"),
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "catalog"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0004_tenancy_events",
            vec![autoxide_kernel::event::tenant_table("catalog")],
        ),
        autoxide_kernel::event::outbox_access_migration("k0001_event_access", "catalog"),
        autoxide_kernel::job::jobs_migration("m0005_create_jobs", "catalog"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0006_tenancy_jobs",
            vec![autoxide_kernel::job::tenant_table("catalog")],
        ),
        Box::new(CreateAttributes),
        Box::new(CreatePriceChanges),
        Box::new(CreateStorefront),
        Box::new(search::CreateSearch),
        // Bộ sưu tập sản phẩm (bước 55).
        Box::new(collections::CreateCollections),
        Box::new(net_content::AddNetContent),
        // Bán chạy, điểm đánh giá, thường mua cùng, sản phẩm tương tự.
        Box::new(recommend::CreateRecommendations),
        // Dấu `ai_assisted` của bộ sưu tập (bước 55).
        Box::new(collection_ai::AddCollectionAiAssisted),
        // Nhập / xuất CSV bằng job.
        Box::new(transfers::CreateTransfers),
        // Bản nháp AI cho mô tả và SEO.
        Box::new(suggestions::CreateSuggestions),
        // `alt` của ảnh sản phẩm theo ngôn ngữ (D-202).
        Box::new(media_alts::CreateMediaAlts),
        // Thuộc tính theo biến thể cho trang sản phẩm.
        Box::new(variant_attributes::CreateVariantAttributes),
    ]
}

async fn index(
    manager: &SchemaManager<'_>,
    table: &str,
    name: &str,
    columns: &[&str],
    unique: bool,
) -> Result<(), DbErr> {
    let mut index = Index::create();
    index.name(name).table(Alias::new(table));
    if unique {
        index.unique();
    }
    for column in columns {
        index.col(Alias::new(*column));
    }
    manager.create_index(index.to_owned()).await
}

/// Bảng có khoá chính `id` (UUIDv7 sinh trong ứng dụng) và `tenant_id`.
fn table(name: &str) -> TableCreateStatement {
    Table::create()
        .table(Alias::new(name))
        .col(schema::pk_uuid("id"))
        .col(schema::uuid("tenant_id"))
        .to_owned()
}

struct CreateCatalog;

impl MigrationName for CreateCatalog {
    fn name(&self) -> &str {
        "m0001_create_catalog"
    }
}

/// Bảng của [`CreateCatalog`], theo thứ tự tạo (xoá theo thứ tự ngược).
const TABLES: [&str; 17] = [
    "catalog_products",
    "catalog_product_translations",
    "catalog_options",
    "catalog_option_values",
    "catalog_variants",
    "catalog_product_media",
    "catalog_categories",
    "catalog_category_paths",
    "catalog_category_translations",
    "catalog_product_categories",
    "catalog_bundle_components",
    "catalog_price_lists",
    "catalog_prices",
    "catalog_rounding_rules",
    "catalog_price_guards",
    "catalog_variant_costs",
    "catalog_store_overrides",
];

#[async_trait::async_trait]
impl MigrationTrait for CreateCatalog {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Sản phẩm. `readiness` là cột dẫn xuất (D-223…D-227); `ever_active`: đã từng mở bán thì
        // không xoá được, chỉ lưu trữ.
        let mut products = table("catalog_products");
        products
            .col(schema::uuid("org_id"))
            .col(schema::string_len("kind", 16))
            .col(schema::string_len("status", 16))
            .col(schema::string_len("tax_category", 64))
            .col(schema::uuid_null("primary_category_id"))
            .col(schema::boolean("track_inventory"))
            .col(schema::boolean("allow_backorder"))
            .col(schema::boolean("lot_tracked"))
            .col(schema::boolean("serial_tracked"))
            .col(schema::integer_null("warranty_months"))
            .col(schema::string_len("readiness", 24))
            .col(schema::string_len("default_locale", 16))
            .col(schema::boolean("ai_assisted"))
            .col(schema::uuid_null("created_by"))
            .col(schema::timestamp_with_time_zone("created_at"))
            .col(schema::timestamp_with_time_zone("updated_at"))
            .col(schema::timestamp_with_time_zone_null("activated_at"))
            .col(schema::boolean("ever_active"));
        manager.create_table(products).await?;
        let t = "catalog_products";
        index(
            manager,
            t,
            "catalog_products_org_idx",
            &["tenant_id", "org_id", "status", "id"],
            false,
        )
        .await?;
        index(
            manager,
            t,
            "catalog_products_updated_idx",
            &["tenant_id", "updated_at", "id"],
            false,
        )
        .await?;
        index(
            manager,
            t,
            "catalog_products_readiness_idx",
            &["tenant_id", "org_id", "readiness"],
            false,
        )
        .await?;

        // Nội dung theo ngôn ngữ. `search_key` = `search::fold` của tên + SKU + mô tả ngắn;
        // `name_key` = `search::exact_key` của tên (xếp khớp đúng dấu trước, D-201).
        let mut translations = table("catalog_product_translations");
        translations
            .col(schema::uuid("product_id"))
            .col(schema::string_len("locale", 16))
            .col(schema::string_len("name", 200))
            .col(schema::string_len("slug", 100))
            .col(schema::text("short_description"))
            .col(schema::text("description"))
            .col(schema::string_len_null("meta_title", 200))
            .col(schema::string_len_null("meta_description", 500))
            .col(schema::text("search_key"))
            .col(schema::string_len("name_key", 200))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(translations).await?;
        let t = "catalog_product_translations";
        index(manager, t, "catalog_product_tr_slug_unique", &["tenant_id", "locale", "slug"], true)
            .await?;
        index(
            manager,
            t,
            "catalog_product_tr_unique",
            &["tenant_id", "product_id", "locale"],
            true,
        )
        .await?;

        // Tuỳ chọn (màu, cỡ…) và giá trị; tên theo ngôn ngữ là JSON `{locale: tên}`.
        let mut options = table("catalog_options");
        options
            .col(schema::uuid("product_id"))
            .col(schema::integer("position"))
            .col(schema::text("names"));
        manager.create_table(options).await?;
        index(
            manager,
            "catalog_options",
            "catalog_options_product_idx",
            &["tenant_id", "product_id"],
            false,
        )
        .await?;
        let mut values = table("catalog_option_values");
        values
            .col(schema::uuid("product_id"))
            .col(schema::uuid("option_id"))
            .col(schema::integer("position"))
            .col(schema::text("labels"));
        manager.create_table(values).await?;
        index(
            manager,
            "catalog_option_values",
            "catalog_option_values_product_idx",
            &["tenant_id", "product_id"],
            false,
        )
        .await?;

        // Biến thể. `sku_key`: SKU viết hoa, bỏ khoảng trắng; `option_key`: ID giá trị tuỳ chọn
        // sắp tăng, nối `,` (rỗng khi sản phẩm không có tuỳ chọn). `barcode` NULL lặp được ở cả
        // hai database (chuỗi rỗng chuẩn hoá thành NULL).
        let mut variants = table("catalog_variants");
        variants
            .col(schema::uuid("product_id"))
            .col(schema::string_len("sku", 64))
            .col(schema::string_len("sku_key", 64))
            .col(schema::string_len_null("barcode", 32))
            .col(schema::string_len("option_key", 400))
            .col(schema::integer("position"))
            .col(schema::boolean("active"))
            .col(schema::string_len("sale_mode", 16))
            .col(schema::date_null("preorder_available_on"))
            .col(schema::big_integer_null("preorder_limit"))
            .col(schema::timestamp_with_time_zone_null("preorder_ends_at"))
            .col(schema::text_null("preorder_messages"))
            .col(schema::big_integer_null("weight_grams"))
            .col(schema::big_integer_null("length_mm"))
            .col(schema::big_integer_null("width_mm"))
            .col(schema::big_integer_null("height_mm"))
            .col(schema::uuid_null("image_media_id"))
            .col(schema::timestamp_with_time_zone("created_at"))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(variants).await?;
        let t = "catalog_variants";
        index(manager, t, "catalog_variants_sku_unique", &["tenant_id", "sku_key"], true).await?;
        index(manager, t, "catalog_variants_barcode_unique", &["tenant_id", "barcode"], true)
            .await?;
        index(
            manager,
            t,
            "catalog_variants_options_unique",
            &["tenant_id", "product_id", "option_key"],
            true,
        )
        .await?;

        // Ảnh của sản phẩm (media của module `media`, chỉ giữ ID).
        let mut media = table("catalog_product_media");
        media
            .col(schema::uuid("product_id"))
            .col(schema::uuid("media_id"))
            .col(schema::uuid_null("variant_id"))
            .col(schema::integer("position"))
            .col(schema::string_len("alt", 500));
        manager.create_table(media).await?;
        index(
            manager,
            "catalog_product_media",
            "catalog_product_media_idx",
            &["tenant_id", "product_id", "position"],
            false,
        )
        .await?;

        // Danh mục dạng cây: bảng tổ tiên – con cháu (`catalog_category_paths`, gồm chính nó).
        let mut categories = table("catalog_categories");
        categories
            .col(schema::uuid_null("parent_id"))
            .col(schema::integer("depth"))
            .col(schema::integer("position"))
            .col(schema::boolean("visible"))
            .col(schema::timestamp_with_time_zone("created_at"))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(categories).await?;
        index(
            manager,
            "catalog_categories",
            "catalog_categories_parent_idx",
            &["tenant_id", "parent_id", "position"],
            false,
        )
        .await?;
        let mut paths = table("catalog_category_paths");
        paths
            .col(schema::uuid("ancestor_id"))
            .col(schema::uuid("descendant_id"))
            .col(schema::integer("depth"));
        manager.create_table(paths).await?;
        let t = "catalog_category_paths";
        index(
            manager,
            t,
            "catalog_category_paths_unique",
            &["tenant_id", "ancestor_id", "descendant_id"],
            true,
        )
        .await?;
        index(
            manager,
            t,
            "catalog_category_paths_desc_idx",
            &["tenant_id", "descendant_id"],
            false,
        )
        .await?;
        let mut category_texts = table("catalog_category_translations");
        category_texts
            .col(schema::uuid("category_id"))
            .col(schema::string_len("locale", 16))
            .col(schema::string_len("name", 200))
            .col(schema::string_len("slug", 100));
        manager.create_table(category_texts).await?;
        let t = "catalog_category_translations";
        index(
            manager,
            t,
            "catalog_category_tr_slug_unique",
            &["tenant_id", "locale", "slug"],
            true,
        )
        .await?;
        index(
            manager,
            t,
            "catalog_category_tr_unique",
            &["tenant_id", "category_id", "locale"],
            true,
        )
        .await?;
        let mut product_categories = table("catalog_product_categories");
        product_categories.col(schema::uuid("product_id")).col(schema::uuid("category_id"));
        manager.create_table(product_categories).await?;
        let t = "catalog_product_categories";
        index(
            manager,
            t,
            "catalog_product_cat_unique",
            &["tenant_id", "product_id", "category_id"],
            true,
        )
        .await?;
        index(
            manager,
            t,
            "catalog_product_cat_cat_idx",
            &["tenant_id", "category_id", "product_id"],
            false,
        )
        .await?;

        // Thành phần của combo.
        let mut components = table("catalog_bundle_components");
        components
            .col(schema::uuid("bundle_variant_id"))
            .col(schema::uuid("component_variant_id"))
            .col(schema::big_integer("quantity"));
        manager.create_table(components).await?;
        index(
            manager,
            "catalog_bundle_components",
            "catalog_bundle_components_unique",
            &["tenant_id", "bundle_variant_id", "component_variant_id"],
            true,
        )
        .await?;

        // Bảng giá (D-222): mọi cột lọc và cột suy ra có ngay từ đầu; chưa dùng thì NULL.
        let mut lists = table("catalog_price_lists");
        lists
            .col(schema::uuid("org_id"))
            .col(schema::string_len("name", 200))
            .col(schema::string_len("currency", 3))
            .col(schema::boolean("tax_inclusive"))
            .col(schema::string_len("audience", 16))
            .col(schema::uuid_null("company_id"))
            .col(schema::string_len_null("customer_group", 64))
            .col(schema::string_len_null("channel", 64))
            .col(schema::string_len_null("market", 16))
            .col(schema::uuid_null("derived_from"))
            .col(schema::big_integer("adjust_bp"))
            .col(schema::big_integer_null("adjust_minor"))
            .col(schema::uuid_null("rounding_rule_id"))
            .col(schema::integer("priority"))
            .col(schema::boolean("active"))
            .col(schema::uuid_null("created_by"))
            .col(schema::timestamp_with_time_zone("created_at"))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(lists).await?;
        index(
            manager,
            "catalog_price_lists",
            "catalog_price_lists_org_idx",
            &["tenant_id", "org_id", "active"],
            false,
        )
        .await?;

        // Dòng giá: CHỈ THÊM, không sửa, không xoá. Giá hiệu lực lúc `t` = dòng có `valid_from`
        // lớn nhất ≤ `t` và (`valid_to` NULL hay > `t`).
        let mut prices = table("catalog_prices");
        prices
            .col(schema::uuid("price_list_id"))
            .col(schema::uuid("variant_id"))
            .col(schema::big_integer("amount_minor"))
            .col(schema::string_len("amount_currency", 3))
            .col(schema::big_integer_null("compare_at_minor"))
            .col(schema::timestamp_with_time_zone("valid_from"))
            .col(schema::timestamp_with_time_zone_null("valid_to"))
            .col(schema::string_len("source", 16))
            .col(schema::string_len_null("reference", 160))
            .col(schema::string_len_null("reference_fingerprint", 64))
            .col(schema::uuid_null("created_by"))
            .col(schema::timestamp_with_time_zone("created_at"));
        manager.create_table(prices).await?;
        let t = "catalog_prices";
        index(
            manager,
            t,
            "catalog_prices_lookup_idx",
            &["tenant_id", "price_list_id", "variant_id", "valid_from"],
            false,
        )
        .await?;
        index(
            manager,
            t,
            "catalog_prices_reference_unique",
            &["tenant_id", "price_list_id", "reference"],
            true,
        )
        .await?;

        // Quy tắc làm tròn giá đẹp (D-222).
        let mut rounding = table("catalog_rounding_rules");
        rounding
            .col(schema::string_len("name", 200))
            .col(schema::big_integer("step_minor"))
            .col(schema::string_len("mode", 8))
            .col(schema::big_integer_null("ending_minor"))
            .col(schema::timestamp_with_time_zone("created_at"));
        manager.create_table(rounding).await?;
        index(
            manager,
            "catalog_rounding_rules",
            "catalog_rounding_rules_idx",
            &["tenant_id", "id"],
            false,
        )
        .await?;

        // Sàn giá (D-222).
        let mut guards = table("catalog_price_guards");
        guards
            .col(schema::uuid("org_id"))
            .col(schema::uuid_null("category_id"))
            .col(schema::string_len("currency", 3))
            .col(schema::big_integer_null("min_price_minor"))
            .col(schema::big_integer_null("min_margin_bp"))
            .col(schema::string_len("action", 8))
            .col(schema::timestamp_with_time_zone("created_at"))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(guards).await?;
        index(
            manager,
            "catalog_price_guards",
            "catalog_price_guards_org_idx",
            &["tenant_id", "org_id"],
            false,
        )
        .await?;

        // Giá vốn dẫn xuất từ event `inventory.cost_changed` (D-222).
        let mut costs = table("catalog_variant_costs");
        costs
            .col(schema::uuid("org_id"))
            .col(schema::uuid("variant_id"))
            .col(schema::big_integer("unit_cost_minor"))
            .col(schema::string_len("currency", 3))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(costs).await?;
        index(
            manager,
            "catalog_variant_costs",
            "catalog_variant_costs_unique",
            &["tenant_id", "variant_id", "org_id", "currency"],
            true,
        )
        .await?;

        // Cửa hàng ẩn sản phẩm của chuỗi.
        let mut overrides = table("catalog_store_overrides");
        overrides
            .col(schema::uuid("org_id"))
            .col(schema::uuid("product_id"))
            .col(schema::boolean("hidden"))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(overrides).await?;
        index(
            manager,
            "catalog_store_overrides",
            "catalog_store_overrides_unique",
            &["tenant_id", "org_id", "product_id"],
            true,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for name in TABLES.iter().rev() {
            manager.drop_table(Table::drop().table(Alias::new(*name)).to_owned()).await?;
        }
        Ok(())
    }
}

struct CreateAttributes;

impl MigrationName for CreateAttributes {
    fn name(&self) -> &str {
        "m0007_attributes_restrictions"
    }
}

const ATTRIBUTE_TABLES: [&str; 7] = [
    "catalog_attributes",
    "catalog_attribute_options",
    "catalog_category_attributes",
    "catalog_product_attributes",
    "catalog_restriction_rules",
    "catalog_category_restrictions",
    "catalog_product_documents",
];

#[async_trait::async_trait]
impl MigrationTrait for CreateAttributes {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Thuộc tính của khách (xuất xứ, chất liệu…). `decimal` lưu nguyên theo `scale`.
        let mut attributes = table("catalog_attributes");
        attributes
            .col(schema::string_len("code", 64))
            .col(schema::string_len("data_type", 16))
            .col(schema::integer("scale"))
            .col(schema::string_len_null("unit", 16))
            .col(schema::text("names"))
            .col(schema::boolean("filterable"))
            .col(schema::string_len_null("legal_basis", 300))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(attributes).await?;
        index(
            manager,
            "catalog_attributes",
            "catalog_attributes_code_unique",
            &["tenant_id", "code"],
            true,
        )
        .await?;
        let mut options = table("catalog_attribute_options");
        options
            .col(schema::uuid("attribute_id"))
            .col(schema::string_len("code", 64))
            .col(schema::text("labels"))
            .col(schema::integer("position"));
        manager.create_table(options).await?;
        index(
            manager,
            "catalog_attribute_options",
            "catalog_attribute_options_unique",
            &["tenant_id", "attribute_id", "code"],
            true,
        )
        .await?;
        // Thuộc tính của danh mục; bắt buộc là dữ liệu có ngày hiệu lực (D-223…D-227).
        let mut category_attributes = table("catalog_category_attributes");
        category_attributes
            .col(schema::uuid("category_id"))
            .col(schema::uuid("attribute_id"))
            .col(schema::boolean("required"))
            .col(schema::boolean("applies_to_descendants"))
            .col(schema::date("effective_from"))
            .col(schema::date_null("effective_to"))
            .col(schema::string_len_null("legal_basis", 300));
        manager.create_table(category_attributes).await?;
        index(
            manager,
            "catalog_category_attributes",
            "catalog_category_attributes_idx",
            &["tenant_id", "category_id"],
            false,
        )
        .await?;
        let mut values = table("catalog_product_attributes");
        values
            .col(schema::uuid("product_id"))
            .col(schema::uuid("attribute_id"))
            .col(schema::text_null("value_text"))
            .col(schema::big_integer_null("value_integer"))
            .col(schema::boolean_null("value_boolean"))
            .col(schema::date_null("value_date"))
            .col(schema::uuid_null("option_id"));
        manager.create_table(values).await?;
        let t = "catalog_product_attributes";
        index(
            manager,
            t,
            "catalog_product_attr_option_idx",
            &["tenant_id", "attribute_id", "option_id", "product_id"],
            false,
        )
        .await?;
        index(
            manager,
            t,
            "catalog_product_attr_integer_idx",
            &["tenant_id", "attribute_id", "value_integer"],
            false,
        )
        .await?;
        index(manager, t, "catalog_product_attr_product_idx", &["tenant_id", "product_id"], false)
            .await?;
        // Dữ liệu luật dùng chung mọi khách (mặt điều khiển, D-198): không có `tenant_id`.
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("catalog_restriction_rules"))
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("jurisdiction", 8))
                    .col(schema::string_len("code", 64))
                    .col(schema::string_len("level", 16))
                    .col(schema::text("names"))
                    .col(schema::text("required_documents"))
                    .col(schema::string_len("legal_basis", 300))
                    .col(schema::string_len("source_url", 300))
                    .col(schema::date("effective_from"))
                    .col(schema::date_null("effective_to"))
                    .col(schema::string_len("pack_version", 32))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "catalog_restriction_rules",
            "catalog_restriction_rules_unique",
            &["jurisdiction", "code", "effective_from"],
            true,
        )
        .await?;
        let mut links = table("catalog_category_restrictions");
        links.col(schema::uuid("category_id")).col(schema::string_len("rule_code", 64));
        manager.create_table(links).await?;
        index(
            manager,
            "catalog_category_restrictions",
            "catalog_category_restrictions_unique",
            &["tenant_id", "category_id", "rule_code"],
            true,
        )
        .await?;
        let mut documents = table("catalog_product_documents");
        documents
            .col(schema::uuid("product_id"))
            .col(schema::uuid("org_id"))
            .col(schema::string_len("document_type", 64))
            .col(schema::string_len("number", 100))
            .col(schema::string_len("issued_by", 200))
            .col(schema::date("issued_on"))
            .col(schema::date_null("expires_on"))
            .col(schema::string_len_null("file_key", 300))
            .col(schema::timestamp_with_time_zone("created_at"));
        manager.create_table(documents).await?;
        index(
            manager,
            "catalog_product_documents",
            "catalog_product_documents_idx",
            &["tenant_id", "product_id"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for name in ATTRIBUTE_TABLES.iter().rev() {
            manager.drop_table(Table::drop().table(Alias::new(*name)).to_owned()).await?;
        }
        Ok(())
    }
}

struct CreatePriceChanges;

impl MigrationName for CreatePriceChanges {
    fn name(&self) -> &str {
        "m0008_price_changes"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreatePriceChanges {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Đổi giá hàng loạt (D-222): nháp → xem trước → duyệt → job ghi dòng giá `bulk`.
        let mut changes = table("catalog_price_changes");
        changes
            .col(schema::uuid("price_list_id"))
            .col(schema::text("filter"))
            .col(schema::string_len("operation", 16))
            .col(schema::big_integer("value"))
            .col(schema::uuid_null("rounding_rule_id"))
            .col(schema::timestamp_with_time_zone("valid_from"))
            .col(schema::timestamp_with_time_zone_null("valid_to"))
            .col(schema::string_len("status", 16))
            .col(schema::big_integer("affected"))
            .col(schema::text_null("preview"))
            .col(schema::uuid_null("created_by"))
            .col(schema::uuid_null("approved_by"))
            .col(schema::timestamp_with_time_zone("created_at"))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(changes).await?;
        index(
            manager,
            "catalog_price_changes",
            "catalog_price_changes_list_idx",
            &["tenant_id", "price_list_id", "id"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Alias::new("catalog_price_changes")).to_owned())
            .await
    }
}

struct CreateStorefront;

impl MigrationName for CreateStorefront {
    fn name(&self) -> &str {
        "m0009_storefront"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateStorefront {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Bảng dẫn xuất cho đọc công khai: một dòng mỗi (nút bán có giá riêng, sản phẩm). Giá,
        // hiện / ẩn và cột sắp xếp ghi sẵn để lọc và sắp có index; job dựng lại (chạy lại được).
        let mut entries = table("catalog_storefront_entries");
        entries
            .col(schema::uuid("org_id"))
            .col(schema::uuid("product_id"))
            .col(schema::uuid("owner_org_id"))
            .col(schema::boolean("visible"))
            .col(schema::string_len("currency", 3))
            .col(schema::big_integer("min_price_minor"))
            .col(schema::big_integer("max_price_minor"))
            .col(schema::big_integer_null("compare_at_minor"))
            .col(schema::big_integer_null("sale_price_minor"))
            .col(schema::timestamp_with_time_zone_null("sale_ends_at"))
            .col(schema::big_integer("best_selling"))
            .col(schema::integer("rating_x100"))
            .col(schema::big_integer("rating_count"))
            .col(schema::integer("variant_count"))
            .col(schema::timestamp_with_time_zone("published_at"))
            .col(schema::timestamp_with_time_zone("refreshed_at"));
        manager.create_table(entries).await?;
        let t = "catalog_storefront_entries";
        index(
            manager,
            t,
            "catalog_storefront_unique",
            &["tenant_id", "org_id", "product_id"],
            true,
        )
        .await?;
        index(
            manager,
            t,
            "catalog_storefront_price_idx",
            &["tenant_id", "org_id", "visible", "min_price_minor", "id"],
            false,
        )
        .await?;
        index(
            manager,
            t,
            "catalog_storefront_newest_idx",
            &["tenant_id", "org_id", "visible", "published_at", "id"],
            false,
        )
        .await?;
        index(
            manager,
            t,
            "catalog_storefront_selling_idx",
            &["tenant_id", "org_id", "visible", "best_selling", "id"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Alias::new("catalog_storefront_entries")).to_owned())
            .await
    }
}
