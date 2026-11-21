//! Bảng role: `auth_role`, permission của role (`auth_role_permission`) và role của
//! người dùng (`auth_user_role`). Tạo sẵn role `admin` có mọi permission.

use autoxide_kernel::migration::prelude::*;
use autoxide_kernel::time::Timestamp;
use uuid::Uuid;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0003_create_roles"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("auth_role")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("name", 64))
                    .col(schema::string_len("description", 500))
                    // `true`: role có mọi permission, kể cả permission thêm sau. Chỉ
                    // role `admin` dựng sẵn mang cờ này; API không cho đặt.
                    .col(schema::boolean("all_permissions"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("auth_role_permission")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("role_id"))
                    .col(schema::string_len("permission", 200))
                    .foreign_key(
                        ForeignKey::create()
                            .name("auth_role_permission_role_fk")
                            .from("auth_role_permission", "role_id")
                            .to("auth_role", "id")
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("auth_user_role")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("user_id"))
                    .col(schema::uuid("role_id"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .foreign_key(
                        ForeignKey::create()
                            .name("auth_user_role_user_fk")
                            .from("auth_user_role", "user_id")
                            .to("auth_user", "id")
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("auth_user_role_role_fk")
                            .from("auth_user_role", "role_id")
                            .to("auth_role", "id")
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        for (name, table, columns) in [
            ("auth_role_name_unique", "auth_role", &["name"][..]),
            ("auth_role_permission_unique", "auth_role_permission", &["role_id", "permission"][..]),
            ("auth_user_role_unique", "auth_user_role", &["user_id", "role_id"][..]),
            // Tra "ai có role này" (đếm admin) mà không quét cả bảng.
            ("auth_user_role_role_idx", "auth_user_role", &["role_id"][..]),
        ] {
            let mut index = Index::create();
            index.name(name).table(table);
            for column in columns {
                index.col(*column);
            }
            if name.ends_with("_unique") {
                index.unique();
            }
            manager.create_index(index.to_owned()).await?;
        }

        let now = Timestamp::now();
        let mut admin = Query::insert();
        admin
            .into_table("auth_role")
            .columns(["id", "name", "description", "all_permissions", "created_at", "updated_at"])
            .values([
                Uuid::now_v7().into(),
                "admin".into(),
                "Every permission, including permissions added later. Built in: cannot be \
                 changed or deleted."
                    .into(),
                true.into(),
                now.into(),
                now.into(),
            ])
            .map_err(|error| DbErr::Custom(format!("cannot build the `admin` role: {error}")))?;
        manager.exec_stmt(admin.to_owned()).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in ["auth_user_role", "auth_role_permission", "auth_role"] {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}
