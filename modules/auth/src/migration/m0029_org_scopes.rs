//! Quyền theo nút của cây tổ chức (bước 34 mục 2, D-304):
//!
//! - `auth_role_permission.scope`: độ sâu của permission trong role (`own` / `unit` / `subtree` /
//!   `tenant`); dòng cũ là `tenant` — mọi người giữ đúng quyền cũ.
//! - `auth_user_role.org_id`: membership = (người, nút, role). Dòng cũ gắn ở nút gốc (= ID của
//!   khách). Khoá unique thêm `org_id`: một người có cùng role ở hai nút.
//! - `auth_api_key.org_id`: hàng rào của API key (nút gốc = không rào); key cũ gắn ở gốc.
//!
//! Hai cột `org_id` là NULL được ở database, điền ngay trong migration và service luôn ghi: thêm
//! cột NOT NULL không `DEFAULT` cho bảng có dữ liệu cần dựng lại bảng, không đáng cho một giá trị
//! mà ứng dụng luôn đặt (D-304).

use autoxide_kernel::migration::prelude::*;
use sea_orm::{ConnectionTrait, Statement};

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0029_org_scopes"
    }
}

async fn execute(manager: &SchemaManager<'_>, sql: &str) -> Result<(), DbErr> {
    let connection = manager.get_connection();
    let backend = connection.get_database_backend();
    connection.execute_raw(Statement::from_string(backend, sql.to_owned())).await?;
    Ok(())
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("auth_role_permission")
                    .add_column(schema::string_len("scope", 16).default("tenant"))
                    .to_owned(),
            )
            .await?;
        for table in ["auth_user_role", "auth_api_key"] {
            manager
                .alter_table(
                    Table::alter().table(table).add_column(schema::uuid_null("org_id")).to_owned(),
                )
                .await?;
            execute(manager, &format!("UPDATE {table} SET org_id = tenant_id")).await?;
        }
        manager
            .drop_index(
                Index::drop().name("auth_user_role_unique").table("auth_user_role").to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_user_role_unique")
                    .table("auth_user_role")
                    .col("tenant_id")
                    .col("user_id")
                    .col("org_id")
                    .col("role_id")
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_user_role_org_idx")
                    .table("auth_user_role")
                    .col("tenant_id")
                    .col("org_id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Membership ở nút khác gốc không có chỗ trong schema cũ: từ chối thay vì làm mất quyền.
        let connection = manager.get_connection();
        let backend = connection.get_database_backend();
        let rows = connection
            .query_one_raw(Statement::from_string(
                backend,
                "SELECT COUNT(*) AS n FROM auth_user_role WHERE org_id <> tenant_id".to_owned(),
            ))
            .await?;
        let below_root: i64 = rows.map(|row| row.try_get("", "n")).transpose()?.unwrap_or(0);
        if below_root > 0 {
            return Err(DbErr::Migration(format!(
                "{below_root} memberships are below the root unit; remove them before reverting \
                 m0029_org_scopes"
            )));
        }
        manager
            .drop_index(
                Index::drop().name("auth_user_role_org_idx").table("auth_user_role").to_owned(),
            )
            .await?;
        manager
            .drop_index(
                Index::drop().name("auth_user_role_unique").table("auth_user_role").to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_user_role_unique")
                    .table("auth_user_role")
                    .col("tenant_id")
                    .col("user_id")
                    .col("role_id")
                    .unique()
                    .to_owned(),
            )
            .await?;
        for table in ["auth_api_key", "auth_user_role"] {
            manager
                .alter_table(Table::alter().table(table).drop_column("org_id").to_owned())
                .await?;
        }
        manager
            .alter_table(
                Table::alter().table("auth_role_permission").drop_column("scope").to_owned(),
            )
            .await
    }
}
