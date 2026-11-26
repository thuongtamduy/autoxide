//! Migration của module `promotions`. Đã commit thì không sửa — thêm migration mới vào cuối [`all`].
//!
//! Bảng cấp khách hàng có `tenant_id`, unique và index bắt đầu bằng nó (bước 33). Phần trăm lưu
//! `integer` basis point (1% = 100); tiền `*_minor` (`big_integer`) + mã tiền tệ; JSON lưu trong
//! `text`; ngày theo múi giờ cửa hàng lưu chuỗi `YYYY-MM-DD` — chạy được trên cả SQLite và
//! Postgres.
//!
//! `promotions_legal_packs`, `promotions_legal_limits`, `promotions_notice_rules` là bảng **mặt
//! điều khiển** (D-198): dữ liệu luật dùng chung mọi khách, không có `tenant_id`.

use autoxide_kernel::migration::prelude::*;

mod m0009_list_indexes;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(CreatePromotions),
        // Bảng `promotions_events` (transactional outbox) và `promotions_jobs`, định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "promotions"),
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "promotions"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0004_tenancy_events",
            vec![autoxide_kernel::event::tenant_table("promotions")],
        ),
        autoxide_kernel::event::outbox_access_migration("k0001_event_access", "promotions"),
        autoxide_kernel::job::jobs_migration("m0005_create_jobs", "promotions"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0006_tenancy_jobs",
            vec![autoxide_kernel::job::tenant_table("promotions")],
        ),
        // Bảng `promotions_throttle`: giới hạn số lần thử mã nhận voucher.
        autoxide_kernel::throttle::throttle_migration("m0007_create_throttle", "promotions"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0008_tenancy_throttle",
            vec![autoxide_kernel::throttle::tenant_table("promotions")],
        ),
        Box::new(m0009_list_indexes::Migration),
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

/// Bảng cấp khách có khoá chính `id` (UUIDv7 sinh trong ứng dụng) và `tenant_id`.
fn table(name: &str) -> TableCreateStatement {
    Table::create()
        .table(Alias::new(name))
        .col(schema::pk_uuid("id"))
        .col(schema::uuid("tenant_id"))
        .to_owned()
}

/// Bảng con của một chương trình: `id`, `tenant_id`, `promotion_id`.
fn child(name: &str) -> TableCreateStatement {
    let mut table = table(name);
    table.col(schema::uuid("promotion_id"));
    table
}

struct CreatePromotions;

impl MigrationName for CreatePromotions {
    fn name(&self) -> &str {
        "m0001_create_promotions"
    }
}

/// Bảng của [`CreatePromotions`], theo thứ tự tạo (xoá theo thứ tự ngược).
const TABLES: [&str; 26] = [
    "promotions_promotions",
    "promotions_translations",
    "promotions_targets",
    "promotions_customer_groups",
    "promotions_channels",
    "promotions_conditions",
    "promotions_schedules",
    "promotions_tiers",
    "promotions_gift_options",
    "promotions_codes",
    "promotions_vouchers",
    "promotions_flash_items",
    "promotions_funding",
    "promotions_participations",
    "promotions_customer_usage",
    "promotions_daily_usage",
    "promotions_redemptions",
    "promotions_redemption_holds",
    "promotions_stats_daily",
    "promotions_revisions",
    "promotions_versions",
    "promotions_notices",
    "promotions_reminders",
    "promotions_legal_packs",
    "promotions_legal_limits",
    "promotions_notice_rules",
];

#[async_trait::async_trait]
impl MigrationTrait for CreatePromotions {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        create_programs(manager).await?;
        create_rules(manager).await?;
        create_access(manager).await?;
        create_counters(manager).await?;
        create_history(manager).await?;
        create_legal(manager).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for name in TABLES.iter().rev() {
            manager.drop_table(Table::drop().table(Alias::new(*name)).to_owned()).await?;
        }
        Ok(())
    }
}

