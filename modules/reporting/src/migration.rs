//! Migration của module `reporting`. Đã commit thì không sửa — thêm migration mới vào cuối [`all`].
//! Mọi bảng là cấp khách hàng; mọi unique / index bắt đầu bằng `tenant_id` (trừ index quét cả
//! database `…_sweep_idx`).

use autoxide_kernel::migration::prelude::*;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(CreateReporting),
        autoxide_kernel::job::jobs_migration("m0002_create_jobs", "reporting"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0003_tenancy_jobs",
            vec![autoxide_kernel::job::tenant_table("reporting")],
        ),
        Box::new(CreateRehomes),
    ]
}

/// Nhật ký dời kho báo cáo khi tách / gộp khách (bước 38).
struct CreateRehomes;

impl MigrationName for CreateRehomes {
    fn name(&self) -> &str {
        "m0004_create_rehomes"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateRehomes {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("reporting_rehomes")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("relocation_id"))
                    .col(schema::uuid("from_root"))
                    .col(schema::uuid("to_root"))
                    .col(schema::string_len("status", 16))
                    .col(schema::big_integer("moved_facts"))
                    .col(schema::timestamp_with_time_zone("started_at"))
                    .col(schema::timestamp_with_time_zone_null("finished_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "reporting_rehomes",
            "reporting_rehomes_started_idx",
            &["tenant_id", "started_at"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("reporting_rehomes").to_owned()).await
    }
}

async fn index(
    manager: &SchemaManager<'_>,
    table: &str,
    name: &str,
    columns: &[&str],
    unique: bool,
) -> Result<(), DbErr> {
    let mut index = Index::create();
    index.name(name).table(Alias::new(table));
    if unique {
        index.unique();
    }
    for column in columns {
        index.col(Alias::new(*column));
    }
    manager.create_index(index.to_owned()).await
}

struct CreateReporting;

impl MigrationName for CreateReporting {
    fn name(&self) -> &str {
        "m0001_create_reporting"
    }
}

const TABLES: [&str; 4] =
    ["reporting_daily", "reporting_fact_dimensions", "reporting_facts", "reporting_local_facts"];

#[async_trait::async_trait]
impl MigrationTrait for CreateReporting {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("reporting_local_facts")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("source_event_id"))
                    .col(schema::integer("fact_index"))
                    .col(schema::string_len("kind", 64))
                    .col(schema::uuid("org_id"))
                    .col(schema::timestamp_with_time_zone("occurred_at"))
                    .col(schema::date("local_day"))
                    .col(schema::big_integer_null("amount_minor"))
                    .col(schema::string_len_null("amount_currency", 3))
                    .col(schema::big_integer_null("quantity"))
                    .col(schema::text("dimensions"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone_null("forwarded_at"))
                    .to_owned(),
            )
            .await?;
        // Nhận lại event không tạo bản trùng (D-054).
        index(
            manager,
            "reporting_local_facts",
            "reporting_local_facts_event_idx",
            &["tenant_id", "source_event_id", "fact_index"],
            true,
        )
        .await?;
        // Đẩy theo khách: số liệu chưa đẩy theo `id`.
        index(
            manager,
            "reporting_local_facts",
            "reporting_local_facts_pending_idx",
            &["tenant_id", "forwarded_at", "id"],
            false,
        )
        .await?;
        // Lịch dọn quét cả database theo lúc đẩy.
        index(
            manager,
            "reporting_local_facts",
            "reporting_local_facts_forwarded_sweep_idx",
            &["forwarded_at"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("reporting_facts")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("source_tenant_id"))
                    .col(schema::string_len("kind", 64))
                    .col(schema::uuid("org_id"))
                    .col(schema::timestamp_with_time_zone("occurred_at"))
                    .col(schema::date("local_day"))
                    .col(schema::big_integer_null("amount_minor"))
                    .col(schema::string_len_null("amount_currency", 3))
                    .col(schema::big_integer_null("quantity"))
                    .col(schema::text("dimensions"))
                    .col(schema::timestamp_with_time_zone("received_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "reporting_facts",
            "reporting_facts_source_idx",
            &["tenant_id", "source_tenant_id", "id"],
            true,
        )
        .await?;
        index(
            manager,
            "reporting_facts",
            "reporting_facts_kind_idx",
            &["tenant_id", "kind", "local_day"],
            false,
        )
        .await?;
        index(
            manager,
            "reporting_facts",
            "reporting_facts_source_kind_idx",
            &["tenant_id", "source_tenant_id", "kind", "local_day"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("reporting_fact_dimensions")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("fact_id"))
                    .col(schema::string_len("name", 64))
                    .col(schema::string_len("value", 100))
                    .primary_key(Index::create().col("tenant_id").col("fact_id").col("name"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "reporting_fact_dimensions",
            "reporting_fact_dimensions_value_idx",
            &["tenant_id", "name", "value", "fact_id"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("reporting_daily")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("source_tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("kind", 64))
                    .col(schema::date("local_day"))
                    .col(schema::string_len("currency", 3))
                    .col(schema::big_integer("amount_minor"))
                    .col(schema::big_integer("quantity"))
                    .col(schema::big_integer("fact_count"))
                    .primary_key(
                        Index::create()
                            .col("tenant_id")
                            .col("source_tenant_id")
                            .col("org_id")
                            .col("kind")
                            .col("local_day")
                            .col("currency"),
                    )
                    .to_owned(),
            )
            .await?;
        // Tổng theo loại và ngày của cả cây (mọi khách nguồn).
        index(
            manager,
            "reporting_daily",
            "reporting_daily_kind_idx",
            &["tenant_id", "kind", "local_day"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in TABLES {
            manager.drop_table(Table::drop().table(Alias::new(table)).to_owned()).await?;
        }
        Ok(())
    }
}
