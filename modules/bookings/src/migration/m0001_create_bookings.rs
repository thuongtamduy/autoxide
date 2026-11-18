//! Bảng của module `bookings` (bước 60, D-20261118-step60-bookings). Mọi bảng cấp khách hàng:
//! `tenant_id` đứng đầu mọi index (bước 33); dữ liệu theo nút có `org_id` (bước 34).
//!
//! Chỉ giữ ID của module khác (người dùng, thanh toán của `ledger`) — không foreign key xuyên module.
//! Thời gian lưu UTC; lịch làm việc lặp theo thứ lưu phút trong ngày theo giờ địa phương của nút.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0001_create_bookings"
    }
}

const TABLES: [&str; 9] = [
    "bookings_guest_challenges",
    "bookings_slots",
    "bookings_sessions",
    "bookings_bookings",
    "bookings_exceptions",
    "bookings_schedules",
    "bookings_service_resources",
    "bookings_resources",
    "bookings_services",
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
                    .table("bookings_services")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("name", 200))
                    // Tên theo ngôn ngữ khác, JSON `{"en": "…"}`.
                    .col(schema::text("names"))
                    .col(schema::string_len("description", 2000))
                    .col(schema::integer("duration_minutes"))
                    .col(schema::integer("buffer_before_minutes"))
                    .col(schema::integer("buffer_after_minutes"))
                    .col(schema::big_integer("price_minor"))
                    .col(schema::string_len("currency", 3))
                    // `none`, `percent`, `amount`.
                    .col(schema::string_len("deposit_kind", 16))
                    .col(schema::big_integer("deposit_value"))
                    .col(schema::integer("capacity"))
                    .col(schema::boolean("active"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "bookings_services",
            "bookings_services_org_idx",
            &["tenant_id", "org_id", "id"],
            false,
        )
        .await?;
        index(
            manager,
            "bookings_services",
            "bookings_services_list_idx",
            &["tenant_id", "id"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("bookings_resources")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    // `staff`, `room`, `equipment`.
                    .col(schema::string_len("kind", 16))
                    .col(schema::string_len("name", 200))
                    // Nhân viên: người dùng của bước 34 (chỉ ID).
                    .col(schema::uuid_null("user_id"))
                    .col(schema::boolean("active"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "bookings_resources",
            "bookings_resources_org_idx",
            &["tenant_id", "org_id", "id"],
            false,
        )
        .await?;
        index(
            manager,
            "bookings_resources",
            "bookings_resources_list_idx",
            &["tenant_id", "id"],
            false,
        )
        .await?;
        index(
            manager,
            "bookings_resources",
            "bookings_resources_user_idx",
            &["tenant_id", "user_id"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("bookings_service_resources")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("service_id"))
                    .col(schema::uuid("resource_id"))
                    // `performer` (nhân viên làm được, chọn một) hay `required` (phòng, máy: cần cả).
                    .col(schema::string_len("role", 16))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "bookings_service_resources",
            "bookings_service_resources_unique",
            &["tenant_id", "service_id", "resource_id"],
            true,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("bookings_schedules")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("resource_id"))
                    // ISO: 1 = thứ Hai … 7 = Chủ nhật.
                    .col(schema::small_integer("weekday"))
                    // Phút trong ngày, giờ địa phương của nút: [start, end).
                    .col(schema::integer("start_minute"))
                    .col(schema::integer("end_minute"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "bookings_schedules",
            "bookings_schedules_resource_idx",
            &["tenant_id", "resource_id", "weekday"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("bookings_exceptions")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("resource_id"))
                    // `closed` (nghỉ) hay `open` (ca thêm).
                    .col(schema::string_len("kind", 16))
                    .col(schema::timestamp_with_time_zone("starts_at"))
                    .col(schema::timestamp_with_time_zone("ends_at"))
                    .col(schema::string_len("reason", 200))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "bookings_exceptions",
            "bookings_exceptions_resource_idx",
            &["tenant_id", "resource_id", "starts_at"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("bookings_bookings")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::uuid("service_id"))
                    .col(schema::uuid("resource_id"))
                    .col(schema::uuid_null("session_id"))
                    .col(schema::integer("seats"))
                    .col(schema::timestamp_with_time_zone("starts_at"))
                    .col(schema::timestamp_with_time_zone("ends_at"))
                    // `held`, `pending`, `confirmed`, `checked_in`, `completed`, `cancelled`,
                    // `no_show`, `expired`.
                    .col(schema::string_len("status", 16))
                    .col(schema::uuid_null("customer_user_id"))
                    .col(schema::string_len("customer_name", 200))
                    .col(schema::string_len_null("customer_phone", 20))
                    .col(schema::string_len_null("customer_email", 254))
                    .col(schema::string_len("note", 500))
                    .col(schema::string_len_null("locale", 35))
                    .col(schema::big_integer("price_minor"))
                    .col(schema::string_len("currency", 3))
                    .col(schema::big_integer("deposit_minor"))
                    // `none`, `pending`, `paid`, `refund_pending`, `refunded`, `retained`.
                    .col(schema::string_len("deposit_status", 16))
                    .col(schema::big_integer("refund_minor"))
                    .col(schema::uuid_null("payment_id"))
                    .col(schema::timestamp_with_time_zone_null("hold_expires_at"))
                    .col(schema::timestamp_with_time_zone_null("reminded_24h_at"))
                    .col(schema::timestamp_with_time_zone_null("reminded_2h_at"))
                    .col(schema::timestamp_with_time_zone_null("cancelled_at"))
                    .col(schema::string_len_null("cancel_reason", 200))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "bookings_bookings",
            "bookings_bookings_org_idx",
            &["tenant_id", "org_id", "id"],
            false,
        )
        .await?;
        index(
            manager,
            "bookings_bookings",
            "bookings_bookings_list_idx",
            &["tenant_id", "id"],
            false,
        )
        .await?;
        index(
            manager,
            "bookings_bookings",
            "bookings_bookings_resource_idx",
            &["tenant_id", "resource_id", "starts_at"],
            false,
        )
        .await?;
        index(
            manager,
            "bookings_bookings",
            "bookings_bookings_customer_idx",
            &["tenant_id", "customer_user_id", "id"],
            false,
        )
        .await?;
        // Job giữ tạm hết hạn và nhắc lịch quét cả database theo trạng thái + giờ.
        index(
            manager,
            "bookings_bookings",
            "bookings_bookings_hold_sweep_idx",
            &["status", "hold_expires_at"],
            false,
        )
        .await?;
        index(
            manager,
            "bookings_bookings",
            "bookings_bookings_start_sweep_idx",
            &["status", "starts_at"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("bookings_sessions")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("service_id"))
                    .col(schema::uuid("resource_id"))
                    .col(schema::timestamp_with_time_zone("starts_at"))
                    .col(schema::timestamp_with_time_zone("ends_at"))
                    .col(schema::integer("capacity"))
                    .col(schema::integer("booked"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "bookings_sessions",
            "bookings_sessions_unique",
            &["tenant_id", "service_id", "resource_id", "starts_at"],
            true,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("bookings_slots")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("resource_id"))
                    .col(schema::timestamp_with_time_zone("slot_start"))
                    // Một trong hai: lịch đặt (sức chứa 1) hay buổi (lớp học).
                    .col(schema::uuid_null("booking_id"))
                    .col(schema::uuid_null("session_id"))
                    .to_owned(),
            )
            .await?;
        // Khoá chống chồng giờ: một tài nguyên, một ô 5 phút, một chủ — trên cả hai database.
        index(
            manager,
            "bookings_slots",
            "bookings_slots_unique",
            &["tenant_id", "resource_id", "slot_start"],
            true,
        )
        .await?;
        index(
            manager,
            "bookings_slots",
            "bookings_slots_booking_idx",
            &["tenant_id", "booking_id"],
            false,
        )
        .await?;
        index(
            manager,
            "bookings_slots",
            "bookings_slots_session_idx",
            &["tenant_id", "session_id"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("bookings_guest_challenges")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("phone", 20))
                    .col(schema::string_len("code_hash", 64))
                    .col(schema::integer("attempts"))
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    .col(schema::timestamp_with_time_zone_null("consumed_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "bookings_guest_challenges",
            "bookings_guest_challenges_sweep_idx",
            &["expires_at"],
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
