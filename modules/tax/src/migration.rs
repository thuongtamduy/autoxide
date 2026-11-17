//! Migration của module `tax`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm migration mới
//! vào cuối [`all`].
//!
//! `tax_packs`, `tax_jurisdictions`, `tax_categories`, `tax_rates`, `tax_withholding_rates`,
//! `tax_rules` là bảng **mặt điều khiển** (D-253): dữ liệu luật dùng chung mọi khách. `tax_profiles`
//! là bảng cấp khách hàng.

use autoxide_kernel::migration::prelude::*;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(CreateTax),
        autoxide_kernel::event::outbox_migration("m0002_create_events", "tax"),
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "tax"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0004_tenancy_events",
            vec![autoxide_kernel::event::tenant_table("tax")],
        ),
        autoxide_kernel::event::outbox_access_migration("k0001_event_access", "tax"),
        autoxide_kernel::job::jobs_migration("m0005_create_jobs", "tax"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0006_tenancy_jobs",
            vec![autoxide_kernel::job::tenant_table("tax")],
        ),
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

struct CreateTax;

impl MigrationName for CreateTax {
    fn name(&self) -> &str {
        "m0001_create_tax"
    }
}

const TABLES: [&str; 7] = [
    "tax_profiles",
    "tax_rules",
    "tax_withholding_rates",
    "tax_rates",
    "tax_categories",
    "tax_jurisdictions",
    "tax_packs",
];

#[async_trait::async_trait]
impl MigrationTrait for CreateTax {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("tax_packs")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("code", 32))
                    .col(schema::big_integer("version"))
                    .col(schema::string_len("checked_on", 10))
                    .col(schema::text("source_note"))
                    .col(schema::string_len("content_sha256", 64))
                    .col(schema::uuid_null("imported_by"))
                    .col(schema::timestamp_with_time_zone("imported_at"))
                    .to_owned(),
            )
            .await?;
        index(manager, "tax_packs", "tax_packs_code_version_unique", &["code", "version"], true)
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("tax_jurisdictions")
                    .col(schema::string_len("code", 8).primary_key())
                    .col(schema::string_len("name", 200))
                    .col(schema::string_len("default_currency", 3))
                    .col(schema::string_len("pack_code", 32))
                    .col(schema::big_integer("pack_version"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("tax_categories")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("jurisdiction", 8))
                    .col(schema::string_len("code", 64))
                    .col(schema::string_len("name_en", 200))
                    .col(schema::string_len("name_vi", 200))
                    .col(schema::string_len("tax_kind", 16))
                    .col(schema::big_integer("pack_version"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "tax_categories",
            "tax_categories_code_unique",
            &["jurisdiction", "code"],
            true,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("tax_rates")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("jurisdiction", 8))
                    .col(schema::string_len("category_code", 64))
                    .col(schema::string_len("tax_kind", 16))
                    .col(schema::string_len("basis", 16))
                    .col(schema::string_len("rate_percent", 32))
                    .col(schema::big_integer_null("per_unit_minor"))
                    .col(schema::string_len_null("per_unit_currency", 3))
                    .col(schema::integer("stage"))
                    .col(schema::string_len("effective_from", 10))
                    .col(schema::string_len_null("effective_to", 10))
                    .col(schema::text("legal_basis"))
                    .col(schema::big_integer("pack_version"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "tax_rates",
            "tax_rates_lookup_idx",
            &["jurisdiction", "category_code", "effective_from"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("tax_withholding_rates")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("jurisdiction", 8))
                    .col(schema::string_len("payer_kind", 32))
                    .col(schema::string_len("payee_regime", 32))
                    .col(schema::string_len("business_line", 32))
                    .col(schema::string_len("tax_kind", 16))
                    .col(schema::string_len("rate_percent", 32))
                    .col(schema::big_integer_null("threshold_minor"))
                    .col(schema::string_len_null("threshold_currency", 3))
                    .col(schema::string_len("effective_from", 10))
                    .col(schema::string_len_null("effective_to", 10))
                    .col(schema::text("legal_basis"))
                    .col(schema::big_integer("pack_version"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "tax_withholding_rates",
            "tax_withholding_rates_lookup_idx",
            &["jurisdiction", "payer_kind", "effective_from"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("tax_rules")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("jurisdiction", 8))
                    .col(schema::string_len("code", 64))
                    .col(schema::text("value"))
                    .col(schema::string_len("effective_from", 10))
                    .col(schema::string_len_null("effective_to", 10))
                    .col(schema::text("legal_basis"))
                    .col(schema::big_integer("pack_version"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "tax_rules",
            "tax_rules_lookup_idx",
            &["jurisdiction", "code", "effective_from"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("tax_profiles")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("jurisdiction", 8))
                    .col(schema::string_len_null("tax_code", 14))
                    .col(schema::string_len("regime", 16))
                    .col(schema::text("business_lines"))
                    .col(schema::big_integer_null("annual_revenue_minor"))
                    .col(schema::string_len_null("annual_revenue_currency", 3))
                    .col(schema::string_len_null("legal_name", 200))
                    .col(schema::text_null("address"))
                    .col(schema::uuid_null("updated_by"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("tenant_id").col("org_id"))
                    .to_owned(),
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
