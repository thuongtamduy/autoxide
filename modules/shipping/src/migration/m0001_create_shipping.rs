//! Bảng của module `shipping` (bước 44). Mọi bảng là cấp khách hàng: có `tenant_id`, unique và
//! index bắt đầu bằng `tenant_id` (bước 33). Không foreign key sang bảng của module khác — chỉ giữ
//! ID (người mua, nút tổ chức, đơn hàng, biến thể). Kiểu cột chạy được trên cả SQLite và Postgres:
//! tiền `*_minor` (`big_integer`) + mã tiền tệ, cân / kích thước `big_integer`, JSON trong `text`.

use autoxide_kernel::migration::prelude::*;

use super::{address_columns, index, table};

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0001_create_shipping"
    }
}

/// Mọi bảng, theo thứ tự tạo (xoá theo thứ tự ngược).
const TABLES: &[&str] = &[
    "shipping_addresses",
    "shipping_zones",
    "shipping_zone_regions",
    "shipping_methods",
    "shipping_rates",
    "shipping_calendars",
    "shipping_calendar_days",
    "shipping_pickup_points",
    "shipping_carrier_accounts",
    "shipping_shipments",
    "shipping_parcels",
    "shipping_shipment_items",
    "shipping_tracking_events",
    "shipping_webhook_inbox",
    "shipping_cod_statements",
    "shipping_cod_statement_lines",
    "shipping_cod_discrepancies",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        addresses(manager).await?;
        rates(manager).await?;
        calendars(manager).await?;
        carriers(manager).await?;
        shipments(manager).await?;
        cod(manager).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for name in TABLES.iter().rev() {
            manager.drop_table(Table::drop().table(Alias::new(*name)).to_owned()).await?;
        }
        Ok(())
    }
}

/// Sổ địa chỉ người mua.
async fn addresses(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let t = "shipping_addresses";
    let mut create = table(t);
    create
        .col(schema::uuid("buyer_id"))
        .col(schema::string_len("label", 100))
        .col(schema::string_len("recipient_name", 200))
        .col(schema::string_len("phone", 32));
    address_columns(&mut create);
    create
        .col(schema::boolean("is_default"))
        // `buyer_id` khi là địa chỉ mặc định, NULL khi không: unique giữ đúng một địa chỉ mặc
        // định mỗi người mua kể cả khi hai lệnh đặt mặc định chạy cùng lúc (NULL không trùng nhau
        // trên cả SQLite và Postgres).
        .col(schema::uuid_null("default_key"))
        .col(schema::timestamp_with_time_zone("created_at"))
        .col(schema::timestamp_with_time_zone("updated_at"));
    manager.create_table(create).await?;
    index(manager, t, "shipping_addresses_buyer_idx", &["tenant_id", "buyer_id", "id"], false)
        .await?;
    index(manager, t, "shipping_addresses_default_unique", &["tenant_id", "default_key"], true)
        .await
}

