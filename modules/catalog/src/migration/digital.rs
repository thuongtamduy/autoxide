//! m0019: hàng số — file của biến thể (`catalog_digital_assets`) và quyền tải theo dòng đơn
//! (`catalog_download_grants`). Token tải chỉ lưu bản băm SHA-256 (`token_hash`), không bao giờ lưu
//! dạng đọc được.

use autoxide_kernel::migration::prelude::*;

use super::{index, table};

pub(super) struct CreateDigital;

impl MigrationName for CreateDigital {
    fn name(&self) -> &str {
        "m0019_digital"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateDigital {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut assets = table("catalog_digital_assets");
        assets
            .col(schema::uuid("product_id"))
            .col(schema::uuid("variant_id"))
            .col(schema::string_len("file_name", 255))
            .col(schema::string_len("content_type", 255))
            .col(schema::big_integer("size"))
            .col(schema::string_len("sha256", 64))
            // `catalog/<id>/file` trong nơi lưu của khách: kernel chép theo `<id>` khi tách khách.
            .col(schema::string_len("storage_key", 200))
            .col(schema::big_integer("download_limit"))
            .col(schema::big_integer("grant_days"))
            .col(schema::uuid_null("created_by"))
            .col(schema::timestamp_with_time_zone("created_at"));
        manager.create_table(assets).await?;
        let name = "catalog_digital_assets";
        index(
            manager,
            name,
            "catalog_digital_assets_variant_idx",
            &["tenant_id", "variant_id"],
            false,
        )
        .await?;
        index(
            manager,
            name,
            "catalog_digital_assets_product_idx",
            &["tenant_id", "product_id"],
            false,
        )
        .await?;

        let mut grants = table("catalog_download_grants");
        grants
            .col(schema::uuid("order_id"))
            .col(schema::uuid("order_line_id"))
            .col(schema::uuid_null("buyer_id"))
            // Nút sở hữu sản phẩm: kiểm quyền xem / thu hồi theo cây tổ chức.
            .col(schema::uuid("org_id"))
            .col(schema::uuid("asset_id"))
            .col(schema::uuid("variant_id"))
            .col(schema::string_len("token_hash", 64))
            .col(schema::big_integer("remaining"))
            .col(schema::timestamp_with_time_zone("expires_at"))
            .col(schema::timestamp_with_time_zone_null("revoked_at"))
            .col(schema::timestamp_with_time_zone("created_at"));
        manager.create_table(grants).await?;
        let name = "catalog_download_grants";
        index(
            manager,
            name,
            "catalog_download_grants_line_uq",
            &["tenant_id", "order_line_id", "asset_id"],
            true,
        )
        .await?;
        index(
            manager,
            name,
            "catalog_download_grants_token_uq",
            &["tenant_id", "token_hash"],
            true,
        )
        .await?;
        index(
            manager,
            name,
            "catalog_download_grants_order_idx",
            &["tenant_id", "order_id"],
            false,
        )
        .await?;
        index(
            manager,
            name,
            "catalog_download_grants_buyer_idx",
            &["tenant_id", "buyer_id", "id"],
            false,
        )
        .await?;
        index(manager, name, "catalog_download_grants_asset_idx", &["tenant_id", "asset_id"], false)
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for name in ["catalog_download_grants", "catalog_digital_assets"] {
            manager.drop_table(Table::drop().table(Alias::new(name)).to_owned()).await?;
        }
        Ok(())
    }
}
