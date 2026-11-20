//! Migration của module `invoices`. Đã commit thì không sửa — thêm migration mới vào cuối [`all`].

use autoxide_kernel::migration::prelude::*;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(CreateInvoices),
        autoxide_kernel::event::outbox_migration("m0002_create_events", "invoices"),
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "invoices"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0004_tenancy_events",
            vec![autoxide_kernel::event::tenant_table("invoices")],
        ),
        autoxide_kernel::event::outbox_access_migration("k0001_event_access", "invoices"),
        autoxide_kernel::job::jobs_migration("m0005_create_jobs", "invoices"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0006_tenancy_jobs",
            vec![autoxide_kernel::job::tenant_table("invoices")],
        ),
        Box::new(AddPrintExportArchive),
        Box::new(AddBookedBySource),
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

struct CreateInvoices;

impl MigrationName for CreateInvoices {
    fn name(&self) -> &str {
        "m0001_create_invoices"
    }
}

const TABLES: [&str; 4] =
    ["invoices_mandates", "invoices_lines", "invoices_documents", "invoices_series"];

#[async_trait::async_trait]
impl MigrationTrait for CreateInvoices {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("invoices_series")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("kind", 32))
                    .col(schema::string_len("symbol", 16))
                    .col(schema::integer("year"))
                    .col(schema::string_len_null("einvoice_provider", 32))
                    .col(schema::string_len("status", 16))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "invoices_series",
            "invoices_series_key_idx",
            &["tenant_id", "org_id", "kind", "symbol", "year"],
            true,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("invoices_documents")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::uuid_null("on_behalf_of_org_id"))
                    .col(schema::string_len("kind", 32))
                    .col(schema::string_len_null("relation", 16))
                    .col(schema::uuid_null("related_document_id"))
                    .col(schema::string_len("direction", 16))
                    .col(schema::uuid_null("series_id"))
                    .col(schema::big_integer_null("number"))
                    .col(schema::string_len("status", 16))
                    .col(schema::date_null("issue_date"))
                    .col(schema::boolean("prices_include_tax"))
                    .col(schema::string_len("currency", 3))
                    .col(schema::big_integer("subtotal_minor"))
                    .col(schema::big_integer("tax_minor"))
                    .col(schema::big_integer("total_minor"))
                    .col(schema::text("buyer"))
                    .col(schema::text("seller"))
                    .col(schema::text("tax_snapshot"))
                    .col(schema::string_len_null("provider_ref", 100))
                    .col(schema::big_integer_null("provider_number"))
                    .col(schema::string_len_null("authority_code", 100))
                    .col(schema::string_len_null("lookup_code", 100))
                    .col(schema::text_null("rejection_reason"))
                    .col(schema::date_null("retention_until"))
                    .col(schema::string_len("source_module", 64))
                    .col(schema::string_len_null("source_id", 100))
                    .col(schema::string_len("reference", 100))
                    .col(schema::string_len("reference_fingerprint", 64))
                    .col(schema::uuid_null("issued_by"))
                    .col(schema::timestamp_with_time_zone_null("issued_at"))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .foreign_key(
                        ForeignKey::create()
                            .name("invoices_documents_series_fk")
                            .from("invoices_documents", "series_id")
                            .to("invoices_series", "id"),
                    )
                    .to_owned(),
            )
            .await?;
        // Số trong một ký hiệu không trùng; nháp (`NULL`) không đụng nhau trên cả hai database.
        index(
            manager,
            "invoices_documents",
            "invoices_documents_number_idx",
            &["tenant_id", "series_id", "number"],
            true,
        )
        .await?;
        index(
            manager,
            "invoices_documents",
            "invoices_documents_reference_idx",
            &["tenant_id", "source_module", "reference"],
            true,
        )
        .await?;
        index(
            manager,
            "invoices_documents",
            "invoices_documents_org_idx",
            &["tenant_id", "org_id", "issue_date", "id"],
            false,
        )
        .await?;
        index(
            manager,
            "invoices_documents",
            "invoices_documents_status_sweep_idx",
            &["status", "updated_at"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("invoices_lines")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("document_id"))
                    .col(schema::integer("line_no"))
                    .col(schema::string_len("kind", 16))
                    .col(schema::string_len_null("item_ref", 100))
                    .col(schema::string_len("description", 500))
                    .col(schema::string_len_null("unit", 32))
                    .col(schema::string_len("quantity", 40))
                    .col(schema::big_integer("unit_price_minor"))
                    .col(schema::big_integer("discount_minor"))
                    .col(schema::big_integer("net_minor"))
                    .col(schema::string_len_null("tax_category", 64))
                    .col(schema::string_len_null("rate_percent", 16))
                    .col(schema::big_integer("tax_minor"))
                    .col(schema::big_integer("total_minor"))
                    .foreign_key(
                        ForeignKey::create()
                            .name("invoices_lines_document_fk")
                            .from("invoices_lines", "document_id")
                            .to("invoices_documents", "id"),
                    )
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "invoices_lines",
            "invoices_lines_document_idx",
            &["tenant_id", "document_id", "line_no"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("invoices_mandates")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("principal_org_id"))
                    .col(schema::uuid("agent_org_id"))
                    .col(schema::text("kinds"))
                    .col(schema::date("valid_from"))
                    .col(schema::date_null("valid_to"))
                    .col(schema::uuid_null("document_file_id"))
                    .col(schema::string_len("status", 16))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone_null("revoked_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "invoices_mandates",
            "invoices_mandates_principal_idx",
            &["tenant_id", "principal_org_id", "status"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in TABLES {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}

/// Bước 36 mục 7, 10, 11, 12 (D-352…D-355): ảnh chụp tỉ giá trên chứng từ ngoại tệ, ẩn danh người mua
/// khi hết hạn lưu, mẫu in theo khách, bản xuất kế toán, bản lưu trữ.
struct AddPrintExportArchive;

impl MigrationName for AddPrintExportArchive {
    fn name(&self) -> &str {
        "m0007_print_export_archive"
    }
}

const DOCUMENT_COLUMNS: [&str; 5] =
    ["exchange_rate", "exchange_rate_id", "base_currency", "base_total_minor", "buyer_purged_at"];

#[async_trait::async_trait]
impl MigrationTrait for AddPrintExportArchive {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite chỉ thêm được một cột mỗi câu `ALTER TABLE`.
        let columns = [
            schema::string_len_null("exchange_rate", 40),
            schema::uuid_null("exchange_rate_id"),
            schema::string_len_null("base_currency", 3),
            schema::big_integer_null("base_total_minor"),
            schema::timestamp_with_time_zone_null("buyer_purged_at"),
        ];
        for mut column in columns {
            manager
                .alter_table(
                    Table::alter().table("invoices_documents").add_column(&mut column).to_owned(),
                )
                .await?;
        }
        // Quét chứng từ hết hạn lưu (job hằng ngày, mọi khách của database).
        index(
            manager,
            "invoices_documents",
            "invoices_documents_retention_sweep_idx",
            &["retention_until", "buyer_purged_at"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("invoices_templates")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    // UUID nil = cả khách (cột trong khoá unique không để null: hai database xử lý
                    // null khác nhau).
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("kind", 32))
                    .col(schema::string_len("locale", 8))
                    .col(schema::integer("version"))
                    .col(schema::text("body"))
                    .col(schema::uuid_null("updated_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "invoices_templates",
            "invoices_templates_key_idx",
            &["tenant_id", "org_id", "kind", "locale"],
            true,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("invoices_exports")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("profile", 32))
                    .col(schema::string_len("format", 8))
                    .col(schema::date("period_from"))
                    .col(schema::date("period_to"))
                    .col(schema::string_len("status", 16))
                    .col(schema::string_len_null("file_key", 300))
                    .col(schema::big_integer_null("size"))
                    .col(schema::big_integer_null("row_count"))
                    .col(schema::text_null("error"))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "invoices_exports",
            "invoices_exports_created_idx",
            &["tenant_id", "created_at"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("invoices_archives")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("status", 16))
                    .col(schema::string_len("reason", 32))
                    .col(schema::string_len("storage_prefix", 300))
                    .col(schema::string_len_null("manifest_sha256", 64))
                    .col(schema::text_null("files"))
                    .col(schema::date_null("retain_until"))
                    .col(schema::text_null("error"))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "invoices_archives",
            "invoices_archives_tenant_idx",
            &["tenant_id", "created_at"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in ["invoices_archives", "invoices_exports", "invoices_templates"] {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        manager
            .drop_index(
                Index::drop()
                    .name("invoices_documents_retention_sweep_idx")
                    .table(Alias::new("invoices_documents"))
                    .to_owned(),
            )
            .await?;
        for column in DOCUMENT_COLUMNS {
            manager
                .alter_table(
                    Table::alter().table("invoices_documents").drop_column(column).to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}

/// Cờ `booked_by_source`: chứng từ do module khác lập mà module đó đã tự ghi doanh thu (vd `orders`
/// ghi khi giao xong) — phát hành không ghi bút toán lần hai (D-20261120-step45-einvoice-pos).
struct AddBookedBySource;

impl MigrationName for AddBookedBySource {
    fn name(&self) -> &str {
        "m0008_booked_by_source"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for AddBookedBySource {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("invoices_documents")
                    .add_column(schema::boolean("booked_by_source").default(false))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("invoices_documents")
                    .drop_column("booked_by_source")
                    .to_owned(),
            )
            .await
    }
}
