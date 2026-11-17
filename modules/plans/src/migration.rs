//! Migration của module `plans`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm migration mới
//! vào cuối [`all`].
//!
//! Mọi bảng `plans_*` là bảng **mặt điều khiển** (D-253): chỉ có dữ liệu ở database điều khiển,
//! cùng chỗ với sổ khách hàng. Bảng theo khách có cột `tenant_id` và mọi unique / index bắt đầu
//! bằng nó, nhưng không theo luật `tenant_id` của bảng cấp khách hàng (khai ở
//! `Module::control_plane_tables`).

use autoxide_kernel::migration::prelude::*;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(CreatePlans),
        autoxide_kernel::event::outbox_migration("m0002_create_events", "plans"),
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "plans"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0004_tenancy_events",
            vec![autoxide_kernel::event::tenant_table("plans")],
        ),
        autoxide_kernel::event::outbox_access_migration("k0001_event_access", "plans"),
        Box::new(CreateUsage),
        autoxide_kernel::job::jobs_migration("m0006_create_jobs", "plans"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0007_tenancy_jobs",
            vec![autoxide_kernel::job::tenant_table("plans")],
        ),
        Box::new(CreateBillingPeriods),
        Box::new(CreateBillingProfiles),
        Box::new(CreateProvisionings),
        Box::new(CreateSignups),
        // Bảng `plans_throttle` và `plans_antispam_nonces`: chống spam cho đăng ký công khai (D-349).
        autoxide_kernel::throttle::throttle_migration("m0012_create_throttle", "plans"),
        autoxide_kernel::antispam::antispam_migration("m0013_create_antispam_nonces", "plans"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0014_tenancy_signup_helpers",
            vec![
                autoxide_kernel::throttle::tenant_table("plans"),
                autoxide_kernel::antispam::tenant_table("plans"),
            ],
        ),
    ]
}

/// Tạo index `name` trên `table`.
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

/// Gói, giá, thuê bao, gói thêm, ghi đè (bước 35 phần B).
struct CreatePlans;

