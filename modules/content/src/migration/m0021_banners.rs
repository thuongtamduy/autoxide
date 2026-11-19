//! Vị trí hiển thị và banner (bước 55, D-236, D-20261119-step55-banners).
//!
//! - `content_placements`: vị trí theo khoá (vd `home_hero`) của cả khách, mang kiểu hiển thị, cỡ
//!   ảnh, thiết bị, loại trang dùng được. Một khoá mỗi khách.
//! - `content_banners`: banner của một vị trí, thuộc một nút (`org_id`, gốc = mọi cửa hàng), có
//!   khung giờ, thứ tự, link tới địa chỉ hay đối tượng của sổ đối tượng (chỉ giữ loại và ID — không
//!   foreign key xuyên module).
//!
//! JSON (`display_settings`, `page_kinds`, `page_targets`) lưu trong `text`: chạy như nhau trên hai
//! database. Không enum của Postgres: chữ có kiểm ở service.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0021_banners"
    }
}

fn index(
    table: &'static str,
    name: &'static str,
    columns: &[&'static str],
) -> IndexCreateStatement {
    let mut index = Index::create();
    index.name(name).table(table);
    for column in columns {
        index.col(*column);
    }
    index.to_owned()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("content_placements")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("key", 64))
                    .col(schema::string_len("name", 120))
                    .col(schema::string_len("description", 500))
                    .col(schema::integer("max_items"))
                    // `site` (website của khách); `seller_store` để dành cho gian hàng (D-233).
                    .col(schema::string_len("audience", 16))
                    .col(schema::string_len("display", 16))
                    .col(schema::text("display_settings"))
                    .col(schema::string_len("accepts", 8))
                    .col(schema::integer_null("ratio_w"))
                    .col(schema::integer_null("ratio_h"))
                    .col(schema::integer_null("mobile_ratio_w"))
                    .col(schema::integer_null("mobile_ratio_h"))
                    .col(schema::integer_null("min_width"))
                    .col(schema::integer_null("min_height"))
                    .col(schema::string_len("devices", 8))
                    .col(schema::text("page_kinds"))
                    // Khoá trong bộ vị trí chuẩn, khi dòng do bộ đó gieo.
                    .col(schema::string_len_null("preset_key", 64))
                    .col(schema::boolean("active"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                index("content_placements", "content_placements_key_unique", &["tenant_id", "key"])
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("content_banners")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("placement_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("locale", 35))
                    .col(schema::string_len("kind", 8))
                    .col(schema::string_len("title", 120))
                    .col(schema::string_len("subtitle", 300))
                    .col(schema::string_len("cta_label", 60))
                    .col(schema::string_len_null("tone", 16))
                    .col(schema::uuid_null("media_id"))
                    .col(schema::uuid_null("mobile_media_id"))
                    .col(schema::string_len("alt", 300))
                    .col(schema::text("page_kinds"))
                    .col(schema::text("page_targets"))
                    // Link: `none`, `url` hay một loại của sổ đối tượng (vd `catalog.product`).
                    .col(schema::string_len("link_kind", 64))
                    .col(schema::string_len_null("link_url", 2048))
                    .col(schema::string_len_null("link_id", 64))
                    .col(schema::timestamp_with_time_zone_null("starts_at"))
                    .col(schema::timestamp_with_time_zone_null("ends_at"))
                    .col(schema::integer("position"))
                    .col(schema::string_len("status", 16))
                    .col(schema::boolean("ai_assisted"))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        // Đọc công khai và danh sách theo vị trí: banner của một vị trí theo trạng thái, thứ tự.
        manager
            .create_index(index(
                "content_banners",
                "content_banners_placement_idx",
                &["tenant_id", "placement_id", "status", "position"],
            ))
            .await?;
        // Banner trỏ tới một đối tượng (vd ẩn khi chương trình khuyến mại kết thúc).
        manager
            .create_index(index(
                "content_banners",
                "content_banners_link_idx",
                &["tenant_id", "link_kind", "link_id"],
            ))
            .await?;
        // Danh sách theo khách sắp theo `id` (D-302) và tách khách theo nút.
        manager
            .create_index(index(
                "content_banners",
                "content_banners_list_idx",
                &["tenant_id", "id"],
            ))
            .await?;
        manager
            .create_index(index(
                "content_banners",
                "content_banners_org_idx",
                &["tenant_id", "org_id", "id"],
            ))
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("content_banners").to_owned()).await?;
        manager.drop_table(Table::drop().table("content_placements").to_owned()).await
    }
}
