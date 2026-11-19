//! Bộ sưu tập sản phẩm (bước 55 mục 3, D-20261119-step55-collections): chọn tay (có thứ tự) hay
//! theo quy tắc (danh mục, thuộc tính, khoảng giá, hàng mới, đang khuyến mại), sắp theo thứ tự chọn
//! / mới nhất / giá / bán chạy; trang công khai có slug và SEO theo ngôn ngữ.
//!
//! - Quản trị cần `catalog.collections.manage` ở nút của bộ sưu tập; đọc cần `catalog.products.read`.
//! - Đọc công khai dùng bảng dẫn xuất `catalog_storefront_entries` như danh sách sản phẩm (cùng
//!   bộ lọc, cùng index): quy tắc thành bộ lọc, không câu SQL riêng.
//! - Đăng ký `catalog.collection` vào sổ đối tượng: banner, menu trỏ tới bộ sưu tập.

pub(crate) mod public;
pub(crate) mod resource;
pub(crate) mod rules;
pub(crate) mod service;
pub(crate) mod tools;

/// Kiểu sắp của bộ sưu tập.
pub(crate) const SORTS: [&str; 5] = ["manual", "newest", "price_asc", "price_desc", "best_selling"];
/// Trạng thái.
pub(crate) const STATUSES: [&str; 3] = ["draft", "active", "archived"];
/// Số sản phẩm chọn tay tối đa của một bộ sưu tập.
pub(crate) const MAX_ITEMS: usize = 500;

pub(crate) fn collection_not_found(id: impl ToString) -> autoxide_kernel::error::Error {
    autoxide_kernel::error::Error::not_found(
        "catalog.collection_not_found",
        "There is no product collection `{id}`.",
    )
    .with_param("id", id.to_string())
}