impl MigrationName for CreatePlans {
    fn name(&self) -> &str {
        "m0001_create_plans"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreatePlans {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("plans_plan")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("code", 64))
                    .col(schema::string_len("name", 200))
                    .col(schema::text("description"))
                    .col(schema::string_len("kind", 16))
                    .col(schema::string_len("status", 16))
                    .col(schema::boolean("is_default"))
                    .col(schema::integer("trial_days"))
                    .col(schema::integer("grace_days"))
                    .col(schema::integer("suspend_days"))
                    .col(schema::integer("retention_days"))
                    .col(schema::integer("renewal_lead_days"))
                    .col(schema::string_len("placement", 16))
                    .col(schema::boolean("requires_kyc"))
                    .col(schema::boolean("sample_data"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(manager, "plans_plan", "plans_plan_code_unique", &["code"], true).await?;
        for (table, column, size) in
            [("plans_plan_module", "module", 64), ("plans_plan_feature", "feature", 128)]
        {
            manager
                .create_table(
                    Table::create()
                        .table(table)
                        .col(schema::uuid("plan_id"))
                        .col(schema::string_len(column, size))
                        .primary_key(Index::create().col("plan_id").col(column))
                        .to_owned(),
                )
                .await?;
        }
        manager
            .create_table(
                Table::create()
                    .table("plans_plan_limit")
                    .col(schema::uuid("plan_id"))
                    .col(schema::string_len("limit_key", 128))
                    .col(schema::big_integer_null("max_value"))
                    .primary_key(Index::create().col("plan_id").col("limit_key"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("plans_price")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("plan_id"))
                    .col(schema::string_len("kind", 16))
                    .col(schema::string_len("interval", 16))
                    .col(schema::integer("interval_count"))
                    .col(schema::big_integer("amount_minor"))
                    .col(schema::string_len("amount_currency", 3))
                    .col(schema::string_len_null("limit_key", 128))
                    .col(schema::big_integer("included"))
                    .col(schema::string_len_null("tax_category", 64))
                    .col(schema::boolean("active"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(manager, "plans_price", "plans_price_plan_idx", &["plan_id"], false).await?;
        manager
            .create_table(
                Table::create()
                    .table("plans_subscription")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("plan_id"))
                    .col(schema::uuid_null("price_id"))
                    .col(schema::string_len("status", 16))
                    .col(schema::timestamp_with_time_zone("current_period_start"))
                    .col(schema::timestamp_with_time_zone("current_period_end"))
                    .col(schema::timestamp_with_time_zone_null("trial_ends_at"))
                    .col(schema::timestamp_with_time_zone_null("grace_ends_at"))
                    .col(schema::timestamp_with_time_zone_null("suspended_at"))
                    .col(schema::timestamp_with_time_zone_null("canceled_at"))
                    .col(schema::timestamp_with_time_zone_null("delete_after"))
                    .col(schema::boolean("cancel_at_period_end"))
                    .col(schema::uuid_null("scheduled_plan_id"))
                    .col(schema::uuid_null("scheduled_price_id"))
                    .col(schema::boolean("trial_used"))
                    .col(schema::string_len_null("billing_email", 254))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "plans_subscription",
            "plans_subscription_tenant_unique",
            &["tenant_id"],
            true,
        )
        .await?;
        index(
            manager,
            "plans_subscription",
            "plans_subscription_due_idx",
            &["status", "current_period_end"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("plans_subscription_addon")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("plan_id"))
                    .col(schema::integer("quantity"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .primary_key(Index::create().col("tenant_id").col("plan_id"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("plans_override")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("kind", 16))
                    .col(schema::string_len("key", 128))
                    .col(schema::boolean_null("enabled"))
                    .col(schema::big_integer_null("max_value"))
                    .col(schema::string_len("reason", 500))
                    .col(schema::timestamp_with_time_zone_null("expires_at"))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "plans_override",
            "plans_override_key_unique",
            &["tenant_id", "kind", "key"],
            true,
        )
        .await?;
        // Một dòng cố định: khoá dòng này khi đổi gói mặc định, hai request không cùng đặt hai gói.
        manager
            .create_table(
                Table::create()
                    .table("plans_settings")
                    .col(schema::integer("id").primary_key())
                    .col(schema::timestamp_with_time_zone_null("updated_at"))
                    .to_owned(),
            )
            .await?;
        let seed = Query::insert()
            .into_table(Alias::new("plans_settings"))
            .columns([Alias::new("id")])
            .values_panic([1.into()])
            .to_owned();
        manager.exec_stmt(seed).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            "plans_settings",
            "plans_override",
            "plans_subscription_addon",
            "plans_subscription",
            "plans_price",
            "plans_plan_limit",
            "plans_plan_feature",
            "plans_plan_module",
            "plans_plan",
        ] {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}

/// Lượng dùng, chỗ giữ, sổ lượng dùng theo kỳ (bước 35 phần D).
struct CreateUsage;

impl MigrationName for CreateUsage {
    fn name(&self) -> &str {
        "m0005_create_usage"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateUsage {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("plans_usage")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("limit_key", 128))
                    // `0000-00-00` cho hạn mức `count` / `amount`; ngày đầu kỳ cho `metered`.
                    .col(schema::string_len("period_start", 10))
                    .col(schema::big_integer("used"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(
                        Index::create().col("tenant_id").col("limit_key").col("period_start"),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("plans_reservation")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("limit_key", 128))
                    .col(schema::string_len("reference", 200))
                    .col(schema::big_integer("amount"))
                    .col(schema::string_len("status", 16))
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "plans_reservation",
            "plans_reservation_reference_unique",
            &["tenant_id", "limit_key", "reference"],
            true,
        )
        .await?;
        index(
            manager,
            "plans_reservation",
            "plans_reservation_due_idx",
            &["status", "expires_at"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("plans_usage_record")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("limit_key", 128))
                    .col(schema::string_len("period_start", 10))
                    .col(schema::big_integer("quantity"))
                    .col(schema::string_len("reference", 200))
                    .col(schema::timestamp_with_time_zone("recorded_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "plans_usage_record",
            "plans_usage_record_reference_unique",
            &["tenant_id", "limit_key", "reference"],
            true,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in ["plans_usage_record", "plans_reservation", "plans_usage"] {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}

/// Kỳ thanh toán của thuê bao (bước 35 phần E; cột hoá đơn / cổng thanh toán dùng ở mục 4).
struct CreateBillingPeriods;

impl MigrationName for CreateBillingPeriods {
    fn name(&self) -> &str {
        "m0008_create_billing_periods"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateBillingPeriods {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("plans_billing_period")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("subscription_id"))
                    // `initial`, `renewal`, `proration`, `metered`.
                    .col(schema::string_len("kind", 16))
                    .col(schema::uuid("plan_id"))
                    .col(schema::uuid_null("price_id"))
                    .col(schema::timestamp_with_time_zone("period_start"))
                    .col(schema::timestamp_with_time_zone("period_end"))
                    .col(schema::big_integer("amount_minor"))
                    .col(schema::string_len("amount_currency", 3))
                    .col(schema::string_len("reference", 200))
                    .col(schema::uuid_null("invoice_id"))
                    .col(schema::string_len_null("invoice_number", 64))
                    // `open`, `paid`, `void`.
                    .col(schema::string_len("status", 16))
                    .col(schema::timestamp_with_time_zone("due_at"))
                    .col(schema::timestamp_with_time_zone_null("paid_at"))
                    .col(schema::string_len_null("provider_reference", 200))
                    // Mốc nhắc đã gửi (JSON), để không gửi trùng.
                    .col(schema::text("reminders"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "plans_billing_period",
            "plans_billing_period_reference_unique",
            &["tenant_id", "reference"],
            true,
        )
        .await?;
        index(
            manager,
            "plans_billing_period",
            "plans_billing_period_due_idx",
            &["status", "due_at"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("plans_billing_period").to_owned()).await
    }
}

/// Hồ sơ xuất hoá đơn của khách (bước 35 mục 4); địa chỉ theo quy ước `address_*` của `address`.
struct CreateBillingProfiles;

impl MigrationName for CreateBillingProfiles {
    fn name(&self) -> &str {
        "m0009_create_billing_profiles"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateBillingProfiles {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("plans_billing_profile")
                    .col(schema::pk_uuid("tenant_id"))
                    .col(schema::string_len("legal_name", 200))
                    .col(schema::string_len_null("tax_code", 14))
                    .col(schema::string_len("email", 254))
                    .col(schema::string_len_null("phone", 20))
                    .col(schema::string_len_null("address_country", 2))
                    .col(schema::string_len_null("address_province_code", 10))
                    .col(schema::string_len_null("address_province_name", 200))
                    .col(schema::string_len_null("address_commune_code", 10))
                    .col(schema::string_len_null("address_commune_name", 200))
                    .col(schema::string_len_null("address_line", 300))
                    .col(schema::uuid_null("updated_by"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("plans_billing_profile").to_owned()).await
    }
}

/// Lượt cấp phát khách mới theo gói (bước 35 mục 5).
struct CreateProvisionings;

impl MigrationName for CreateProvisionings {
    fn name(&self) -> &str {
        "m0010_create_provisionings"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateProvisionings {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("plans_provisioning")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid_null("tenant_id"))
                    .col(schema::string_len("slug", 63))
                    .col(schema::string_len("tenant_name", 200))
                    .col(schema::uuid_null("parent_tenant_id"))
                    .col(schema::uuid("plan_id"))
                    .col(schema::uuid_null("price_id"))
                    .col(schema::string_len_null("placement", 63))
                    .col(schema::string_len("admin_email", 254))
                    .col(schema::string_len_null("admin_name", 200))
                    .col(schema::boolean("sample_data"))
                    .col(schema::boolean("trial"))
                    // `created`, `tenant_created`, `subscribed`, `seeded`, `invited`, `done`.
                    .col(schema::string_len("step", 16))
                    // `pending`, `running`, `failed`, `done`.
                    .col(schema::string_len("status", 16))
                    .col(schema::string_len_null("error_code", 100))
                    .col(schema::text_null("error"))
                    .col(schema::text("steps"))
                    .col(schema::uuid_null("requested_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(manager, "plans_provisioning", "plans_provisioning_slug_unique", &["slug"], true)
            .await?;
        index(
            manager,
            "plans_provisioning",
            "plans_provisioning_status_idx",
            &["status", "id"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("plans_provisioning").to_owned()).await
    }
}

/// Khách tự đăng ký (bước 35 mục 5, D-349): cờ `self_signup` của gói và bảng `plans_signup`.
struct CreateSignups;

impl MigrationName for CreateSignups {
    fn name(&self) -> &str {
        "m0011_create_signups"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateSignups {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("plans_plan")
                    .add_column(schema::boolean("self_signup").default(false))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("plans_signup")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("email", 254))
                    .col(schema::string_len_null("admin_name", 200))
                    .col(schema::string_len("tenant_name", 200))
                    .col(schema::string_len("slug", 63))
                    .col(schema::uuid("plan_id"))
                    .col(schema::uuid_null("price_id"))
                    // `individual`, `household_business`, `organization` (định danh khi gói cần).
                    .col(schema::string_len("party_type", 32))
                    .col(schema::string_len_null("locale", 16))
                    // `pending_email`, `awaiting_kyc`, `provisioning`, `completed`, `rejected`,
                    // `expired`.
                    .col(schema::string_len("status", 16))
                    // SHA-256 của token trong mail; token thật không lưu.
                    .col(schema::string_len("token_hash", 64))
                    // JSON: phiên bản văn bản pháp lý đã chấp nhận lúc xác nhận email.
                    .col(schema::text_null("terms"))
                    .col(schema::uuid_null("kyc_profile_id"))
                    .col(schema::uuid_null("provisioning_id"))
                    .col(schema::uuid_null("tenant_id"))
                    .col(schema::string_len_null("reason", 100))
                    .col(schema::timestamp_with_time_zone_null("confirmed_at"))
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(manager, "plans_signup", "plans_signup_token_unique", &["token_hash"], true).await?;
        index(manager, "plans_signup", "plans_signup_email_idx", &["email", "status"], false)
            .await?;
        index(manager, "plans_signup", "plans_signup_status_idx", &["status", "expires_at"], false)
            .await?;
        index(manager, "plans_signup", "plans_signup_list_idx", &["status", "id"], false).await?;
        index(manager, "plans_signup", "plans_signup_provisioning_idx", &["provisioning_id"], false)
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("plans_signup").to_owned()).await?;
        manager
            .alter_table(Table::alter().table("plans_plan").drop_column("self_signup").to_owned())
            .await
    }
}
