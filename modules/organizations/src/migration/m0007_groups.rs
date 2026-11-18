//! Nhóm nút ngoài cây cho báo cáo (bước 38, D-232): tập nút tự do trong một khách ("cửa hàng miền
//! Bắc"), một nút ở nhiều nhóm được. Nhóm KHÔNG cấp quyền. Thành viên chỉ giữ ID nút (không foreign
//! key sang bảng nút: xoá / lưu trữ nút không xoá thành viên, báo cáo lịch sử vẫn gom được).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0007_groups"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("organizations_group")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("id"))
                    .col(schema::string_len("name", 200))
                    .col(schema::string_len("name_folded", 200))
                    .col(schema::string_len_null("description", 1000))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("tenant_id").col("id"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("organizations_group_name_unique")
                    .table("organizations_group")
                    .col("tenant_id")
                    .col("name_folded")
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("organizations_group_member")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("group_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::uuid_null("added_by"))
                    .col(schema::timestamp_with_time_zone("added_at"))
                    .primary_key(Index::create().col("tenant_id").col("group_id").col("org_id"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("organizations_group_member_org_idx")
                    .table("organizations_group_member")
                    .col("tenant_id")
                    .col("org_id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("organizations_group_member").to_owned()).await?;
        manager.drop_table(Table::drop().table("organizations_group").to_owned()).await
    }
}
