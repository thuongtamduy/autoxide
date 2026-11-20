//! Bảng điểm thưởng, hạng thành viên và voucher tặng của `customers` (bước 47 mục 3, 6). Quy tắc và
//! hạng là dữ liệu có ngày hiệu lực, chỉ thêm; sổ điểm chỉ thêm; số dư đổi bằng `rows::adjust`.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0006_loyalty"
    }
}

const TABLES: [&str; 7] = [
    "customers_voucher_programs",
    "customers_point_holds",
    "customers_points_entries",
    "customers_points_lots",
    "customers_points_balances",
    "customers_tiers",
    "customers_loyalty_rules",
];

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

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("customers_loyalty_rules")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    // Mỗi `earn_per_minor` tiền hàng đã trả được `points_per_unit` điểm.
                    .col(schema::big_integer("earn_per_minor"))
                    .col(schema::big_integer("points_per_unit"))
                    // Giá trị một điểm (đơn vị nhỏ nhất của `currency`).
                    .col(schema::big_integer("point_value_minor"))
                    .col(schema::string_len("currency", 3))
                    .col(schema::integer("points_valid_months"))
                    .col(schema::big_integer("min_redeem_points"))
                    // Trần đổi điểm theo phần vạn giá trị đơn (5000 = 50%).
                    .col(schema::integer("max_redeem_ratio_bp"))
                    .col(schema::timestamp_with_time_zone("effective_from"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "customers_loyalty_rules",
            "customers_loyalty_rules_from",
            &["tenant_id", "effective_from"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("customers_tiers")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("code", 64))
                    .col(schema::string_len("name", 100))
                    // Chi tiêu 12 tháng trượt tối thiểu để vào hạng.
                    .col(schema::big_integer("threshold_minor"))
                    .col(schema::string_len("currency", 3))
                    // Hệ số tích điểm, phần vạn (12000 = ×1,2).
                    .col(schema::integer("earn_multiplier_bp"))
                    .col(schema::integer("rank"))
                    .col(schema::timestamp_with_time_zone("effective_from"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "customers_tiers",
            "customers_tiers_code",
            &["tenant_id", "code", "effective_from"],
            true,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("customers_points_balances")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("profile_id"))
                    // Điểm dùng được (đã trừ phần đang giữ).
                    .col(schema::big_integer("balance"))
                    .col(schema::big_integer("held"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("tenant_id").col("profile_id"))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table("customers_points_lots")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("profile_id"))
                    .col(schema::big_integer("earned"))
                    // Còn lại (gồm phần đang giữ) và phần đang giữ cho đơn chưa xác nhận.
                    .col(schema::big_integer("remaining"))
                    .col(schema::big_integer("held"))
                    // Giá trị một điểm lúc tích: nghĩa vụ ghi theo giá trị này.
                    .col(schema::big_integer("value_minor_per_point"))
                    .col(schema::string_len("currency", 3))
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    .col(schema::string_len("source_ref", 200))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        let lots = "customers_points_lots";
        index(manager, lots, "customers_points_lots_source", &["tenant_id", "source_ref"], true)
            .await?;
        index(
            manager,
            lots,
            "customers_points_lots_profile",
            &["tenant_id", "profile_id", "expires_at", "id"],
            false,
        )
        .await?;
        index(manager, lots, "customers_points_lots_expiry", &["tenant_id", "expires_at"], false)
            .await?;

        manager
            .create_table(
                Table::create()
                    .table("customers_points_entries")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("profile_id"))
                    .col(schema::uuid_null("lot_id"))
                    // `lot_id` hay `''`: khoá chống ghi hai lần không vướng NULL.
                    .col(schema::string_len("lot_key", 36))
                    // `earn/hold/redeem/release/expire/adjust/restore`.
                    .col(schema::string_len("kind", 16))
                    .col(schema::big_integer("points"))
                    .col(schema::big_integer("value_minor"))
                    .col(schema::string_len("currency", 3))
                    .col(schema::string_len("reference", 200))
                    .col(schema::string_len_null("reason", 500))
                    .col(schema::uuid_null("actor_id"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        let entries = "customers_points_entries";
        index(
            manager,
            entries,
            "customers_points_entries_ref",
            &["tenant_id", "reference", "kind", "lot_key"],
            true,
        )
        .await?;
        index(
            manager,
            entries,
            "customers_points_entries_profile",
            &["tenant_id", "profile_id", "id"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("customers_point_holds")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("reference", 200))
                    .col(schema::uuid("profile_id"))
                    .col(schema::big_integer("points"))
                    // Tiền người mua được trừ (điểm × giá trị điểm hiện hành).
                    .col(schema::big_integer("amount_minor"))
                    .col(schema::string_len("currency", 3))
                    // `[{lot_id, points}]`.
                    .col(schema::text("lots"))
                    // `held/confirmed/released`.
                    .col(schema::string_len("status", 16))
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .primary_key(Index::create().col("tenant_id").col("reference"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "customers_point_holds",
            "customers_point_holds_status",
            &["tenant_id", "status", "expires_at"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("customers_voucher_programs")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    // `birthday`, `purchase` hay `points`.
                    .col(schema::string_len("kind", 16))
                    // Chương trình voucher của `promotions`.
                    .col(schema::uuid("promotion_id"))
                    // Đổi điểm: số điểm phải trả.
                    .col(schema::big_integer_null("points_cost"))
                    // Sau khi mua: tiền hàng tối thiểu của đơn.
                    .col(schema::big_integer_null("min_order_minor"))
                    .col(schema::boolean("active"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "customers_voucher_programs",
            "customers_voucher_programs_kind",
            &["tenant_id", "kind", "active"],
            false,
        )
        .await?;

        // Ngày sinh dạng `MM-DD` để job voucher sinh nhật tìm theo index trên cả hai database (tách
        // tháng / ngày của cột `date` mỗi database viết một kiểu).
        manager
            .alter_table(
                Table::alter()
                    .table("customers_profiles")
                    .add_column(schema::string_len_null("birthday_md", 5))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "customers_profiles",
            "customers_profiles_birthday",
            &["tenant_id", "birthday_md"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("customers_profiles_birthday")
                    .table(Alias::new("customers_profiles"))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter().table("customers_profiles").drop_column("birthday_md").to_owned(),
            )
            .await?;
        for table in TABLES {
            manager.drop_table(Table::drop().table(Alias::new(table)).to_owned()).await?;
        }
        Ok(())
    }
}
