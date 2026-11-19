//! Permission của module `catalog`: `<module>.<việc>`. Handler REST gọi `principal.require(&…)`
//! rồi kiểm phạm vi theo nút của bản ghi (`require_at` / `require_write_at`, bước 34); tool MCP
//! khai `.requires(…)`.

use autoxide_kernel::access::Permission;

// Theo nút (bước 34): độ sâu `unit` / `subtree` = sản phẩm, bảng giá của nút / nhánh; `tenant` =
// của cả khách.
pub(crate) const PRODUCTS_READ: Permission = Permission::new(
    "catalog.products.read",
    "See the products, variants and categories in your scope, including drafts.",
)
.scoped();

pub(crate) const PRODUCTS_WRITE: Permission = Permission::new(
    "catalog.products.write",
    "Create and change the products and variants in your scope (not their sale status).",
)
.scoped();

pub(crate) const PRODUCTS_PUBLISH: Permission = Permission::new(
    "catalog.products.publish",
    "Change the sale status of the products in your scope: put on sale, stop, archive.",
)
.scoped();

pub(crate) const PRICES_READ: Permission =
    Permission::new("catalog.prices.read", "See the price lists and prices in your scope.")
        .scoped();

pub(crate) const PRICES_WRITE: Permission = Permission::new(
    "catalog.prices.write",
    "Create price lists, add prices and set price floors in your scope.",
)
.scoped();

pub(crate) const PRICES_APPROVE: Permission =
    Permission::new("catalog.prices.approve", "Approve bulk price changes in your scope.").scoped();

// Danh mục, thuộc tính và mã hàng có điều kiện thuộc cả khách: cần quyền ở độ sâu `tenant`.
pub(crate) const CATEGORIES_MANAGE: Permission = Permission::new(
    "catalog.categories.manage",
    "Manage the category tree, attributes and restricted-goods links of the whole tenant.",
);

pub(crate) const DOCUMENTS_MANAGE: Permission = Permission::new(
    "catalog.documents.manage",
    "Manage the licences and certificates of restricted products in your scope.",
)
.scoped();

pub(crate) const IMPORT: Permission =
    Permission::new("catalog.import", "Import products and prices from CSV files.");

pub(crate) const EXPORT: Permission =
    Permission::new("catalog.export", "Export products and prices to CSV files.");

pub(crate) const DOWNLOADS_MANAGE: Permission = Permission::new(
    "catalog.downloads.manage",
    "See and revoke the download grants of digital products.",
);

pub(crate) const COLLECTIONS_MANAGE: Permission = Permission::new(
    "catalog.collections.manage",
    "Create, change, order and archive the product collections in your scope for the website.",
)
.scoped();

pub(crate) fn all() -> Vec<Permission> {
    vec![
        PRODUCTS_READ,
        PRODUCTS_WRITE,
        PRODUCTS_PUBLISH,
        PRICES_READ,
        PRICES_WRITE,
        PRICES_APPROVE,
        CATEGORIES_MANAGE,
        DOCUMENTS_MANAGE,
        IMPORT,
        EXPORT,
        DOWNLOADS_MANAGE,
        COLLECTIONS_MANAGE,
    ]
}