/// Chương trình và bản dịch.
async fn create_programs(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    // Chương trình là luật (D-222). Mọi số tiền của chương trình cùng `currency`. `version`: số
    // phiên bản hiện tại (`promotions_versions`).
    let mut promotions = table("promotions_promotions");
    promotions
        .col(schema::uuid("org_id"))
        .col(schema::string_len("kind", 16))
        .col(schema::string_len("scope", 16))
        .col(schema::string_len("trigger", 16))
        .col(schema::string_len("status", 16))
        .col(schema::string_len("currency", 3))
        .col(schema::integer_null("value_bp"))
        .col(schema::big_integer_null("value_minor"))
        .col(schema::big_integer_null("min_order_minor"))
        .col(schema::integer_null("min_quantity"))
        .col(schema::big_integer_null("max_discount_minor"))
        .col(schema::integer_null("buy_quantity"))
        .col(schema::integer_null("get_quantity"))
        .col(schema::string_len_null("reward_selection", 16))
        .col(schema::integer_null("reward_value_bp"))
        .col(schema::integer_null("max_applications"))
        .col(schema::integer_null("bundle_quantity"))
        .col(schema::big_integer_null("bundle_price_minor"))
        .col(schema::integer_null("max_add_on_quantity"))
        .col(schema::integer_null("gift_quantity"))
        .col(schema::boolean("first_order_only"))
        .col(schema::timestamp_with_time_zone("starts_at"))
        .col(schema::timestamp_with_time_zone("ends_at"))
        .col(schema::integer("priority"))
        .col(schema::string_len("exclusivity", 16))
        .col(schema::string_len("combines_with", 64))
        .col(schema::boolean("stop_after"))
        .col(schema::big_integer_null("usage_limit"))
        .col(schema::big_integer("usage_count"))
        .col(schema::big_integer_null("per_customer_limit"))
        .col(schema::string_len("per_customer_period", 16))
        .col(schema::big_integer_null("daily_usage_limit"))
        .col(schema::big_integer_null("budget_minor"))
        .col(schema::big_integer("budget_used_minor"))
        .col(schema::boolean("return_usage_on_cancel"))
        .col(schema::string_len("gift_return_policy", 24))
        .col(schema::string_len("program_kind", 24))
        .col(schema::big_integer_null("voucher_quota"))
        .col(schema::big_integer("issued_count"))
        .col(schema::integer_null("per_user_claim_limit"))
        .col(schema::integer_null("voucher_valid_days"))
        .col(schema::boolean("claimable"))
        .col(schema::boolean("claim_requires_verified_phone"))
        .col(schema::boolean("public"))
        .col(schema::integer("version"))
        .col(schema::uuid_null("created_by"))
        .col(schema::timestamp_with_time_zone("created_at"))
        .col(schema::timestamp_with_time_zone("updated_at"))
        .col(schema::timestamp_with_time_zone_null("activated_at"));
    manager.create_table(promotions).await?;
    let t = "promotions_promotions";
    index(manager, t, "promotions_status_idx", &["tenant_id", "status", "starts_at"], false)
        .await?;
    // Job `promotions.tick` quét cả database (mọi khách) theo trạng thái và giờ.
    index(manager, t, "promotions_promotions_sweep_idx", &["status", "starts_at"], false).await?;
    index(manager, t, "promotions_org_idx", &["tenant_id", "org_id", "status", "id"], false)
        .await?;

    // Tiêu đề và điều kiện người mua đọc, theo ngôn ngữ.
    let mut translations = child("promotions_translations");
    translations
        .col(schema::string_len("locale", 16))
        .col(schema::string_len("title", 200))
        .col(schema::text("terms"));
    manager.create_table(translations).await?;
    index(
        manager,
        "promotions_translations",
        "promotions_translations_unique",
        &["tenant_id", "promotion_id", "locale"],
        true,
    )
    .await
}

