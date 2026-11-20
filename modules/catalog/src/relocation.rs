//! Chính sách tách / gộp khách của các bảng của `catalog` (bước 38, D-20261118-step38-relocation):
//! sản phẩm, bảng giá, rào giá, giá vốn và tuỳ biến cửa hàng theo nút; bảng con đi theo dòng cha;
//! danh mục, thuộc tính và quy tắc làm tròn là cấu hình cả khách, chép sang khách mới khi tách
//! (cùng thuộc tính, quy tắc hàng cấm của danh mục).

use autoxide_kernel::tenancy::Relocation;

pub(crate) fn relocation() -> Relocation {
    Relocation::new()
        .by_org(&[
            "catalog_products",
            "catalog_price_lists",
            "catalog_price_guards",
            "catalog_variant_costs",
            "catalog_store_overrides",
            "catalog_collections",
        ])
        // Bảng dẫn xuất: dựng lại sau khi tách / gộp (`autoxide task catalog.rebuild_storefront`).
        .follow("catalog_storefront_entries", "product_id", "catalog_products")
        .follow("catalog_product_translations", "product_id", "catalog_products")
        .follow("catalog_options", "product_id", "catalog_products")
        .follow("catalog_option_values", "option_id", "catalog_options")
        .follow("catalog_variants", "product_id", "catalog_products")
        .follow("catalog_product_media", "product_id", "catalog_products")
        .follow("catalog_product_categories", "product_id", "catalog_products")
        .follow("catalog_bundle_components", "bundle_variant_id", "catalog_variants")
        .follow("catalog_prices", "price_list_id", "catalog_price_lists")
        .follow("catalog_price_changes", "price_list_id", "catalog_price_lists")
        .follow("catalog_product_attributes", "product_id", "catalog_products")
        .follow("catalog_product_documents", "product_id", "catalog_products")
        .follow("catalog_collection_texts", "collection_id", "catalog_collections")
        .follow("catalog_collection_items", "collection_id", "catalog_collections")
        // Bán chạy, đánh giá, gợi ý sản phẩm (m0013): đi theo sản phẩm.
        .follow("catalog_sold_products", "product_id", "catalog_products")
        .follow("catalog_product_stats", "product_id", "catalog_products")
        .follow("catalog_co_purchases", "product_id", "catalog_products")
        .follow("catalog_embeddings", "product_id", "catalog_products")
        .follow("catalog_similar_products", "product_id", "catalog_products")
        .copy_on_split(&[
            "catalog_categories",
            "catalog_rounding_rules",
            "catalog_attributes",
            "catalog_synonyms",
        ])
        .follow("catalog_attribute_options", "attribute_id", "catalog_attributes")
        .follow("catalog_category_attributes", "category_id", "catalog_categories")
        .follow("catalog_category_restrictions", "category_id", "catalog_categories")
        .follow("catalog_category_paths", "descendant_id", "catalog_categories")
        .follow("catalog_category_translations", "category_id", "catalog_categories")
}