/// Vùng, phương thức, bậc phí, điểm nhận tại cửa hàng.
async fn rates(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let mut create = table("shipping_zones");
    create
        .col(schema::uuid("org_id"))
        .col(schema::string_len("name", 200))
        .col(schema::timestamp_with_time_zone("created_at"))
        .col(schema::timestamp_with_time_zone("updated_at"));
    manager.create_table(create).await?;
    index(
        manager,
        "shipping_zones",
        "shipping_zones_org_idx",
        &["tenant_id", "org_id", "id"],
        false,
    )
    .await?;

    let t = "shipping_zone_regions";
    let mut create = table(t);
    create
        .col(schema::uuid("zone_id"))
        .col(schema::string_len("country", 2))
        // NULL = cả nước.
        .col(schema::string_len_null("province_code", 64));
    manager.create_table(create).await?;
    index(
        manager,
        t,
        "shipping_zone_regions_unique",
        &["tenant_id", "zone_id", "country", "province_code"],
        true,
    )
    .await?;
    index(
        manager,
        t,
        "shipping_zone_regions_province_idx",
        &["tenant_id", "country", "province_code"],
        false,
    )
    .await?;

    let t = "shipping_methods";
    let mut create = table(t);
    create
        .col(schema::uuid("org_id"))
        .col(schema::string_len("code", 64))
        .col(schema::string_len("kind", 16))
        // `{locale: tên}`.
        .col(schema::text("names"))
        .col(schema::boolean("active"))
        .col(schema::integer("position"))
        .col(schema::uuid_null("carrier_account_id"))
        .col(schema::string_len_null("service_code", 64))
        .col(schema::big_integer_null("free_over_minor"))
        .col(schema::big_integer("handling_fee_minor"))
        .col(schema::string_len("currency", 3))
        .col(schema::string_len("tax_category", 64))
        .col(schema::integer("volumetric_divisor"))
        .col(schema::big_integer_null("default_item_weight_grams"))
        // Mảng JSON mã quốc gia giao tới, vd `["VN"]`.
        .col(schema::text("countries"))
        .col(schema::small_integer_null("estimated_min_days"))
        .col(schema::small_integer_null("estimated_max_days"))
        .col(schema::uuid_null("calendar_id"))
        .col(schema::integer_null("cutoff_minute"))
        .col(schema::boolean("same_day"))
        .col(schema::small_integer_null("hold_days"))
        // Tự đặt vận đơn với hãng khi nhận yêu cầu giao (mặc định tắt: nhân viên kiểm hàng trước).
        .col(schema::boolean("auto_book"))
        .col(schema::timestamp_with_time_zone("created_at"))
        .col(schema::timestamp_with_time_zone("updated_at"));
    manager.create_table(create).await?;
    index(manager, t, "shipping_methods_code_unique", &["tenant_id", "org_id", "code"], true)
        .await?;

    let t = "shipping_rates";
    let mut create = table(t);
    create
        .col(schema::uuid("method_id"))
        .col(schema::uuid("zone_id"))
        .col(schema::big_integer("min_weight_grams"))
        // NULL = không giới hạn trên.
        .col(schema::big_integer_null("max_weight_grams"))
        .col(schema::big_integer_null("max_side_mm"))
        .col(schema::big_integer("amount_minor"))
        .col(schema::string_len("currency", 3))
        .col(schema::big_integer_null("step_grams"))
        .col(schema::big_integer_null("step_amount_minor"));
    manager.create_table(create).await?;
    index(
        manager,
        t,
        "shipping_rates_method_idx",
        &["tenant_id", "method_id", "zone_id", "min_weight_grams"],
        false,
    )
    .await?;

    let t = "shipping_pickup_points";
    let mut create = table(t);
    create
        .col(schema::uuid("method_id"))
        .col(schema::uuid("org_id"))
        .col(schema::text("instructions"))
        .col(schema::integer("prep_hours"));
    manager.create_table(create).await?;
    index(manager, t, "shipping_pickup_points_unique", &["tenant_id", "method_id", "org_id"], true)
        .await
}

/// Lịch giao, ngày nghỉ và giờ chốt riêng (D-201, D-223…D-227).
async fn calendars(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let t = "shipping_calendars";
    let mut create = table(t);
    create
        .col(schema::uuid("org_id"))
        .col(schema::string_len("name", 200))
        .col(schema::string_len("time_zone", 64))
        // Bit 0 = thứ Hai … bit 6 = Chủ nhật.
        .col(schema::integer("working_weekdays"))
        .col(schema::timestamp_with_time_zone("created_at"))
        .col(schema::timestamp_with_time_zone("updated_at"));
    manager.create_table(create).await?;
    index(manager, t, "shipping_calendars_org_idx", &["tenant_id", "org_id", "id"], false).await?;

    let t = "shipping_calendar_days";
    let mut create = table(t);
    create
        .col(schema::uuid("calendar_id"))
        .col(schema::date("day"))
        // `closed` hay `special_hours`.
        .col(schema::string_len("kind", 16))
        .col(schema::integer_null("cutoff_minute"))
        .col(schema::string_len("note", 300));
    manager.create_table(create).await?;
    index(manager, t, "shipping_calendar_days_unique", &["tenant_id", "calendar_id", "day"], true)
        .await
}

/// Tài khoản hãng vận chuyển của khách.
async fn carriers(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let t = "shipping_carrier_accounts";
    let mut create = table(t);
    create
        .col(schema::uuid("org_id"))
        .col(schema::string_len("carrier_code", 64))
        .col(schema::string_len("name", 200))
        // Thông tin đăng nhập đã mã hoá AEAD (`Sealer` của kernel, khoá theo khách); không bao giờ
        // trả ra API, log hay event.
        .col(schema::text("credentials_sealed"))
        // SHA-256 của token trong đường dẫn webhook.
        .col(schema::string_len("webhook_token_hash", 64))
        .col(schema::text("settings"))
        .col(schema::boolean("sandbox"))
        .col(schema::boolean("active"))
        // `never`, `ok`, `failed`.
        .col(schema::string_len("check_status", 16))
        .col(schema::timestamp_with_time_zone_null("checked_at"))
        .col(schema::string_len_null("check_error_code", 100))
        .col(schema::timestamp_with_time_zone("created_at"))
        .col(schema::timestamp_with_time_zone("updated_at"));
    manager.create_table(create).await?;
    index(manager, t, "shipping_carrier_accounts_code_idx", &["tenant_id", "carrier_code"], false)
        .await?;
    index(
        manager,
        t,
        "shipping_carrier_accounts_webhook_unique",
        &["tenant_id", "webhook_token_hash"],
        true,
    )
    .await
}