/// Mục tiêu, nhóm khách, kênh, điều kiện, khung giờ, bậc, quà, flash sale, người trả tiền.
async fn create_rules(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let mut targets = child("promotions_targets");
    targets
        .col(schema::string_len("target_type", 16))
        .col(schema::uuid("target_id"))
        .col(schema::string_len("role", 16));
    manager.create_table(targets).await?;
    let t = "promotions_targets";
    index(manager, t, "promotions_targets_promotion_idx", &["tenant_id", "promotion_id"], false)
        .await?;
    index(
        manager,
        t,
        "promotions_targets_target_idx",
        &["tenant_id", "target_type", "target_id"],
        false,
    )
    .await?;

    let mut groups = child("promotions_customer_groups");
    groups.col(schema::uuid("group_id"));
    manager.create_table(groups).await?;
    index(
        manager,
        "promotions_customer_groups",
        "promotions_customer_groups_unique",
        &["tenant_id", "promotion_id", "group_id"],
        true,
    )
    .await?;

    let mut channels = child("promotions_channels");
    channels.col(schema::string_len("channel", 64));
    manager.create_table(channels).await?;
    index(
        manager,
        "promotions_channels",
        "promotions_channels_unique",
        &["tenant_id", "promotion_id", "channel"],
        true,
    )
    .await?;

    // Điều kiện mở rộng (D-222): `values` là mảng JSON; biên dịch thành struct khi nạp.
    let mut conditions = child("promotions_conditions");
    conditions
        .col(schema::string_len("kind", 32))
        .col(schema::string_len("operator", 8))
        .col(schema::text("values"));
    manager.create_table(conditions).await?;
    index(
        manager,
        "promotions_conditions",
        "promotions_conditions_promotion_idx",
        &["tenant_id", "promotion_id"],
        false,
    )
    .await?;

    // Khung giờ theo múi giờ cửa hàng: `weekdays` bitmask (bit 0 = thứ Hai), phút trong ngày.
    let mut schedules = child("promotions_schedules");
    schedules
        .col(schema::integer("weekdays"))
        .col(schema::integer("start_minute"))
        .col(schema::integer("end_minute"));
    manager.create_table(schedules).await?;
    index(
        manager,
        "promotions_schedules",
        "promotions_schedules_promotion_idx",
        &["tenant_id", "promotion_id"],
        false,
    )
    .await?;

    let mut tiers = child("promotions_tiers");
    tiers
        .col(schema::string_len("threshold_kind", 16))
        .col(schema::big_integer("threshold"))
        .col(schema::integer_null("value_bp"))
        .col(schema::big_integer_null("value_minor"));
    manager.create_table(tiers).await?;
    index(
        manager,
        "promotions_tiers",
        "promotions_tiers_promotion_idx",
        &["tenant_id", "promotion_id"],
        false,
    )
    .await?;

    let mut gifts = child("promotions_gift_options");
    gifts
        .col(schema::uuid("variant_id"))
        .col(schema::big_integer_null("stock_limit"))
        .col(schema::big_integer("quantity_used"));
    manager.create_table(gifts).await?;
    index(
        manager,
        "promotions_gift_options",
        "promotions_gift_options_unique",
        &["tenant_id", "promotion_id", "variant_id"],
        true,
    )
    .await?;

    // Giá / mức giảm theo biến thể của flash sale và món thưởng của `add_on`.
    let mut flash = child("promotions_flash_items");
    flash
        .col(schema::uuid("variant_id"))
        .col(schema::big_integer_null("sale_price_minor"))
        .col(schema::integer_null("value_bp"))
        .col(schema::big_integer_null("quantity_limit"))
        .col(schema::big_integer("quantity_used"))
        .col(schema::big_integer_null("per_customer_quantity_limit"));
    manager.create_table(flash).await?;
    index(
        manager,
        "promotions_flash_items",
        "promotions_flash_items_unique",
        &["tenant_id", "promotion_id", "variant_id"],
        true,
    )
    .await?;

    let mut funding = child("promotions_funding");
    funding
        .col(schema::string_len("payer_kind", 24))
        .col(schema::uuid_null("payer_org_id"))
        .col(schema::integer("share_bp"));
    manager.create_table(funding).await?;
    index(
        manager,
        "promotions_funding",
        "promotions_funding_promotion_idx",
        &["tenant_id", "promotion_id"],
        false,
    )
    .await?;

    let mut participations = child("promotions_participations");
    participations
        .col(schema::uuid("seller_org_id"))
        .col(schema::string_len("status", 16))
        .col(schema::text("items"))
        .col(schema::uuid_null("reviewed_by"))
        .col(schema::timestamp_with_time_zone_null("reviewed_at"))
        .col(schema::timestamp_with_time_zone("created_at"))
        .col(schema::timestamp_with_time_zone("updated_at"));
    manager.create_table(participations).await?;
    index(
        manager,
        "promotions_participations",
        "promotions_participations_unique",
        &["tenant_id", "promotion_id", "seller_org_id"],
        true,
    )
    .await
}

