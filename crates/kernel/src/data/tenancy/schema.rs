//! Bảng mặt điều khiển của phần đa khách (module nội bộ `kernel`, đặc tả 33 "Bảng dữ liệu"):
//! `kernel_placements`, `kernel_tenants`, `kernel_tenant_domains`. Không có `tenant_id` (loại C):
//! chúng mô tả các khách, không thuộc về khách nào. Mọi database có cùng schema (D-253 mục 2), nhưng
//! chỉ database điều khiển có dữ liệu.
//!
//! Migration gieo placement `main` (database điều khiển) và khách mặc định — dữ liệu của bản cài cũ
//! thuộc về khách này.

use crate::migration::prelude::*;
use crate::time::Timestamp;

use super::{DEFAULT_TENANT, MAIN_PLACEMENT};

pub(crate) struct CreateTenancy;

impl MigrationName for CreateTenancy {
    fn name(&self) -> &str {
        "m0003_create_tenancy"
    }
}

async fn index(
    manager: &SchemaManager<'_>,
    table: &'static str,
    name: &'static str,
    columns: &[&'static str],
    unique: bool,
) -> Result<(), DbErr> {
    let mut index = Index::create();
    index.name(name).table(Alias::new(table));
    for column in columns {
        index.col(Alias::new(*column));
    }
    if unique {
        index.unique();
    }
    manager.create_index(index.to_owned()).await
}

#[async_trait::async_trait]
impl MigrationTrait for CreateTenancy {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("kernel_placements"))
                    .col(schema::pk_uuid(Alias::new("id")))
                    .col(schema::string_len(Alias::new("name"), 63))
                    .col(schema::string_len(Alias::new("kind"), 16))
                    .col(schema::string_len(Alias::new("backend"), 16))
                    // URL kết nối, mã hoá (`sealed:…`); NULL = `DATABASE_URL`.
                    .col(schema::text_null(Alias::new("url_sealed")))
                    .col(schema::text_null(Alias::new("read_url_sealed")))
                    // Nơi lưu file riêng của khách `dedicated` (mã hoá); NULL = nơi lưu chung.
                    .col(schema::text_null(Alias::new("storage_sealed")))
                    .col(schema::string_len(Alias::new("schema_state"), 16))
                    .col(schema::timestamp_with_time_zone_null(Alias::new("schema_checked_at")))
                    .col(schema::timestamp_with_time_zone(Alias::new("created_at")))
                    .col(schema::timestamp_with_time_zone(Alias::new("updated_at")))
                    .to_owned(),
            )
            .await?;
        index(manager, "kernel_placements", "kernel_placements_name_unique", &["name"], true)
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Alias::new("kernel_tenants"))
                    .col(schema::pk_uuid(Alias::new("id")))
                    .col(schema::string_len(Alias::new("slug"), 63))
                    .col(schema::string_len(Alias::new("name"), 200))
                    .col(schema::string_len(Alias::new("status"), 16))
                    // Khách cha (bước 38); không foreign key.
                    .col(schema::uuid_null(Alias::new("parent_id")))
                    .col(schema::uuid(Alias::new("placement_id")))
                    .col(schema::timestamp_with_time_zone(Alias::new("created_at")))
                    .col(schema::timestamp_with_time_zone(Alias::new("updated_at")))
                    .to_owned(),
            )
            .await?;
        index(manager, "kernel_tenants", "kernel_tenants_slug_unique", &["slug"], true).await?;
        index(manager, "kernel_tenants", "kernel_tenants_parent_idx", &["parent_id"], false)
            .await?;
        index(manager, "kernel_tenants", "kernel_tenants_placement_idx", &["placement_id"], false)
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Alias::new("kernel_tenant_domains"))
                    .col(schema::pk_uuid(Alias::new("id")))
                    .col(schema::uuid(Alias::new("tenant_id")))
                    // Nút trong cây tổ chức (bước 34).
                    .col(schema::uuid_null(Alias::new("org_id")))
                    .col(schema::string_len(Alias::new("domain"), 253))
                    .col(schema::string_len(Alias::new("status"), 16))
                    .col(schema::string_len(Alias::new("verification_token"), 64))
                    .col(schema::timestamp_with_time_zone_null(Alias::new("verified_at")))
                    .col(schema::timestamp_with_time_zone_null(Alias::new("last_checked_at")))
                    .col(schema::string_len_null(Alias::new("check_error"), 200))
                    .col(schema::timestamp_with_time_zone(Alias::new("created_at")))
                    .col(schema::timestamp_with_time_zone(Alias::new("updated_at")))
                    .to_owned(),
            )
            .await?;
        // Một tên miền thuộc đúng một khách trên cả nền tảng.
        index(
            manager,
            "kernel_tenant_domains",
            "kernel_tenant_domains_domain_unique",
            &["domain"],
            true,
        )
        .await?;
        index(
            manager,
            "kernel_tenant_domains",
            "kernel_tenant_domains_tenant_idx",
            &["tenant_id", "id"],
            false,
        )
        .await?;
        index(
            manager,
            "kernel_tenant_domains",
            "kernel_tenant_domains_check_sweep_idx",
            &["status", "last_checked_at"],
            false,
        )
        .await?;

        // Gieo placement `main` và khách mặc định (ID cố định, D-253).
        let now = Timestamp::now();
        let placement = Query::insert()
            .into_table(Alias::new("kernel_placements"))
            .columns(
                ["id", "name", "kind", "backend", "schema_state", "created_at", "updated_at"]
                    .map(Alias::new),
            )
            .values_panic([
                MAIN_PLACEMENT.as_uuid().into(),
                "main".into(),
                "shared".into(),
                backend_name(manager).into(),
                "ready".into(),
                now.into(),
                now.into(),
            ])
            .to_owned();
        manager.exec_stmt(placement).await?;
        let tenant = Query::insert()
            .into_table(Alias::new("kernel_tenants"))
            .columns(
                ["id", "slug", "name", "status", "placement_id", "created_at", "updated_at"]
                    .map(Alias::new),
            )
            .values_panic([
                DEFAULT_TENANT.as_uuid().into(),
                "default".into(),
                "Default".into(),
                "active".into(),
                MAIN_PLACEMENT.as_uuid().into(),
                now.into(),
                now.into(),
            ])
            .to_owned();
        manager.exec_stmt(tenant).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in ["kernel_tenant_domains", "kernel_tenants", "kernel_placements"] {
            manager.drop_table(Table::drop().table(Alias::new(table)).to_owned()).await?;
        }
        Ok(())
    }
}

fn backend_name(manager: &SchemaManager<'_>) -> &'static str {
    match manager.get_database_backend() {
        sea_orm::DbBackend::Postgres => "postgres",
        _ => "sqlite",
    }
}
