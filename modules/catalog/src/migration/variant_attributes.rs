//! m0018: thuộc tính theo biến thể cho trang sản phẩm (website bán hàng, 16/11/2026): số người
//! dùng, trong hộp có gì, cách dùng… — cùng bộ thuộc tính khai theo danh mục, không viết cứng ngành.
//! Thuộc tính chữ có bản dịch theo ngôn ngữ (`texts`).

use autoxide_kernel::migration::prelude::*;

use super::{index, table};

pub(super) struct CreateVariantAttributes;

impl MigrationName for CreateVariantAttributes {
    fn name(&self) -> &str {
        "m0018_variant_attributes"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateVariantAttributes {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut rows = table("catalog_variant_attributes");
        rows.col(schema::uuid("product_id"))
            .col(schema::uuid("variant_id"))
            .col(schema::uuid("attribute_id"))
            .col(schema::text_null("value_text"))
            .col(schema::big_integer_null("value_integer"))
            .col(schema::boolean_null("value_boolean"))
            .col(schema::date_null("value_date"))
            .col(schema::uuid_null("option_id"))
            // JSON `{ngôn ngữ: chữ}` cho thuộc tính kiểu chữ; `value_text` là bản ngôn ngữ mặc định.
            .col(schema::text_null("texts"));
        manager.create_table(rows).await?;
        index(
            manager,
            "catalog_variant_attributes",
            "catalog_variant_attributes_variant_idx",
            &["tenant_id", "variant_id", "attribute_id"],
            false,
        )
        .await?;
        index(
            manager,
            "catalog_variant_attributes",
            "catalog_variant_attributes_product_idx",
            &["tenant_id", "product_id"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Alias::new("catalog_variant_attributes")).to_owned())
            .await
    }
}
