//! Mỗi cỡ ảnh thu nhỏ có hai bản: JPEG / PNG và WebP (D-186). Unique index của `media_variant`
//! thêm cột `content_type`, để hai bản cùng cỡ cùng nằm được trong bảng.

use autoxide_kernel::migration::prelude::*;
use sea_orm::ConnectionTrait;

pub(crate) struct Migration;

/// Index cũ (m0004): một dòng cho mỗi (item, phiên bản, chiều rộng).
const OLD: &str = "media_variant_unique";
/// Index mới: một dòng cho mỗi (item, phiên bản, chiều rộng, loại nội dung).
const NEW: &str = "media_variant_format_unique";

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0007_variant_formats"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_index(Index::drop().name(OLD).table("media_variant").to_owned()).await?;
        manager
            .create_index(
                Index::create()
                    .name(NEW)
                    .table("media_variant")
                    .col("item_id")
                    .col("version_id")
                    .col("width")
                    .col("content_type")
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Index cũ không chứa nổi hai bản cùng cỡ: bỏ dòng của bản WebP trước. File của chúng ở
        // nơi lưu không còn dòng nào trỏ tới; `media.make_variants` của phiên bản sau hay việc
        // xoá item không dọn chúng — chấp nhận được cho một lần hạ phiên bản.
        let delete = Query::delete()
            .from_table("media_variant")
            .and_where(Expr::col("content_type").eq("image/webp"))
            .to_owned();
        manager.get_connection().execute(&delete).await?;
        manager.drop_index(Index::drop().name(NEW).table("media_variant").to_owned()).await?;
        manager
            .create_index(
                Index::create()
                    .name(OLD)
                    .table("media_variant")
                    .col("item_id")
                    .col("version_id")
                    .col("width")
                    .unique()
                    .to_owned(),
            )
            .await
    }
}
