//! Chính sách tách / gộp khách của các bảng của `catalog` (bước 38, D-20261118-step38-relocation):
//! sản phẩm, bảng giá, rào giá, giá vốn và tuỳ biến cửa hàng theo nút; bảng con đi theo dòng cha;
//! danh mục và quy tắc làm tròn là cấu hình cả khách, chép sang khách mới khi tách.

use autoxide_kernel::tenancy::Relocation;

pub(crate) fn relocation() -> Relocation {
    Relocation::new()
        .by_org(&[
            "catalog_products",
            "catalog_price_lists",
            "catalog_price_guards",
            "catalog_variant_costs",
            "catalog_store_overrides",
        ])
        .follow("catalog_product_translations", "product_id", "catalog_products")
        .follow("catalog_options", "product_id", "catalog_products")
        .follow("catalog_option_values", "option_id", "catalog_options")
        .follow("catalog_variants", "product_id", "catalog_products")
        .follow("catalog_product_media", "product_id", "catalog_products")
        .follow("catalog_product_categories", "product_id", "catalog_products")
        .follow("catalog_bundle_components", "bundle_variant_id", "catalog_variants")
        .follow("catalog_prices", "price_list_id", "catalog_price_lists")
        .copy_on_split(&["catalog_categories", "catalog_rounding_rules"])
        .follow("catalog_category_paths", "descendant_id", "catalog_categories")
        .follow("catalog_category_translations", "category_id", "catalog_categories")
}
