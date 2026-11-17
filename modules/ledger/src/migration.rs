//! Migration của module `ledger`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm migration mới
//! vào cuối [`all`]. Mọi bảng là cấp khách hàng; mọi unique / index bắt đầu bằng `tenant_id`.

use autoxide_kernel::migration::prelude::*;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(CreateLedger),
        autoxide_kernel::event::outbox_migration("m0002_create_events", "ledger"),
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "ledger"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0004_tenancy_events",
            vec![autoxide_kernel::event::tenant_table("ledger")],
        ),
        autoxide_kernel::event::outbox_access_migration("k0001_event_access", "ledger"),
        autoxide_kernel::job::jobs_migration("m0005_create_jobs", "ledger"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0006_tenancy_jobs",
            vec![autoxide_kernel::job::tenant_table("ledger")],
        ),
        Box::new(CreatePayments),
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

struct CreateLedger;

impl MigrationName for CreateLedger {
    fn name(&self) -> &str {
        "m0001_create_ledger"
    }
}

const TABLES: [&str; 9] = [
    "ledger_entry_seals",
    "ledger_chain_heads",
    "ledger_period_balances",
    // `ledger_entries` giữ foreign key tới `ledger_periods`: xoá bút toán trước kỳ (Postgres từ chối thứ tự
    // ngược lại; SQLite không kiểm).
    "ledger_lines",
    "ledger_entries",
    "ledger_periods",
    "ledger_account_balances",
    "ledger_accounts",
    "ledger_books",
];

