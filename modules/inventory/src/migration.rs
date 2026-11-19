//! Migration của module `inventory`. Đã commit thì không sửa — thêm migration mới vào cuối [`all`].
//!
//! Mọi bảng là cấp khách hàng: có `tenant_id`, unique và index bắt đầu bằng `tenant_id` (bước 33).
//! Kiểu cột dùng được trên cả SQLite và Postgres: số lượng `big_integer`, tiền `*_minor`
//! (`big_integer`) + mã tiền tệ, JSON trong `text`, không `jsonb`, mảng hay enum của Postgres.

use autoxide_kernel::migration::prelude::*;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(CreateStock),
        // Bảng `inventory_events` (transactional outbox) và `inventory_jobs`, định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "inventory"),
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "inventory"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0004_tenancy_events",
            vec![autoxide_kernel::event::tenant_table("inventory")],
        ),
        autoxide_kernel::event::outbox_access_migration("k0001_event_access", "inventory"),
        autoxide_kernel::job::jobs_migration("m0005_create_jobs", "inventory"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0006_tenancy_jobs",
            vec![autoxide_kernel::job::tenant_table("inventory")],
        ),
        Box::new(CreateReservations),
        Box::new(CreateDocuments),
        Box::new(CreateLots),
        Box::new(CreatePurchasing),
        Box::new(CreatePreorders),
    ]
}

