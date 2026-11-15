//! Bảng `menus_menus` (menu theo `key`, `locale`) và `menus_items` (mục menu, cây nhiều tầng).
//! Chưa có `tenant_id` / `org_id`: bước 33 và 34 thêm cho mọi bảng cùng lúc (D-238).
//!
//! Đích của mục chỉ giữ loại và ID trong sổ đối tượng (`ref_kind`, `ref_id`) — không foreign key
//! xuyên module.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0001_create_menus"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("menus_menus")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("key", 64))
                    .col(schema::string_len("locale", 35))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        // Một menu cho mỗi (`key`, `locale`): database chặn cả khi hai request tạo cùng lúc. Index
        // này cũng phục vụ đọc menu công khai.
        manager
            .create_index(
                Index::create()
                    .name("menus_menus_key_locale_unique")
                    .table("menus_menus")
                    .col("key")
                    .col("locale")
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("menus_items")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("menu_id"))
                    .col(schema::uuid_null("parent_id"))
                    .col(schema::integer("position"))
                    // Rỗng: lấy tiêu đề của đối tượng đích.
                    .col(schema::string_len("label", 200))
                    .col(schema::string_len_null("url", 2048))
                    .col(schema::string_len_null("ref_kind", 64))
                    .col(schema::string_len_null("ref_id", 64))
                    .col(schema::boolean("auto_children"))
                    .col(schema::integer_null("auto_children_limit"))
                    .col(schema::string_len_null("icon", 32))
                    .col(schema::string_len_null("highlight", 16))
                    .col(schema::string_len_null("badge_text", 32))
                    .col(schema::string_len_null("badge_tone", 16))
                    .col(schema::timestamp_with_time_zone_null("badge_from"))
                    .col(schema::timestamp_with_time_zone_null("badge_until"))
                    .col(schema::string_len_null("feature_kind", 64))
                    .col(schema::string_len_null("feature_id", 64))
                    .to_owned(),
            )
            .await?;
        // Mục của một menu theo thứ tự.
        manager
            .create_index(
                Index::create()
                    .name("menus_items_menu_position_idx")
                    .table("menus_items")
                    .col("menu_id")
                    .col("position")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("menus_items").to_owned()).await?;
        manager.drop_table(Table::drop().table("menus_menus").to_owned()).await
    }
}
