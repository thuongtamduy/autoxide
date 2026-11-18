//! `readiness` của sản phẩm (D-223…D-227): cột dẫn xuất cho biết sản phẩm đã mở bán được chưa
//! (`ready`, `missing_attributes`, `missing_documents`, `restricted`). Thuộc tính bắt buộc và
//! giấy tờ của hàng có điều kiện tính lại cột này; mở bán đọc nó.

use autoxide_kernel::error::Error;

use crate::entity::product;

pub(crate) const MISSING_ATTRIBUTES: &str = "missing_attributes";
pub(crate) const MISSING_DOCUMENTS: &str = "missing_documents";
pub(crate) const RESTRICTED: &str = "restricted";

/// Mở bán chỉ khi `readiness = ready`; còn lại báo đúng mã lỗi của lý do.
pub(crate) fn require_ready(product: &product::Model) -> Result<(), Error> {
    match product.readiness.as_str() {
        MISSING_ATTRIBUTES => Err(Error::new(
            autoxide_kernel::error::ErrorKind::Validation,
            "catalog.required_attributes_missing",
            "Required attributes of the product's categories are missing: {attributes}.",
        )
        .with_param("attributes", "see `GET /api/v1/catalog/products/{id}`")),
        MISSING_DOCUMENTS => Err(Error::conflict(
            "catalog.documents_required",
            "This restricted product needs valid documents: {documents}.",
        )
        .with_param("documents", "see `GET /api/v1/catalog/products/{id}`")),
        RESTRICTED => Err(Error::conflict(
            "catalog.prohibited_goods",
            "The product belongs to a category of prohibited goods ({rule}) and cannot be sold.",
        )
        .with_param("rule", "see the category restrictions")),
        _ => Ok(()),
    }
}
