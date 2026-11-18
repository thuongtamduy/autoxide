//! Migration của module `analytics`. Đã commit thì không sửa — thêm migration mới vào cuối [`all`].
//! Mọi bảng là cấp khách hàng (`tenant_id` đứng đầu mọi index).
//!
//! - `analytics_events`: sự kiện thô, giữ ngắn (`AUTOXIDE_ANALYTICS_RAW_RETENTION_DAYS`).
//! - `analytics_daily`, `analytics_funnel_daily`: số gộp theo ngày — báo cáo chỉ đọc hai bảng này.
//! - `analytics_day_keys`: khoá ngẫu nhiên của từng ngày cho khoá phiên; xoá khi ngày qua.
//! - `analytics_rollups`: ngày nào đã gộp xong (không cần gộp lại).

use autoxide_kernel::migration::prelude::*;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(CreateTables),
        // Bảng `analytics_jobs`: job gộp mỗi giờ và dọn mỗi ngày.
        autoxide_kernel::job::jobs_migration("m0002_create_jobs", "analytics"),
        // Bảng `analytics_throttle`: giới hạn số sự kiện theo khối mạng đã băm.
        autoxide_kernel::throttle::throttle_migration("m0003_create_throttle", "analytics"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0004_tenancy_helpers",
            vec![
                autoxide_kernel::job::tenant_table("analytics"),
                autoxide_kernel::throttle::tenant_table("analytics"),
            ],
        ),
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

struct CreateTables;

impl MigrationName for CreateTables {
    fn name(&self) -> &str {
        "m0001_create_analytics"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateTables {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("analytics_events")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    // Website: nút của cây tổ chức (gốc = cả khách). Chỉ giữ ID, không foreign key.
                    .col(schema::uuid("site_org_id"))
                    .col(schema::timestamp_with_time_zone("occurred_at"))
                    // Ngày UTC `YYYY-MM-DD`: gộp và dọn theo ngày bằng so sánh chuỗi, chạy giống
                    // nhau trên hai database.
                    .col(schema::string_len("day", 10))
                    .col(schema::string_len("type", 64))
                    .col(schema::string_len("session_key", 32))
                    // Chỉ khi người dùng đồng ý theo dõi theo người.
                    .col(schema::uuid_null("user_id"))
                    .col(schema::string_len("path", 512))
                    .col(schema::string_len_null("referrer_host", 255))
                    // Nguồn: `utm_source`, tên miền giới thiệu ngoài website, hay `direct`.
                    .col(schema::string_len("source", 255))
                    // Sự kiện đến từ ngoài website (không phải bấm link trong chính website).
                    .col(schema::boolean("landing"))
                    .col(schema::string_len_null("utm_source", 100))
                    .col(schema::string_len_null("utm_medium", 100))
                    .col(schema::string_len_null("utm_campaign", 100))
                    .col(schema::string_len_null("utm_term", 100))
                    .col(schema::string_len_null("utm_content", 100))
                    .col(schema::string_len_null("object", 200))
                    // JSON.
                    .col(schema::text_null("props"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "analytics_events",
            "analytics_events_time_idx",
            &["tenant_id", "occurred_at"],
            false,
        )
        .await?;
        index(
            manager,
            "analytics_events",
            "analytics_events_day_idx",
            &["tenant_id", "day"],
            false,
        )
        .await?;
        index(
            manager,
            "analytics_events",
            "analytics_events_user_idx",
            &["tenant_id", "user_id"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("analytics_daily")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("site_org_id"))
                    .col(schema::string_len("day", 10))
                    // `total`, `page`, `source`, `campaign`, `object`, `type`.
                    .col(schema::string_len("dimension", 16))
                    .col(schema::string_len("value", 512))
                    .col(schema::big_integer("views"))
                    .col(schema::big_integer("visitors"))
                    .col(schema::big_integer("events"))
                    .col(schema::big_integer("bounces"))
                    .to_owned(),
            )
            .await?;
        // Khoá của dòng, cũng là index của báo cáo: một website, một chiều, một khoảng ngày.
        index(
            manager,
            "analytics_daily",
            "analytics_daily_key",
            &["tenant_id", "site_org_id", "dimension", "day", "value"],
            true,
        )
        .await?;
        index(manager, "analytics_daily", "analytics_daily_day_idx", &["tenant_id", "day"], false)
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("analytics_funnel_daily")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("site_org_id"))
                    .col(schema::string_len("day", 10))
                    .col(schema::string_len("funnel", 64))
                    .col(schema::integer("step"))
                    .col(schema::string_len("step_key", 64))
                    .col(schema::big_integer("sessions"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "analytics_funnel_daily",
            "analytics_funnel_daily_key",
            &["tenant_id", "site_org_id", "funnel", "day", "step"],
            true,
        )
        .await?;
        index(
            manager,
            "analytics_funnel_daily",
            "analytics_funnel_daily_day_idx",
            &["tenant_id", "day"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("analytics_day_keys")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("day", 10))
                    .col(schema::string_len("key", 64))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .primary_key(Index::create().col("tenant_id").col("day"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("analytics_rollups")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("day", 10))
                    .col(schema::timestamp_with_time_zone("rolled_at"))
                    .primary_key(Index::create().col("tenant_id").col("day"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            "analytics_rollups",
            "analytics_day_keys",
            "analytics_funnel_daily",
            "analytics_daily",
            "analytics_events",
        ] {
            manager.drop_table(Table::drop().table(Alias::new(table)).to_owned()).await?;
        }
        Ok(())
    }
}