/// Mã và voucher.
async fn create_access(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    // `code_key`: chữ hoa, bỏ khoảng trắng — unique trong khách quyết định trùng mã.
    let mut codes = child("promotions_codes");
    codes
        .col(schema::string_len("code", 64))
        .col(schema::string_len("code_key", 64))
        .col(schema::string_len("purpose", 8))
        .col(schema::big_integer_null("usage_limit"))
        .col(schema::big_integer("usage_count"))
        .col(schema::timestamp_with_time_zone("created_at"));
    manager.create_table(codes).await?;
    let t = "promotions_codes";
    index(manager, t, "promotions_codes_key_unique", &["tenant_id", "code_key"], true).await?;
    index(
        manager,
        t,
        "promotions_codes_promotion_idx",
        &["tenant_id", "promotion_id", "id"],
        false,
    )
    .await?;

    // Voucher: quyền dùng một lần trong ví của một tài khoản. `holder_user_id` NULL khi tài khoản
    // đã xoá (voucher đã dùng giữ cho chứng từ).
    let mut vouchers = child("promotions_vouchers");
    vouchers
        .col(schema::uuid_null("holder_user_id"))
        .col(schema::string_len("status", 16))
        .col(schema::string_len("source", 24))
        .col(schema::string_len("source_ref", 200))
        .col(schema::timestamp_with_time_zone("valid_from"))
        .col(schema::timestamp_with_time_zone("expires_at"))
        .col(schema::timestamp_with_time_zone("issued_at"))
        .col(schema::uuid_null("used_order_id"))
        .col(schema::timestamp_with_time_zone("updated_at"));
    manager.create_table(vouchers).await?;
    let t = "promotions_vouchers";
    index(
        manager,
        t,
        "promotions_vouchers_source_unique",
        &["tenant_id", "promotion_id", "holder_user_id", "source", "source_ref"],
        true,
    )
    .await?;
    index(
        manager,
        t,
        "promotions_vouchers_holder_idx",
        &["tenant_id", "holder_user_id", "status", "expires_at"],
        false,
    )
    .await?;
    index(manager, t, "promotions_vouchers_sweep_idx", &["status", "expires_at"], false).await?;
    index(
        manager,
        t,
        "promotions_vouchers_expiry_idx",
        &["tenant_id", "status", "expires_at"],
        false,
    )
    .await
}

