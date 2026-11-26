//! Index cho danh sách có phân trang (bước 70 P9 mục 3, D-20261125-list-pages): mỗi danh sách duyệt bằng keyset
//! có index cột lọc + cột sắp + id, để database nhảy thẳng tới điểm bắt đầu trang. Index cũ là tiền tố của index
//! mới thì thừa: xoá (`down` tạo lại).

use autoxide_kernel::migration::prelude::*;

use super::index;

pub(super) struct CreateListIndexes;

impl MigrationName for CreateListIndexes {
    fn name(&self) -> &str {
        "m0021_list_indexes"
    }
}

/// `(bảng, tên index, cột)` thêm mới.
const ADDED: [(&str, &str, &[&str]); 6] = [
    // `GET /categories`, `GET /public/categories`: cây theo độ sâu, vị trí, id.
    (
        "catalog_categories",
        "catalog_categories_tree_page_idx",
        &["tenant_id", "depth", "position", "id"],
    ),
    // `GET /categories/{id}/attributes`: theo thứ tự của lần thay gần nhất (id).
    (
        "catalog_category_attributes",
        "catalog_category_attributes_page_idx",
        &["tenant_id", "category_id", "id"],
    ),
    // `GET /products/{id}/attributes`.
    (
        "catalog_product_attributes",
        "catalog_product_attr_product_page_idx",
        &["tenant_id", "product_id", "id"],
    ),
    // `GET /variants/{id}/attributes`.
    (
        "catalog_variant_attributes",
        "catalog_variant_attributes_page_idx",
        &["tenant_id", "variant_id", "id"],
    ),
    // `GET /products/{id}/documents`: theo thứ tự tạo.
    (
        "catalog_product_documents",
        "catalog_product_documents_page_idx",
        &["tenant_id", "product_id", "id"],
    ),
    // `GET /variants/{id}/digital-assets`: theo thứ tự tạo.
    (
        "catalog_digital_assets",
        "catalog_digital_assets_variant_page_idx",
        &["tenant_id", "variant_id", "id"],
    ),
];

/// Index cũ là tiền tố của index mới cùng bảng: xoá khi lên, tạo lại khi xuống.
const REPLACED: [(&str, &str, &[&str]); 4] = [
    (
        "catalog_category_attributes",
        "catalog_category_attributes_idx",
        &["tenant_id", "category_id"],
    ),
    (
        "catalog_product_attributes",
        "catalog_product_attr_product_idx",
        &["tenant_id", "product_id"],
    ),
    ("catalog_product_documents", "catalog_product_documents_idx", &["tenant_id", "product_id"]),
    ("catalog_digital_assets", "catalog_digital_assets_variant_idx", &["tenant_id", "variant_id"]),
];

#[async_trait::async_trait]
impl MigrationTrait for CreateListIndexes {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, name, columns) in ADDED {
            index(manager, table, name, columns, false).await?;
        }
        for (table, name, _) in REPLACED {
            manager
                .drop_index(Index::drop().name(name).table(Alias::new(table)).to_owned())
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, name, columns) in REPLACED {
            index(manager, table, name, columns, false).await?;
        }
        for (table, name, _) in ADDED {
            manager
                .drop_index(Index::drop().name(name).table(Alias::new(table)).to_owned())
                .await?;
        }
        Ok(())
    }
}
