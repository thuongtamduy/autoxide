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