/// Bộ đếm và lượt giữ.
async fn create_counters(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    // Lượt mỗi khoá người mua: `subject` rỗng = cả chương trình, `<variant_id>` = suất flash sale
    // của biến thể; `period_key` = `lifetime`, ngày, tuần (`2026-W47`) hay tháng (`2026-11`).
    let mut usage = child("promotions_customer_usage");
    usage
        .col(schema::string_len("subject", 40))
        .col(schema::string_len("buyer_key", 128))
        .col(schema::string_len("period_key", 16))
        .col(schema::big_integer("used"));
    manager.create_table(usage).await?;
    index(
        manager,
        "promotions_customer_usage",
        "promotions_customer_usage_unique",
        &["tenant_id", "promotion_id", "subject", "buyer_key", "period_key"],
        true,
    )
    .await?;

    let mut daily = child("promotions_daily_usage");
    daily.col(schema::string_len("day", 10)).col(schema::big_integer("used"));
    manager.create_table(daily).await?;
    index(
        manager,
        "promotions_daily_usage",
        "promotions_daily_usage_unique",
        &["tenant_id", "promotion_id", "day"],
        true,
    )
    .await?;

    // Lượt giữ của một đơn. `request_fingerprint`: dấu của giỏ gửi tới (gọi lại cùng đơn khác
    // giỏ → `reference_reused`); `fingerprint`: dấu của kết quả.
    let mut redemptions = table("promotions_redemptions");
    redemptions
        .col(schema::uuid("order_id"))
        .col(schema::uuid("org_id"))
        .col(schema::uuid_null("buyer_id"))
        .col(schema::string_len("status", 16))
        .col(schema::string_len("fingerprint", 64))
        .col(schema::string_len("request_fingerprint", 64))
        .col(schema::text("evaluation"))
        .col(schema::big_integer("total_discount_minor"))
        .col(schema::big_integer("order_value_minor"))
        .col(schema::string_len("currency", 3))
        .col(schema::string_len("day", 10))
        .col(schema::timestamp_with_time_zone_null("expires_at"))
        .col(schema::timestamp_with_time_zone("created_at"))
        .col(schema::timestamp_with_time_zone("updated_at"));
    manager.create_table(redemptions).await?;
    let t = "promotions_redemptions";
    index(manager, t, "promotions_redemptions_sweep_idx", &["status", "expires_at"], false).await?;
    index(manager, t, "promotions_redemptions_order_unique", &["tenant_id", "order_id"], true)
        .await?;
    index(
        manager,
        t,
        "promotions_redemptions_expiry_idx",
        &["tenant_id", "status", "expires_at"],
        false,
    )
    .await?;

    // Mỗi bộ đếm một lần giữ đã cộng: trả lại đúng các dòng này.
    let mut holds = table("promotions_redemption_holds");
    holds
        .col(schema::uuid("redemption_id"))
        .col(schema::string_len("counter", 16))
        .col(schema::string_len("counter_id", 200))
        .col(schema::big_integer("quantity"));
    manager.create_table(holds).await?;
    index(
        manager,
        "promotions_redemption_holds",
        "promotions_redemption_holds_idx",
        &["tenant_id", "redemption_id"],
        false,
    )
    .await?;

    let mut stats = child("promotions_stats_daily");
    stats
        .col(schema::string_len("day", 10))
        .col(schema::big_integer("redemptions"))
        .col(schema::big_integer("discount_minor"))
        .col(schema::big_integer("order_value_minor"))
        .col(schema::big_integer("platform_funded_minor"))
        .col(schema::big_integer("seller_funded_minor"));
    manager.create_table(stats).await?;
    index(
        manager,
        "promotions_stats_daily",
        "promotions_stats_daily_unique",
        &["tenant_id", "promotion_id", "day"],
        true,
    )
    .await?;

    // Số phiên bản của bộ chương trình của khách: tăng trong cùng transaction với mọi lần đổi
    // chương trình; bộ nhớ `CompiledPromotions` nạp lại khi số đổi.
    manager
        .create_table(
            Table::create()
                .table(Alias::new("promotions_revisions"))
                .col(schema::uuid("tenant_id"))
                .col(schema::big_integer("revision"))
                .primary_key(Index::create().col("tenant_id"))
                .to_owned(),
        )
        .await
}