#[async_trait::async_trait]
impl MigrationTrait for CreateLedger {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("ledger_books")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("base_currency", 3))
                    .col(schema::integer("fiscal_year_start_month"))
                    .col(schema::date_null("closed_through"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(manager, "ledger_books", "ledger_books_org_idx", &["tenant_id", "org_id"], true)
            .await?;

        manager
            .create_table(
                Table::create()
                    .table("ledger_accounts")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("book_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("purpose", 64))
                    .col(schema::string_len("counterparty_kind", 64))
                    .col(schema::string_len("counterparty_id", 100))
                    .col(schema::string_len("currency", 3))
                    .col(schema::string_len("normal_side", 8))
                    .col(schema::big_integer_null("floor_minor"))
                    .col(schema::string_len_null("external_code", 32))
                    .col(schema::string_len_null("name", 200))
                    .col(schema::string_len("status", 8))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .foreign_key(
                        ForeignKey::create()
                            .name("ledger_accounts_book_fk")
                            .from("ledger_accounts", "book_id")
                            .to("ledger_books", "id"),
                    )
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "ledger_accounts",
            "ledger_accounts_key_idx",
            &[
                "tenant_id",
                "book_id",
                "purpose",
                "counterparty_kind",
                "counterparty_id",
                "currency",
            ],
            true,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("ledger_account_balances")
                    .col(schema::pk_uuid("account_id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::big_integer("balance_minor"))
                    .col(schema::string_len("currency", 3))
                    .foreign_key(
                        ForeignKey::create()
                            .name("ledger_account_balances_account_fk")
                            .from("ledger_account_balances", "account_id")
                            .to("ledger_accounts", "id"),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table("ledger_periods")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("book_id"))
                    .col(schema::date("starts_on"))
                    .col(schema::date("ends_on"))
                    .col(schema::string_len("status", 8))
                    .col(schema::timestamp_with_time_zone_null("closed_at"))
                    .col(schema::uuid_null("closed_by"))
                    .foreign_key(
                        ForeignKey::create()
                            .name("ledger_periods_book_fk")
                            .from("ledger_periods", "book_id")
                            .to("ledger_books", "id"),
                    )
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "ledger_periods",
            "ledger_periods_start_idx",
            &["tenant_id", "book_id", "starts_on"],
            true,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("ledger_entries")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("book_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::uuid("period_id"))
                    .col(schema::date("effective_date"))
                    .col(schema::timestamp_with_time_zone("posted_at"))
                    .col(schema::string_len("description", 200))
                    .col(schema::string_len("source_module", 64))
                    .col(schema::string_len("source_kind", 64))
                    .col(schema::string_len_null("source_id", 100))
                    .col(schema::string_len("reference", 100))
                    .col(schema::string_len("reference_fingerprint", 64))
                    .col(schema::uuid_null("reverses_entry_id"))
                    .col(schema::uuid_null("actor_user_id"))
                    .col(schema::string_len("actor_kind", 16))
                    .foreign_key(
                        ForeignKey::create()
                            .name("ledger_entries_book_fk")
                            .from("ledger_entries", "book_id")
                            .to("ledger_books", "id"),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("ledger_entries_period_fk")
                            .from("ledger_entries", "period_id")
                            .to("ledger_periods", "id"),
                    )
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "ledger_entries",
            "ledger_entries_reference_idx",
            &["tenant_id", "source_module", "reference"],
            true,
        )
        .await?;
        // Mỗi bút toán đảo được một lần; nhiều `NULL` không đụng nhau trên cả hai database.
        index(
            manager,
            "ledger_entries",
            "ledger_entries_reverses_idx",
            &["tenant_id", "reverses_entry_id"],
            true,
        )
        .await?;
        index(
            manager,
            "ledger_entries",
            "ledger_entries_book_date_idx",
            &["tenant_id", "book_id", "effective_date", "id"],
            false,
        )
        .await?;
        index(
            manager,
            "ledger_entries",
            "ledger_entries_source_idx",
            &["tenant_id", "source_module", "source_id"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("ledger_lines")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("entry_id"))
                    .col(schema::integer("line_no"))
                    .col(schema::uuid("account_id"))
                    .col(schema::string_len("side", 8))
                    .col(schema::big_integer("amount_minor"))
                    .col(schema::string_len("amount_currency", 3))
                    .col(schema::big_integer("base_minor"))
                    .col(schema::string_len("base_currency", 3))
                    .col(schema::string_len_null("exchange_rate", 40))
                    .col(schema::date("effective_date"))
                    .foreign_key(
                        ForeignKey::create()
                            .name("ledger_lines_entry_fk")
                            .from("ledger_lines", "entry_id")
                            .to("ledger_entries", "id"),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("ledger_lines_account_fk")
                            .from("ledger_lines", "account_id")
                            .to("ledger_accounts", "id"),
                    )
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "ledger_lines",
            "ledger_lines_account_date_idx",
            &["tenant_id", "account_id", "effective_date", "id"],
            false,
        )
        .await?;
        index(manager, "ledger_lines", "ledger_lines_entry_idx", &["tenant_id", "entry_id"], false)
            .await?;

        manager
            .create_table(
                Table::create()
                    .table("ledger_period_balances")
                    .col(schema::uuid("period_id"))
                    .col(schema::uuid("account_id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::big_integer("closing_minor"))
                    .col(schema::string_len("currency", 3))
                    .primary_key(Index::create().col("period_id").col("account_id"))
                    .foreign_key(
                        ForeignKey::create()
                            .name("ledger_period_balances_period_fk")
                            .from("ledger_period_balances", "period_id")
                            .to("ledger_periods", "id"),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table("ledger_chain_heads")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("book_id"))
                    .col(schema::big_integer("last_sequence"))
                    .col(schema::string_len("last_hash", 64))
                    .primary_key(Index::create().col("tenant_id").col("book_id"))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table("ledger_entry_seals")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("book_id"))
                    .col(schema::big_integer("sequence"))
                    .col(schema::uuid("entry_id"))
                    .col(schema::string_len("entry_hash", 64))
                    .col(schema::string_len("chain_hash", 64))
                    .col(schema::timestamp_with_time_zone("sealed_at"))
                    .primary_key(Index::create().col("tenant_id").col("book_id").col("sequence"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "ledger_entry_seals",
            "ledger_entry_seals_entry_idx",
            &["tenant_id", "entry_id"],
            true,
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

/// Thanh toán, hoàn tiền, thông báo đối tác, sao kê, hàng chờ sai lệch, lượt đối soát (bước 36 mục 5).
struct CreatePayments;

impl MigrationName for CreatePayments {
    fn name(&self) -> &str {
        "m0007_create_payments"
    }
}

const PAYMENT_TABLES: [&str; 6] = [
    "ledger_reconciliations",
    "ledger_exceptions",
    "ledger_statement_lines",
    "ledger_gateway_events",
    "ledger_refunds",
    "ledger_payments",
];

#[async_trait::async_trait]
impl MigrationTrait for CreatePayments {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("ledger_payments")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::uuid("book_id"))
                    .col(schema::string_len("provider", 32))
                    .col(schema::string_len_null("provider_ref", 100))
                    .col(schema::string_len("method", 32))
                    .col(schema::string_len("description", 140))
                    .col(schema::big_integer("amount_minor"))
                    .col(schema::string_len("amount_currency", 3))
                    .col(schema::big_integer("refunded_minor"))
                    .col(schema::string_len("status", 16))
                    .col(schema::text_null("checkout_url"))
                    .col(schema::text_null("qr_payload"))
                    .col(schema::text_null("return_url"))
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    .col(schema::timestamp_with_time_zone_null("captured_at"))
                    .col(schema::string_len("source_module", 64))
                    .col(schema::string_len_null("source_id", 100))
                    .col(schema::string_len("reference", 100))
                    .col(schema::string_len("reference_fingerprint", 64))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .foreign_key(
                        ForeignKey::create()
                            .name("ledger_payments_book_fk")
                            .from("ledger_payments", "book_id")
                            .to("ledger_books", "id"),
                    )
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "ledger_payments",
            "ledger_payments_reference_idx",
            &["tenant_id", "source_module", "reference"],
            true,
        )
        .await?;
        // Nhiều `NULL` (chưa gọi đối tác) không đụng nhau trên cả hai database.
        index(
            manager,
            "ledger_payments",
            "ledger_payments_provider_ref_idx",
            &["tenant_id", "provider", "provider_ref"],
            true,
        )
        .await?;
        index(
            manager,
            "ledger_payments",
            "ledger_payments_expiry_sweep_idx",
            &["status", "expires_at"],
            false,
        )
        .await?;
        index(
            manager,
            "ledger_payments",
            "ledger_payments_source_idx",
            &["tenant_id", "source_module", "source_id"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("ledger_refunds")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("payment_id"))
                    .col(schema::big_integer("amount_minor"))
                    .col(schema::string_len("amount_currency", 3))
                    .col(schema::string_len("status", 16))
                    .col(schema::string_len_null("provider_ref", 100))
                    .col(schema::string_len("reason", 200))
                    .col(schema::string_len("reference", 100))
                    .col(schema::string_len("reference_fingerprint", 64))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .foreign_key(
                        ForeignKey::create()
                            .name("ledger_refunds_payment_fk")
                            .from("ledger_refunds", "payment_id")
                            .to("ledger_payments", "id"),
                    )
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "ledger_refunds",
            "ledger_refunds_reference_idx",
            &["tenant_id", "payment_id", "reference"],
            true,
        )
        .await?;
        index(
            manager,
            "ledger_refunds",
            "ledger_refunds_provider_ref_idx",
            &["tenant_id", "provider_ref"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("ledger_gateway_events")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("provider", 32))
                    .col(schema::string_len("provider_event_id", 100))
                    .col(schema::string_len("kind", 16))
                    .col(schema::text("body"))
                    .col(schema::text("notification"))
                    .col(schema::string_len("status", 16))
                    .col(schema::text_null("error"))
                    .col(schema::timestamp_with_time_zone("received_at"))
                    .col(schema::timestamp_with_time_zone_null("processed_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "ledger_gateway_events",
            "ledger_gateway_events_provider_idx",
            &["tenant_id", "provider", "provider_event_id"],
            true,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("ledger_reconciliations")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("provider", 32))
                    .col(schema::date("on_date"))
                    .col(schema::string_len("status", 16))
                    .col(schema::big_integer("lines"))
                    .col(schema::big_integer("matched"))
                    .col(schema::big_integer("exceptions"))
                    .col(schema::text_null("error"))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "ledger_reconciliations",
            "ledger_reconciliations_created_idx",
            &["tenant_id", "created_at"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("ledger_statement_lines")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("provider", 32))
                    .col(schema::string_len("provider_line_id", 100))
                    .col(schema::string_len("provider_ref", 100))
                    .col(schema::string_len("kind", 16))
                    .col(schema::big_integer("amount_minor"))
                    .col(schema::string_len("amount_currency", 3))
                    .col(schema::big_integer("fee_minor"))
                    .col(schema::date("booked_on"))
                    .col(schema::string_len("match_status", 16))
                    .col(schema::uuid_null("payment_id"))
                    .col(schema::uuid("reconciliation_id"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "ledger_statement_lines",
            "ledger_statement_lines_provider_idx",
            &["tenant_id", "provider", "provider_line_id"],
            true,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("ledger_exceptions")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("kind", 32))
                    .col(schema::string_len("subject", 100))
                    .col(schema::text("details"))
                    .col(schema::string_len("dedupe_key", 200))
                    .col(schema::string_len("status", 16))
                    .col(schema::string_len_null("resolution", 32))
                    .col(schema::string_len_null("note", 500))
                    .col(schema::uuid_null("resolved_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "ledger_exceptions",
            "ledger_exceptions_dedupe_idx",
            &["tenant_id", "dedupe_key"],
            true,
        )
        .await?;
        index(
            manager,
            "ledger_exceptions",
            "ledger_exceptions_status_idx",
            &["tenant_id", "status", "created_at"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in PAYMENT_TABLES {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}
