//! Bảng hồ sơ khách, lịch sử mua và nhóm khách của module `customers` (bước 47). Mọi bảng cấp khách
//! hàng: `tenant_id` đứng đầu mọi index (bước 33); hồ sơ và nhóm theo nút có `org_id` (bước 34).
//!
//! Chỉ giữ ID của module khác (người dùng, đơn hàng) — không foreign key xuyên module. Tiền là số
//! nguyên đơn vị nhỏ nhất + mã tiền tệ; thời gian lưu UTC.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0001_create_customers"
    }
}

const TABLES: [&str; 5] = [
    "customers_group_members",
    "customers_groups",
    "customers_refund_facts",
    "customers_order_facts",
    "customers_profiles",
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
                    .table("customers_profiles")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    // Nút nơi khách đăng ký / mua lần đầu (quyền `unit` / `subtree` của nhân viên).
                    .col(schema::uuid("org_id"))
                    .col(schema::uuid_null("user_id"))
                    // HMAC-SHA256 của số E.164 bằng khoá riêng của khách (`SecretKey::phone_key`).
                    .col(schema::string_len_null("phone_key", 64))
                    .col(schema::string_len_null("phone", 32))
                    .col(schema::string_len_null("email", 320))
                    .col(schema::string_len_null("email_lower", 320))
                    .col(schema::string_len_null("full_name", 200))
                    // Tên chữ thường không dấu (`autoxide_kernel::search::fold`) để tìm.
                    .col(schema::string_len_null("full_name_search", 200))
                    .col(schema::date_null("birthday"))
                    .col(schema::string_len_null("gender", 16))
                    .col(schema::string_len_null("tier_code", 64))
                    .col(schema::string_len_null("currency", 3))
                    .col(schema::big_integer("total_spent_minor"))
                    .col(schema::big_integer("orders_count"))
                    .col(schema::timestamp_with_time_zone_null("first_order_at"))
                    .col(schema::timestamp_with_time_zone_null("last_order_at"))
                    // Trường tự định nghĩa (bước 51 P19), JSON.
                    .col(schema::text("custom_fields"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        let profiles = "customers_profiles";
        index(manager, profiles, "customers_profiles_user", &["tenant_id", "user_id"], true)
            .await?;
        index(manager, profiles, "customers_profiles_phone", &["tenant_id", "phone_key"], true)
            .await?;
        index(manager, profiles, "customers_profiles_org", &["tenant_id", "org_id", "id"], false)
            .await?;
        index(
            manager,
            profiles,
            "customers_profiles_name",
            &["tenant_id", "full_name_search"],
            false,
        )
        .await?;
        index(manager, profiles, "customers_profiles_email", &["tenant_id", "email_lower"], false)
            .await?;

        manager
            .create_table(
                Table::create()
                    .table("customers_order_facts")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("order_id"))
                    .col(schema::uuid("profile_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("number", 64))
                    .col(schema::string_len("channel", 64))
                    .col(schema::string_len("status", 32))
                    .col(schema::string_len("currency", 3))
                    // Tiền người mua thực trả cho hàng hoá (`goods_paid` của `orders`, D-226).
                    .col(schema::big_integer("net_minor"))
                    .col(schema::big_integer("refunded_minor"))
                    .col(schema::timestamp_with_time_zone("placed_at"))
                    .col(schema::timestamp_with_time_zone_null("completed_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("tenant_id").col("order_id"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "customers_order_facts",
            "customers_order_facts_profile",
            &["tenant_id", "profile_id", "placed_at"],
            false,
        )
        .await?;

        // Khoản hoàn đã tính (event `orders.refund_completed` nhận lại không trừ hai lần).
        manager
            .create_table(
                Table::create()
                    .table("customers_refund_facts")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("refund_id"))
                    .col(schema::uuid("order_id"))
                    .col(schema::big_integer("amount_minor"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .primary_key(Index::create().col("tenant_id").col("refund_id"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "customers_refund_facts",
            "customers_refund_facts_order",
            &["tenant_id", "order_id"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("customers_groups")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    // Nhóm của một nhánh; nút gốc = nhóm của cả khách.
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("name", 100))
                    .col(schema::string_len_null("description", 500))
                    // `manual` hay `rule`.
                    .col(schema::string_len("kind", 16))
                    // Quy tắc (JSON) của nhóm `rule`.
                    .col(schema::text_null("rule"))
                    // Mã nhóm khách cho bảng giá của `catalog`.
                    .col(schema::string_len_null("price_group", 64))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(manager, "customers_groups", "customers_groups_name", &["tenant_id", "name"], true)
            .await?;

        manager
            .create_table(
                Table::create()
                    .table("customers_group_members")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("group_id"))
                    .col(schema::uuid("profile_id"))
                    // `manual` (nhân viên thêm) hay `rule` (quy tắc khớp).
                    .col(schema::string_len("source", 16))
                    .col(schema::timestamp_with_time_zone("added_at"))
                    .primary_key(Index::create().col("tenant_id").col("group_id").col("profile_id"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "customers_group_members",
            "customers_group_members_profile",
            &["tenant_id", "profile_id"],
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