/// Vận đơn, kiện, món, hành trình, webhook nhận nguyên văn.
async fn shipments(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let t = "shipping_shipments";
    let mut create = table(t);
    create
        .col(schema::uuid("org_id"))
        .col(schema::uuid("origin_org_id"))
        .col(schema::uuid("order_id"))
        .col(schema::string_len("order_number", 64))
        .col(schema::string_len("request_key", 100))
        // SHA-256 của yêu cầu: cùng khoá khác nội dung bị từ chối, không ghi đè.
        .col(schema::string_len("request_fingerprint", 64))
        .col(schema::string_len("direction", 16))
        .col(schema::uuid_null("buyer_id"))
        .col(schema::uuid("method_id"))
        .col(schema::string_len("method_kind", 16))
        .col(schema::uuid_null("carrier_account_id"))
        .col(schema::string_len_null("carrier_code", 64))
        .col(schema::string_len_null("service_code", 64))
        .col(schema::string_len_null("tracking_number", 100))
        .col(schema::string_len("status", 32))
        .col(schema::timestamp_with_time_zone("status_at"))
        .col(schema::string_len("recipient_name", 200))
        .col(schema::string_len("recipient_phone", 32));
    address_columns(&mut create);
    create
        .col(schema::big_integer("declared_value_minor"))
        .col(schema::big_integer_null("cod_minor"))
        .col(schema::string_len("currency", 3))
        .col(schema::big_integer_null("quoted_fee_minor"))
        .col(schema::big_integer_null("actual_fee_minor"))
        // `none`, `pending`, `collected`, `settled`, `mismatch`.
        .col(schema::string_len("cod_status", 16))
        .col(schema::timestamp_with_time_zone_null("delivered_at"))
        .col(schema::integer("attempts"))
        .col(schema::string_len_null("last_error_code", 100))
        .col(schema::string_len("note", 1000))
        .col(schema::timestamp_with_time_zone("created_at"))
        .col(schema::timestamp_with_time_zone("updated_at"));
    manager.create_table(create).await?;
    index(
        manager,
        t,
        "shipping_shipments_request_unique",
        &["tenant_id", "order_id", "request_key"],
        true,
    )
    .await?;
    index(
        manager,
        t,
        "shipping_shipments_tracking_unique",
        &["tenant_id", "carrier_code", "tracking_number"],
        true,
    )
    .await?;
    index(
        manager,
        t,
        "shipping_shipments_status_idx",
        &["tenant_id", "status", "status_at"],
        false,
    )
    .await?;
    index(
        manager,
        t,
        "shipping_shipments_org_idx",
        &["tenant_id", "org_id", "created_at", "id"],
        false,
    )
    .await?;
    index(manager, t, "shipping_shipments_tracking_idx", &["tenant_id", "tracking_number"], false)
        .await?;

    let t = "shipping_parcels";
    let mut create = table(t);
    create
        .col(schema::uuid("shipment_id"))
        .col(schema::integer("position"))
        .col(schema::big_integer("weight_grams"))
        .col(schema::big_integer_null("length_mm"))
        .col(schema::big_integer_null("width_mm"))
        .col(schema::big_integer_null("height_mm"))
        .col(schema::string_len_null("tracking_number", 100));
    manager.create_table(create).await?;
    index(manager, t, "shipping_parcels_shipment_idx", &["tenant_id", "shipment_id"], false)
        .await?;

    let t = "shipping_shipment_items";
    let mut create = table(t);
    create
        .col(schema::uuid("shipment_id"))
        .col(schema::uuid_null("parcel_id"))
        .col(schema::string_len("order_line_key", 100))
        .col(schema::uuid("variant_id"))
        .col(schema::string_len("sku", 100))
        .col(schema::string_len("name", 300))
        .col(schema::big_integer("quantity"))
        .col(schema::big_integer_null("weight_grams"))
        .col(schema::big_integer_null("length_mm"))
        .col(schema::big_integer_null("width_mm"))
        .col(schema::big_integer_null("height_mm"));
    manager.create_table(create).await?;
    index(manager, t, "shipping_shipment_items_shipment_idx", &["tenant_id", "shipment_id"], false)
        .await?;

    // Chỉ thêm, không sửa.
    let t = "shipping_tracking_events";
    let mut create = table(t);
    create
        .col(schema::uuid("shipment_id"))
        // `carrier`, `staff`, `system`.
        .col(schema::string_len("source", 16))
        .col(schema::string_len("event_key", 200))
        .col(schema::string_len("status", 32))
        .col(schema::string_len("carrier_status", 200))
        .col(schema::timestamp_with_time_zone("occurred_at"))
        .col(schema::string_len("note", 1000))
        .col(schema::timestamp_with_time_zone("created_at"));
    manager.create_table(create).await?;
    index(
        manager,
        t,
        "shipping_tracking_events_unique",
        &["tenant_id", "shipment_id", "event_key"],
        true,
    )
    .await?;
    index(
        manager,
        t,
        "shipping_tracking_events_time_idx",
        &["tenant_id", "shipment_id", "occurred_at"],
        false,
    )
    .await?;

    let t = "shipping_webhook_inbox";
    let mut create = table(t);
    create
        .col(schema::uuid("carrier_account_id"))
        .col(schema::timestamp_with_time_zone("received_at"))
        // Header đã bỏ các header bí mật (JSON).
        .col(schema::text("headers"))
        .col(schema::text("body"))
        // `received`, `processed`, `failed`.
        .col(schema::string_len("status", 16))
        .col(schema::string_len_null("error", 500));
    manager.create_table(create).await?;
    index(
        manager,
        t,
        "shipping_webhook_inbox_status_idx",
        &["tenant_id", "status", "received_at"],
        false,
    )
    .await
}

