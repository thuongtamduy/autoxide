//! Nhóm SCIM và luật ánh xạ nhóm → role ở nút (bước 66, D-20261121-scim-groups). Bảng cấp khách:
//! khoá chính `(tenant_id, id)`, không foreign key (xoá cùng nhóm / tài khoản trong transaction).
//!
//! - `auth_scim_groups`: nhóm hệ thống danh tính của khách đẩy sang.
//! - `auth_scim_group_members`: ai trong nhóm nào.
//! - `auth_scim_group_mappings`: nhóm X → role R ở nút N, khách tự cấu hình.
//! - `auth_scim_grants`: dòng membership (`auth_user_role.id`) do SCIM tạo — chỉ chúng bị SCIM gỡ,
//!   membership gán tay không bao giờ.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0045_scim_groups"
    }
}

fn unique(name: &str, table: &str, columns: &[&str]) -> IndexCreateStatement {
    let mut index = Index::create();
    index.name(name).table(Alias::new(table)).unique();
    for column in columns {
        index.col(Alias::new(*column));
    }
    index.to_owned()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("auth_scim_groups")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("id"))
                    .col(schema::string_len("display_name", 255))
                    // `displayName` viết thường: unique và lọc không phân biệt hoa thường.
                    .col(schema::string_len("display_name_key", 255))
                    .col(schema::string_len_null("external_id", 255))
                    .col(schema::string_len_null("external_key", 255))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("tenant_id").col("id"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(unique(
                "auth_scim_groups_name_unique",
                "auth_scim_groups",
                &["tenant_id", "display_name_key"],
            ))
            .await?;
        manager
            .create_index(unique(
                "auth_scim_groups_external_id_unique",
                "auth_scim_groups",
                &["tenant_id", "external_id"],
            ))
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("auth_scim_group_members")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("id"))
                    .col(schema::uuid("group_id"))
                    .col(schema::uuid("user_id"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .primary_key(Index::create().col("tenant_id").col("id"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(unique(
                "auth_scim_group_members_unique",
                "auth_scim_group_members",
                &["tenant_id", "group_id", "user_id"],
            ))
            .await?;
        // Nhóm của một người (bản `User`, đồng bộ membership).
        manager
            .create_index(
                Index::create()
                    .name("auth_scim_group_members_user_idx")
                    .table("auth_scim_group_members")
                    .col("tenant_id")
                    .col("user_id")
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("auth_scim_group_mappings")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("id"))
                    .col(schema::uuid("group_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::uuid("role_id"))
                    .col(schema::uuid("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .primary_key(Index::create().col("tenant_id").col("id"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(unique(
                "auth_scim_group_mappings_unique",
                "auth_scim_group_mappings",
                &["tenant_id", "group_id", "org_id", "role_id"],
            ))
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("auth_scim_grants")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("id"))
                    // Dòng `auth_user_role` SCIM đã tạo: SCIM chỉ gỡ đúng dòng này. Dòng bị thay
                    // (gán tay lại) thì dấu này hết hiệu lực.
                    .col(schema::uuid("user_role_id"))
                    .col(schema::uuid("user_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::uuid("role_id"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .primary_key(Index::create().col("tenant_id").col("id"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(unique(
                "auth_scim_grants_unique",
                "auth_scim_grants",
                &["tenant_id", "user_id", "org_id", "role_id"],
            ))
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            "auth_scim_grants",
            "auth_scim_group_mappings",
            "auth_scim_group_members",
            "auth_scim_groups",
        ] {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}
