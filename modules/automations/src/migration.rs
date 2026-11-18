//! Migration của module `automations`. Đã commit thì không sửa — thêm migration mới vào cuối [`all`].
//! Mọi bảng là cấp khách hàng; mọi unique / index bắt đầu bằng `tenant_id` (trừ index quét cả
//! database `…_sweep_idx` của lịch).

use autoxide_kernel::migration::prelude::*;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(CreateAutomations),
        autoxide_kernel::event::outbox_migration("m0002_create_events", "automations"),
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "automations"),
        autoxide_kernel::job::jobs_migration("m0004_create_jobs", "automations"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0005_tenancy_helpers",
            vec![
                autoxide_kernel::event::tenant_table("automations"),
                autoxide_kernel::job::tenant_table("automations"),
            ],
        ),
        autoxide_kernel::event::outbox_access_migration("m0006_event_access", "automations"),
    ]
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

struct CreateAutomations;

impl MigrationName for CreateAutomations {
    fn name(&self) -> &str {
        "m0001_create_automations"
    }
}

const TABLES: [&str; 5] = [
    "automations_metric_seen",
    "automations_metric_hours",
    "automations_run_keys",
    "automations_runs",
    "automations_rules",
];

#[async_trait::async_trait]
impl MigrationTrait for CreateAutomations {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Luật. Thân luật (kích hoạt, điều kiện, hành động) lưu JSON dạng chữ: chạy giống nhau trên
        // SQLite và Postgres; cột rời chỉ cho phần cần lọc bằng index.
        manager
            .create_table(
                Table::create()
                    .table("automations_rules")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("name", 200))
                    .col(schema::text_null("description"))
                    .col(schema::string_len("trigger_type", 16))
                    // Tên event của kích hoạt `event`; số liệu (`fact:…`, `summary:…`) của `metric`.
                    .col(schema::string_len_null("trigger_ref", 160))
                    .col(schema::text("trigger"))
                    .col(schema::text("conditions"))
                    .col(schema::text("actions"))
                    .col(schema::string_len("status", 24))
                    .col(schema::string_len_null("status_reason", 64))
                    .col(schema::string_len("timezone", 64))
                    .col(schema::uuid("run_as"))
                    .col(schema::uuid("created_by"))
                    .col(schema::string_len("created_by_kind", 16))
                    .col(schema::uuid("updated_by"))
                    .col(schema::integer("version"))
                    .col(schema::timestamp_with_time_zone_null("next_run_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        // Event đến: luật đang chạy của khách nghe đúng event đó.
        index(
            manager,
            "automations_rules",
            "automations_rules_trigger_idx",
            &["tenant_id", "trigger_type", "trigger_ref", "status"],
            false,
        )
        .await?;
        // Danh sách theo nhánh: `org_id IN (nhánh)` rồi sắp theo `id`.
        index(
            manager,
            "automations_rules",
            "automations_rules_org_idx",
            &["tenant_id", "org_id", "id"],
            false,
        )
        .await?;
        // Lịch quét cả database: luật tới hạn chạy theo lịch / kiểm số liệu.
        index(
            manager,
            "automations_rules",
            "automations_rules_due_sweep_idx",
            &["status", "next_run_at"],
            false,
        )
        .await?;

        // Lần chạy: bản chụp thân luật lúc kích hoạt (sửa luật không đổi lần chạy đang dở), dữ liệu
        // kích hoạt, nhật ký từng bước.
        manager
            .create_table(
                Table::create()
                    .table("automations_runs")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::uuid("rule_id"))
                    .col(schema::integer("rule_version"))
                    .col(schema::string_len("trigger_key", 200))
                    .col(schema::uuid_null("event_id"))
                    .col(schema::string_len_null("event_name", 128))
                    .col(schema::integer("depth"))
                    .col(schema::string_len("status", 24))
                    .col(schema::integer("cursor"))
                    .col(schema::text("context"))
                    .col(schema::text("definition"))
                    .col(schema::text("steps"))
                    .col(schema::string_len_null("error_code", 128))
                    .col(schema::text_null("error_message"))
                    .col(schema::integer_null("approval_step"))
                    .col(schema::uuid_null("decided_by"))
                    .col(schema::uuid_null("triggered_by"))
                    .col(schema::timestamp_with_time_zone_null("resume_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .col(schema::timestamp_with_time_zone_null("finished_at"))
                    .to_owned(),
            )
            .await?;
        // Lần chạy của một luật (danh sách, đếm số lần chạy trong giờ).
        index(
            manager,
            "automations_runs",
            "automations_runs_rule_idx",
            &["tenant_id", "rule_id", "created_at"],
            false,
        )
        .await?;
        // Danh sách theo trạng thái (hộp việc: chờ duyệt).
        index(
            manager,
            "automations_runs",
            "automations_runs_status_idx",
            &["tenant_id", "status", "id"],
            false,
        )
        .await?;
        index(
            manager,
            "automations_runs",
            "automations_runs_org_idx",
            &["tenant_id", "org_id", "id"],
            false,
        )
        .await?;
        // Dọn lần chạy cũ (quét cả database).
        index(
            manager,
            "automations_runs",
            "automations_runs_created_sweep_idx",
            &["created_at"],
            false,
        )
        .await?;

        // Khoá chống chạy trùng: một luật chạy một lần cho mỗi event (nhận lại event), mỗi khe lịch
        // hay mỗi kỳ số liệu.
        manager
            .create_table(
                Table::create()
                    .table("automations_run_keys")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("rule_id"))
                    .col(schema::string_len("trigger_key", 200))
                    .col(schema::uuid("run_id"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .primary_key(Index::create().col("tenant_id").col("rule_id").col("trigger_key"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "automations_run_keys",
            "automations_run_keys_created_sweep_idx",
            &["created_at"],
            false,
        )
        .await?;

        // Số liệu theo giờ UTC (cộng ra ngày theo múi giờ nào cũng đúng): tổng của số liệu báo cáo
        // (`fact:<loại>`) hay mẫu đọc của số liệu tóm tắt (`summary:<id>`). `currency` rỗng khi không
        // phải tiền.
        manager
            .create_table(
                Table::create()
                    .table("automations_metric_hours")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("source", 160))
                    .col(schema::timestamp_with_time_zone("hour"))
                    .col(schema::string_len("currency", 3))
                    .col(schema::big_integer("value"))
                    .col(schema::big_integer("samples"))
                    .primary_key(
                        Index::create().col("tenant_id").col("source").col("hour").col("currency"),
                    )
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "automations_metric_hours",
            "automations_metric_hours_sweep_idx",
            &["hour"],
            false,
        )
        .await?;
        // Event số liệu đã cộng (nhận lại event không cộng hai lần).
        manager
            .create_table(
                Table::create()
                    .table("automations_metric_seen")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("event_id"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .primary_key(Index::create().col("tenant_id").col("event_id"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "automations_metric_seen",
            "automations_metric_seen_created_sweep_idx",
            &["created_at"],
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