/// Lưu vết, thông báo theo nơi nhận, lời nhắc.
async fn create_history(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    // Chỉ thêm: mỗi lần đổi sau khi bật một dòng.
    let mut versions = child("promotions_versions");
    versions
        .col(schema::integer("version"))
        .col(schema::string_len("action", 32))
        .col(schema::text("snapshot"))
        .col(schema::uuid_null("changed_by"))
        .col(schema::timestamp_with_time_zone("changed_at"));
    manager.create_table(versions).await?;
    index(
        manager,
        "promotions_versions",
        "promotions_versions_unique",
        &["tenant_id", "promotion_id", "version"],
        true,
    )
    .await?;

    let mut notices = child("promotions_notices");
    notices
        .col(schema::string_len("authority_code", 32))
        .col(schema::string_len("status", 16))
        .col(schema::string_len_null("reference", 100))
        .col(schema::timestamp_with_time_zone_null("submitted_at"))
        .col(schema::uuid_null("submitted_by"))
        .col(schema::timestamp_with_time_zone("updated_at"));
    manager.create_table(notices).await?;
    index(
        manager,
        "promotions_notices",
        "promotions_notices_unique",
        &["tenant_id", "promotion_id", "authority_code"],
        true,
    )
    .await?;

    let mut reminders = child("promotions_reminders");
    reminders
        .col(schema::uuid("user_id"))
        .col(schema::string_len("channel", 16))
        .col(schema::uuid_null("consent_id"))
        .col(schema::timestamp_with_time_zone("remind_at"))
        .col(schema::timestamp_with_time_zone_null("sent_at"))
        .col(schema::timestamp_with_time_zone("created_at"));
    manager.create_table(reminders).await?;
    let t = "promotions_reminders";
    index(
        manager,
        t,
        "promotions_reminders_unique",
        &["tenant_id", "promotion_id", "user_id"],
        true,
    )
    .await?;
    index(manager, t, "promotions_reminders_due_idx", &["tenant_id", "sent_at", "remind_at"], false)
        .await
}

/// Dữ liệu luật dùng chung (mặt điều khiển, không `tenant_id`).
async fn create_legal(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let plain =
        |name: &str| Table::create().table(Alias::new(name)).col(schema::pk_uuid("id")).to_owned();
    let mut packs = plain("promotions_legal_packs");
    packs
        .col(schema::string_len("code", 32))
        .col(schema::big_integer("version"))
        .col(schema::string_len("checked_on", 10))
        .col(schema::text("source_note"))
        .col(schema::string_len("content_sha256", 64))
        .col(schema::timestamp_with_time_zone("imported_at"));
    manager.create_table(packs).await?;
    index(
        manager,
        "promotions_legal_packs",
        "promotions_legal_packs_unique",
        &["code", "version"],
        true,
    )
    .await?;

    // Ngày hiệu lực là ngày theo giờ Việt Nam, chuỗi `YYYY-MM-DD`; `effective_to` không gồm.
    let mut limits = plain("promotions_legal_limits");
    limits
        .col(schema::string_len("jurisdiction", 8))
        .col(schema::string_len("promotion_kind", 32))
        .col(schema::string_len("program_kind", 24))
        .col(schema::integer("max_bp"))
        .col(schema::integer_null("max_days_per_program"))
        .col(schema::integer_null("max_days_per_year"))
        .col(schema::string_len("effective_from", 10))
        .col(schema::string_len_null("effective_to", 10))
        .col(schema::text("legal_basis"))
        .col(schema::text("source_url"))
        .col(schema::text("note"))
        .col(schema::big_integer("pack_version"));
    manager.create_table(limits).await?;
    index(
        manager,
        "promotions_legal_limits",
        "promotions_legal_limits_unique",
        &["jurisdiction", "promotion_kind", "program_kind", "effective_from"],
        true,
    )
    .await?;

    let mut rules = plain("promotions_notice_rules");
    rules
        .col(schema::string_len("jurisdiction", 8))
        .col(schema::string_len("promotion_kind", 32))
        .col(schema::boolean("notice_required"))
        .col(schema::integer("days_before"))
        .col(schema::string_len("authority", 32))
        .col(schema::string_len("effective_from", 10))
        .col(schema::string_len_null("effective_to", 10))
        .col(schema::text("legal_basis"))
        .col(schema::text("note"))
        .col(schema::big_integer("pack_version"));
    manager.create_table(rules).await?;
    index(
        manager,
        "promotions_notice_rules",
        "promotions_notice_rules_unique",
        &["jurisdiction", "promotion_kind", "effective_from"],
        true,
    )
    .await
}