/// Đối soát tiền thu hộ (COD).
async fn cod(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let t = "shipping_cod_statements";
    let mut create = table(t);
    create
        .col(schema::uuid("carrier_account_id"))
        .col(schema::string_len("statement_reference", 100))
        // SHA-256 của nội dung: nhập lại cùng nội dung không nhân đôi, khác nội dung bị từ chối.
        .col(schema::string_len("content_hash", 64))
        .col(schema::date("period_from"))
        .col(schema::date("period_to"))
        .col(schema::big_integer("total_cod_minor"))
        .col(schema::big_integer("total_fee_minor"))
        .col(schema::big_integer("net_paid_minor"))
        .col(schema::string_len("currency", 3))
        // `imported`, `reconciled`, `has_mismatch`.
        .col(schema::string_len("status", 16))
        .col(schema::uuid_null("imported_by"))
        .col(schema::timestamp_with_time_zone("imported_at"));
    manager.create_table(create).await?;
    index(
        manager,
        t,
        "shipping_cod_statements_reference_unique",
        &["tenant_id", "carrier_account_id", "statement_reference"],
        true,
    )
    .await?;

    let t = "shipping_cod_statement_lines";
    let mut create = table(t);
    create
        .col(schema::uuid("statement_id"))
        .col(schema::integer("position"))
        .col(schema::string_len("tracking_number", 100))
        .col(schema::big_integer("cod_minor"))
        .col(schema::big_integer("fee_minor"))
        .col(schema::uuid_null("shipment_id"))
        // `matched`, `amount_mismatch`, `unknown_tracking`, `not_delivered`.
        .col(schema::string_len("match", 32));
    manager.create_table(create).await?;
    index(
        manager,
        t,
        "shipping_cod_statement_lines_statement_idx",
        &["tenant_id", "statement_id"],
        false,
    )
    .await?;

    let t = "shipping_cod_discrepancies";
    let mut create = table(t);
    create
        // Chống trùng: `line:<id>` hay `overdue:<vận đơn>`.
        .col(schema::string_len("discrepancy_key", 100))
        .col(schema::uuid_null("statement_line_id"))
        .col(schema::uuid_null("shipment_id"))
        // `amount_mismatch`, `unknown_tracking`, `not_delivered`, `overdue`.
        .col(schema::string_len("kind", 32))
        .col(schema::big_integer("expected_minor"))
        .col(schema::big_integer("reported_minor"))
        .col(schema::string_len("currency", 3))
        // `open`, `resolved`.
        .col(schema::string_len("status", 16))
        .col(schema::string_len_null("resolution_code", 64))
        .col(schema::string_len("resolution_note", 1000))
        .col(schema::uuid_null("resolved_by"))
        .col(schema::timestamp_with_time_zone_null("resolved_at"))
        .col(schema::timestamp_with_time_zone("created_at"));
    manager.create_table(create).await?;
    index(
        manager,
        t,
        "shipping_cod_discrepancies_key_unique",
        &["tenant_id", "discrepancy_key"],
        true,
    )
    .await?;
    index(
        manager,
        t,
        "shipping_cod_discrepancies_status_idx",
        &["tenant_id", "status", "id"],
        false,
    )
    .await
}
