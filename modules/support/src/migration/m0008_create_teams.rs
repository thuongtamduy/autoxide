//! Nhóm nhân viên hỗ trợ của nút, thành viên (cờ "nhận việc", lần nhận cuối cho chia vòng) và quy
//! tắc tự phân công (bước 53 mục 2). Quy tắc là dữ liệu: theo loại / kênh của phiếu → nhóm.

use autoxide_kernel::migration::prelude::*;

use super::m0001_create_support::index;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0008_create_teams"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("support_teams")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("name", 100))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "support_teams",
            "support_teams_name_unique",
            &["tenant_id", "org_id", "name"],
            true,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("support_team_members")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("team_id"))
                    .col(schema::uuid("user_id"))
                    .col(schema::boolean("accepting"))
                    .col(schema::timestamp_with_time_zone_null("last_assigned_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .primary_key(Index::create().col("tenant_id").col("team_id").col("user_id"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "support_team_members",
            "support_team_members_user_idx",
            &["tenant_id", "user_id"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("support_routing_rules")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    // Điều kiện khớp: loại và kênh của phiếu (NULL: mọi giá trị).
                    .col(schema::string_len_null("category", 64))
                    .col(schema::string_len_null("channel", 32))
                    .col(schema::uuid("team_id"))
                    .col(schema::integer("position"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "support_routing_rules",
            "support_routing_rules_org_idx",
            &["tenant_id", "org_id", "position"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in ["support_routing_rules", "support_team_members", "support_teams"] {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}