pub(crate) async fn index(
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

/// Bảng có khoá chính `id` (UUIDv7 sinh trong ứng dụng) và `tenant_id`.
pub(crate) fn table(name: &str) -> TableCreateStatement {
    Table::create()
        .table(Alias::new(name))
        .col(schema::pk_uuid("id"))
        .col(schema::uuid("tenant_id"))
        .to_owned()
}

/// Xoá các bảng `tables` theo thứ tự ngược.
pub(crate) async fn drop_all(manager: &SchemaManager<'_>, tables: &[&str]) -> Result<(), DbErr> {
    for name in tables.iter().rev() {
        manager.drop_table(Table::drop().table(Alias::new(*name)).to_owned()).await?;
    }
    Ok(())
}

struct CreateStock;

impl MigrationName for CreateStock {
    fn name(&self) -> &str {
        "m0001_create_stock"
    }
}

const STOCK_TABLES: [&str; 3] =
    ["inventory_levels", "inventory_movements", "inventory_adjustments"];

#[async_trait::async_trait]
impl MigrationTrait for CreateStock {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Tồn theo biến thể × kho. `on_hand`, `available` chỉ đổi bằng `rows::adjust`;
        // `reserved` = `on_hand − available`, không lưu.
        let mut levels = table("inventory_levels");
        levels
            .col(schema::uuid("org_id"))
            .col(schema::uuid("variant_id"))
            .col(schema::big_integer("on_hand"))
            .col(schema::big_integer("available"))
            .col(schema::big_integer("incoming"))
            .col(schema::big_integer_null("reorder_point"))
            .col(schema::boolean("allow_negative"))
            .col(schema::big_integer("average_cost_minor"))
            .col(schema::string_len_null("average_cost_currency", 3))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(levels).await?;
        let t = "inventory_levels";
        index(manager, t, "inventory_levels_key", &["tenant_id", "org_id", "variant_id"], true)
            .await?;
        index(manager, t, "inventory_levels_variant_idx", &["tenant_id", "variant_id"], false)
            .await?;

        // Sổ biến động: CHỈ THÊM, không bao giờ sửa hay xoá (chứng từ kế toán).
        let mut movements = table("inventory_movements");
        movements
            .col(schema::uuid("org_id"))
            .col(schema::uuid("variant_id"))
            .col(schema::uuid("level_id"))
            .col(schema::string_len("kind", 24))
            .col(schema::big_integer("quantity"))
            .col(schema::big_integer_null("unit_cost_minor"))
            .col(schema::string_len_null("unit_cost_currency", 3))
            .col(schema::uuid_null("lot_id"))
            .col(schema::string_len_null("reason_code", 64))
            .col(schema::text("note"))
            .col(schema::string_len("source_type", 32))
            .col(schema::uuid("source_id"))
            .col(schema::uuid_null("actor_id"))
            .col(schema::timestamp_with_time_zone("created_at"));
        manager.create_table(movements).await?;
        let t = "inventory_movements";
        index(manager, t, "inventory_movements_level_idx", &["tenant_id", "level_id", "id"], false)
            .await?;
        index(
            manager,
            t,
            "inventory_movements_source_idx",
            &["tenant_id", "source_type", "source_id"],
            false,
        )
        .await?;
        index(
            manager,
            t,
            "inventory_movements_created_idx",
            &["tenant_id", "created_at", "id"],
            false,
        )
        .await?;

        // Lệnh điều chỉnh tồn: giữ `reference` của người gọi vĩnh viễn (chống trùng, D-184).
        let mut adjustments = table("inventory_adjustments");
        adjustments
            .col(schema::uuid("org_id"))
            .col(schema::string_len("reference", 100))
            .col(schema::string_len("reference_fingerprint", 64))
            .col(schema::string_len("kind", 16))
            .col(schema::uuid_null("actor_id"))
            .col(schema::timestamp_with_time_zone("created_at"));
        manager.create_table(adjustments).await?;
        index(
            manager,
            "inventory_adjustments",
            "inventory_adjustments_reference_key",
            &["tenant_id", "reference"],
            true,
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_all(manager, &STOCK_TABLES).await
    }
}

struct CreateReservations;

impl MigrationName for CreateReservations {
    fn name(&self) -> &str {
        "m0007_create_reservations"
    }
}

const RESERVATION_TABLES: [&str; 6] = [
    "inventory_settings",
    "inventory_sources",
    "inventory_reservations",
    "inventory_reservation_lines",
    "inventory_fulfilments",
    "inventory_restocks",
];

#[async_trait::async_trait]
impl MigrationTrait for CreateReservations {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Cài đặt theo nút: cách cửa hàng chọn kho, và tỉnh / nhận tại cửa hàng của nút (chép từ
        // event `organizations.public_profile_updated` — module không đọc bảng của `organizations`).
        let mut settings = table("inventory_settings");
        settings
            .col(schema::uuid("org_id"))
            .col(schema::boolean("prefer_nearby"))
            .col(schema::boolean("allow_split"))
            .col(schema::string_len_null("province_code", 16))
            .col(schema::boolean("pickup"))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(settings).await?;
        index(
            manager,
            "inventory_settings",
            "inventory_settings_key",
            &["tenant_id", "org_id"],
            true,
        )
        .await?;

        // Kho phục vụ một cửa hàng / kênh bán, theo thứ tự ưu tiên.
        let mut sources = table("inventory_sources");
        sources
            .col(schema::uuid("selling_org_id"))
            .col(schema::uuid("warehouse_org_id"))
            .col(schema::integer("priority"))
            .col(schema::boolean("active"));
        manager.create_table(sources).await?;
        index(
            manager,
            "inventory_sources",
            "inventory_sources_key",
            &["tenant_id", "selling_org_id", "warehouse_org_id"],
            true,
        )
        .await?;

        // Chỗ giữ hàng: một đơn một dòng.
        let mut reservations = table("inventory_reservations");
        reservations
            .col(schema::uuid("order_id"))
            .col(schema::uuid("selling_org_id"))
            .col(schema::string_len("status", 16))
            .col(schema::timestamp_with_time_zone_null("expires_at"))
            .col(schema::string_len("fingerprint", 64))
            .col(schema::string_len_null("release_reason", 32))
            .col(schema::timestamp_with_time_zone("created_at"))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(reservations).await?;
        let t = "inventory_reservations";
        index(manager, t, "inventory_reservations_order_key", &["tenant_id", "order_id"], true)
            .await?;
        index(
            manager,
            t,
            "inventory_reservations_due_idx",
            &["tenant_id", "status", "expires_at"],
            false,
        )
        .await?;
        // Job hết hạn quét cả database (mọi khách): index không bắt đầu bằng `tenant_id`.
        index(manager, t, "inventory_reservations_sweep_idx", &["status", "expires_at"], false)
            .await?;

        // Phần đã giữ của từng dòng đơn ở một kho. `per_unit`: số đơn vị của biến thể này trong
        // một đơn vị của dòng đơn (thành phần combo; 1 với dòng thường).
        let mut lines = table("inventory_reservation_lines");
        lines
            .col(schema::uuid("reservation_id"))
            .col(schema::string_len("line_key", 100))
            .col(schema::uuid("variant_id"))
            .col(schema::uuid_null("bundle_variant_id"))
            .col(schema::big_integer("per_unit"))
            .col(schema::uuid("level_id"))
            .col(schema::uuid("org_id"))
            .col(schema::big_integer("quantity"))
            .col(schema::big_integer("fulfilled_quantity"))
            .col(schema::big_integer("released_quantity"))
            .col(schema::boolean("backordered"))
            .col(schema::boolean("preordered"));
        manager.create_table(lines).await?;
        let t = "inventory_reservation_lines";
        index(
            manager,
            t,
            "inventory_reservation_lines_idx",
            &["tenant_id", "reservation_id"],
            false,
        )
        .await?;
        index(
            manager,
            t,
            "inventory_reservation_lines_level_idx",
            &["tenant_id", "level_id"],
            false,
        )
        .await?;

        // Lần xuất kho theo chỗ giữ: chống xuất hai lần cùng `fulfilment_key`.
        let mut fulfilments = table("inventory_fulfilments");
        fulfilments
            .col(schema::uuid("reservation_id"))
            .col(schema::string_len("fulfilment_key", 100))
            .col(schema::string_len("fingerprint", 64))
            .col(schema::timestamp_with_time_zone("created_at"));
        manager.create_table(fulfilments).await?;
        index(
            manager,
            "inventory_fulfilments",
            "inventory_fulfilments_key",
            &["tenant_id", "reservation_id", "fulfilment_key"],
            true,
        )
        .await?;

        // Lần nhập lại hàng trả về: `reference` vĩnh viễn (D-184); dòng không bán lại được chỉ ghi
        // nhận trong `lines` (JSON).
        let mut restocks = table("inventory_restocks");
        restocks
            .col(schema::string_len("reference", 100))
            .col(schema::string_len("reference_fingerprint", 64))
            .col(schema::uuid_null("order_id"))
            .col(schema::uuid("org_id"))
            .col(schema::text("lines"))
            .col(schema::timestamp_with_time_zone("created_at"));
        manager.create_table(restocks).await?;
        index(
            manager,
            "inventory_restocks",
            "inventory_restocks_reference_key",
            &["tenant_id", "reference"],
            true,
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_all(manager, &RESERVATION_TABLES).await
    }
}

struct CreateDocuments;

impl MigrationName for CreateDocuments {
    fn name(&self) -> &str {
        "m0008_create_transfers_counts"
    }
}

const DOCUMENT_TABLES: [&str; 4] = [
    "inventory_transfers",
    "inventory_transfer_lines",
    "inventory_counts",
    "inventory_count_lines",
];

#[async_trait::async_trait]
impl MigrationTrait for CreateDocuments {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Phiếu chuyển kho. Số chứng từ `CK-<năm>-<số>` cấp bằng `numbering` theo khách và năm.
        let mut transfers = table("inventory_transfers");
        transfers
            .col(schema::string_len("number", 32))
            .col(schema::uuid("from_org_id"))
            .col(schema::uuid("to_org_id"))
            .col(schema::string_len("status", 24))
            .col(schema::timestamp_with_time_zone_null("shipped_at"))
            .col(schema::string_len_null("reference", 100))
            .col(schema::text("note"))
            .col(schema::uuid_null("created_by"))
            .col(schema::timestamp_with_time_zone("created_at"))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(transfers).await?;
        let t = "inventory_transfers";
        index(manager, t, "inventory_transfers_number_key", &["tenant_id", "number"], true).await?;
        index(
            manager,
            t,
            "inventory_transfers_to_idx",
            &["tenant_id", "to_org_id", "status"],
            false,
        )
        .await?;
        index(
            manager,
            t,
            "inventory_transfers_from_idx",
            &["tenant_id", "from_org_id", "status"],
            false,
        )
        .await?;

        // Dòng chuyển: giá vốn ở kho đi lúc gửi, để kho đến tính giá vốn bình quân.
        let mut lines = table("inventory_transfer_lines");
        lines
            .col(schema::uuid("transfer_id"))
            .col(schema::uuid("variant_id"))
            .col(schema::big_integer("quantity"))
            .col(schema::big_integer("received_quantity"))
            .col(schema::big_integer_null("unit_cost_minor"))
            .col(schema::string_len_null("unit_cost_currency", 3));
        manager.create_table(lines).await?;
        index(
            manager,
            "inventory_transfer_lines",
            "inventory_transfer_lines_idx",
            &["tenant_id", "transfer_id"],
            false,
        )
        .await?;

        // Phiếu kiểm kê (`KK-<năm>-<số>`).
        let mut counts = table("inventory_counts");
        counts
            .col(schema::string_len("number", 32))
            .col(schema::uuid("org_id"))
            .col(schema::string_len("status", 16))
            .col(schema::text("note"))
            .col(schema::timestamp_with_time_zone_null("applied_at"))
            .col(schema::uuid_null("created_by"))
            .col(schema::timestamp_with_time_zone("created_at"))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(counts).await?;
        let t = "inventory_counts";
        index(manager, t, "inventory_counts_number_key", &["tenant_id", "number"], true).await?;
        index(manager, t, "inventory_counts_org_idx", &["tenant_id", "org_id", "status"], false)
            .await?;

        // Số đếm: `expected_at_count` = `on_hand` lúc đếm dòng đó (kiểm kê khi đang bán).
        let mut count_lines = table("inventory_count_lines");
        count_lines
            .col(schema::uuid("count_id"))
            .col(schema::uuid("variant_id"))
            .col(schema::big_integer("counted"))
            .col(schema::big_integer("expected_at_count"))
            .col(schema::timestamp_with_time_zone("counted_at"))
            .col(schema::string_len_null("reason_code", 64));
        manager.create_table(count_lines).await?;
        index(
            manager,
            "inventory_count_lines",
            "inventory_count_lines_key",
            &["tenant_id", "count_id", "variant_id"],
            true,
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_all(manager, &DOCUMENT_TABLES).await
    }
}

struct CreateLots;

impl MigrationName for CreateLots {
    fn name(&self) -> &str {
        "m0009_create_lots_serials"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateLots {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Lô của một dòng tồn. `expiry_state` không lưu: tính lúc đọc từ `expires_on`.
        let mut lots = table("inventory_lots");
        lots.col(schema::uuid("level_id"))
            .col(schema::uuid("org_id"))
            .col(schema::uuid("variant_id"))
            .col(schema::string_len("lot_code", 64))
            .col(schema::date_null("expires_on"))
            .col(schema::big_integer("on_hand"))
            .col(schema::timestamp_with_time_zone("received_at"));
        manager.create_table(lots).await?;
        let t = "inventory_lots";
        index(manager, t, "inventory_lots_key", &["tenant_id", "level_id", "lot_code"], true)
            .await?;
        index(
            manager,
            t,
            "inventory_lots_expiry_idx",
            &["tenant_id", "level_id", "expires_on"],
            false,
        )
        .await?;
        index(manager, t, "inventory_lots_org_idx", &["tenant_id", "org_id", "expires_on"], false)
            .await?;
        // Job hằng ngày quét lô quá hạn của cả database.
        index(manager, t, "inventory_lots_sweep_idx", &["expires_on", "on_hand"], false).await?;

        let mut serials = table("inventory_serials");
        serials
            .col(schema::uuid("variant_id"))
            .col(schema::string_len("serial", 100))
            .col(schema::uuid_null("org_id"))
            .col(schema::string_len("status", 16))
            .col(schema::uuid_null("order_id"))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(serials).await?;
        index(
            manager,
            "inventory_serials",
            "inventory_serials_key",
            &["tenant_id", "variant_id", "serial"],
            true,
        )
        .await?;
        index(
            manager,
            "inventory_serials",
            "inventory_serials_org_idx",
            &["tenant_id", "org_id", "status"],
            false,
        )
        .await?;

        // Lô và serial đang đi đường của dòng chuyển kho (JSON), để kho nhận ghi lại đúng lô / serial.
        for column in ["lots", "serials"] {
            manager
                .alter_table(
                    Table::alter()
                        .table(Alias::new("inventory_transfer_lines"))
                        .add_column(
                            ColumnDef::new(Alias::new(column)).text().not_null().default("[]"),
                        )
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for column in ["serials", "lots"] {
            manager
                .alter_table(
                    Table::alter()
                        .table(Alias::new("inventory_transfer_lines"))
                        .drop_column(Alias::new(column))
                        .to_owned(),
                )
                .await?;
        }
        drop_all(manager, &["inventory_lots", "inventory_serials"]).await
    }
}

struct CreatePurchasing;

impl MigrationName for CreatePurchasing {
    fn name(&self) -> &str {
        "m0010_create_purchasing"
    }
}

const PURCHASING_TABLES: [&str; 7] = [
    "inventory_suppliers",
    "inventory_purchase_orders",
    "inventory_purchase_order_lines",
    "inventory_receipts",
    "inventory_receipt_lines",
    "inventory_supplier_returns",
    "inventory_supplier_return_lines",
];

/// Cột ghi sổ cái của phiếu (D-223…D-227).
fn ledger_columns(table: &mut TableCreateStatement) {
    table
        .col(schema::string_len("ledger_status", 16))
        .col(schema::uuid_null("ledger_entry_id"))
        .col(schema::timestamp_with_time_zone_null("ledger_posted_at"))
        .col(schema::string_len_null("ledger_error_code", 100));
}

#[async_trait::async_trait]
impl MigrationTrait for CreatePurchasing {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Nhà cung cấp, thuộc một nút (đơn vị mua hàng — sổ cái ghi công nợ ở nút đó).
        let mut suppliers = table("inventory_suppliers");
        suppliers
            .col(schema::uuid("org_id"))
            .col(schema::string_len("code", 64))
            .col(schema::string_len("name", 200))
            .col(schema::string_len("name_key", 200))
            .col(schema::string_len_null("tax_code", 20))
            .col(schema::string_len_null("contact_name", 200))
            .col(schema::string_len_null("phone", 32))
            .col(schema::string_len_null("email", 254))
            .col(schema::text_null("address"))
            .col(schema::integer("payment_terms_days"))
            .col(schema::string_len("currency", 3))
            .col(schema::boolean("active"))
            .col(schema::timestamp_with_time_zone("created_at"))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(suppliers).await?;
        let t = "inventory_suppliers";
        index(manager, t, "inventory_suppliers_code_key", &["tenant_id", "code"], true).await?;
        index(
            manager,
            t,
            "inventory_suppliers_org_idx",
            &["tenant_id", "org_id", "name_key"],
            false,
        )
        .await?;

        // Đơn đặt hàng nhà cung cấp (`PO-…`).
        let mut orders = table("inventory_purchase_orders");
        orders
            .col(schema::string_len("number", 32))
            .col(schema::uuid("org_id"))
            .col(schema::uuid("supplier_id"))
            .col(schema::string_len("status", 24))
            .col(schema::date_null("expected_on"))
            .col(schema::big_integer("total_minor"))
            .col(schema::string_len("currency", 3))
            .col(schema::text("note"))
            .col(schema::uuid_null("created_by"))
            .col(schema::timestamp_with_time_zone("created_at"))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(orders).await?;
        let t = "inventory_purchase_orders";
        index(manager, t, "inventory_purchase_orders_number_key", &["tenant_id", "number"], true)
            .await?;
        index(
            manager,
            t,
            "inventory_purchase_orders_supplier_idx",
            &["tenant_id", "supplier_id", "status"],
            false,
        )
        .await?;

        let mut order_lines = table("inventory_purchase_order_lines");
        order_lines
            .col(schema::uuid("purchase_order_id"))
            .col(schema::uuid("variant_id"))
            .col(schema::big_integer("quantity"))
            .col(schema::big_integer("received_quantity"))
            .col(schema::big_integer("unit_cost_minor"))
            .col(schema::string_len("unit_cost_currency", 3));
        manager.create_table(order_lines).await?;
        index(
            manager,
            "inventory_purchase_order_lines",
            "inventory_purchase_order_lines_idx",
            &["tenant_id", "purchase_order_id"],
            false,
        )
        .await?;

        // Phiếu nhập (`PN-…`): `reference` vĩnh viễn (D-184).
        let mut receipts = table("inventory_receipts");
        receipts
            .col(schema::string_len("number", 32))
            .col(schema::uuid("org_id"))
            .col(schema::uuid_null("supplier_id"))
            .col(schema::uuid_null("purchase_order_id"))
            .col(schema::string_len("reference", 100))
            .col(schema::string_len("reference_fingerprint", 64))
            .col(schema::timestamp_with_time_zone("posted_at"))
            .col(schema::big_integer("total_minor"))
            .col(schema::string_len("currency", 3))
            .col(schema::uuid_null("created_by"));
        ledger_columns(&mut receipts);
        manager.create_table(receipts).await?;
        let t = "inventory_receipts";
        index(manager, t, "inventory_receipts_number_key", &["tenant_id", "number"], true).await?;
        index(manager, t, "inventory_receipts_reference_key", &["tenant_id", "reference"], true)
            .await?;
        index(manager, t, "inventory_receipts_supplier_idx", &["tenant_id", "supplier_id"], false)
            .await?;

        let mut receipt_lines = table("inventory_receipt_lines");
        receipt_lines
            .col(schema::uuid("receipt_id"))
            .col(schema::uuid("variant_id"))
            .col(schema::big_integer("quantity"))
            .col(schema::big_integer("unit_cost_minor"))
            .col(schema::string_len("unit_cost_currency", 3))
            .col(schema::string_len_null("lot_code", 64))
            .col(schema::date_null("expires_on"))
            .col(schema::text("serials"));
        manager.create_table(receipt_lines).await?;
        index(
            manager,
            "inventory_receipt_lines",
            "inventory_receipt_lines_idx",
            &["tenant_id", "receipt_id"],
            false,
        )
        .await?;

        // Trả hàng nhà cung cấp (`TH-…`).
        let mut returns = table("inventory_supplier_returns");
        returns
            .col(schema::string_len("number", 32))
            .col(schema::uuid("org_id"))
            .col(schema::uuid("supplier_id"))
            .col(schema::uuid_null("receipt_id"))
            .col(schema::string_len("reference", 100))
            .col(schema::string_len("reference_fingerprint", 64))
            .col(schema::timestamp_with_time_zone("posted_at"))
            .col(schema::big_integer("total_minor"))
            .col(schema::string_len("currency", 3))
            .col(schema::uuid_null("created_by"));
        ledger_columns(&mut returns);
        manager.create_table(returns).await?;
        let t = "inventory_supplier_returns";
        index(manager, t, "inventory_supplier_returns_number_key", &["tenant_id", "number"], true)
            .await?;
        index(
            manager,
            t,
            "inventory_supplier_returns_reference_key",
            &["tenant_id", "reference"],
            true,
        )
        .await?;

        let mut return_lines = table("inventory_supplier_return_lines");
        return_lines
            .col(schema::uuid("supplier_return_id"))
            .col(schema::uuid("variant_id"))
            .col(schema::big_integer("quantity"))
            .col(schema::big_integer("unit_cost_minor"))
            .col(schema::string_len("unit_cost_currency", 3))
            .col(schema::string_len_null("lot_code", 64))
            .col(schema::text("serials"));
        manager.create_table(return_lines).await?;
        index(
            manager,
            "inventory_supplier_return_lines",
            "inventory_supplier_return_lines_idx",
            &["tenant_id", "supplier_return_id"],
            false,
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_all(manager, &PURCHASING_TABLES).await
    }
}

struct CreatePreorders;

impl MigrationName for CreatePreorders {
    fn name(&self) -> &str {
        "m0011_create_preorders"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreatePreorders {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Suất đặt trước theo biến thể (D-204). `preorder_limit` chép từ `catalog` mỗi lần giữ.
        let mut counters = table("inventory_preorder_counters");
        counters
            .col(schema::uuid("variant_id"))
            .col(schema::big_integer("reserved"))
            .col(schema::big_integer("preorder_limit"))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(counters).await?;
        index(
            manager,
            "inventory_preorder_counters",
            "inventory_preorder_counters_key",
            &["tenant_id", "variant_id"],
            true,
        )
        .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("inventory_reservation_lines"))
                    .add_column(
                        ColumnDef::new(Alias::new("allocated_quantity"))
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("inventory_reservation_lines"))
                    .drop_column(Alias::new("allocated_quantity"))
                    .to_owned(),
            )
            .await?;
        drop_all(manager, &["inventory_preorder_counters"]).await
    }
}
