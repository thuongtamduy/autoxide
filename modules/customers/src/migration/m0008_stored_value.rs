//! Thẻ quà tặng và số dư cửa hàng của `customers` (bước 47 mục 4) — TẮT mặc định (D-194,
//! D-20261120-step47-stored-value). Chỉ lưu băm của mã thẻ và 4 ký tự cuối.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0008_stored_value"
    }
}

const TABLES: [&str; 4] = [
    "customers_stored_value_holds",
    "customers_stored_value_entries",
    "customers_store_credits",
    "customers_gift_cards",
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
                    .table("customers_gift_cards")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    // Nhánh dùng được thẻ; NULL = mọi cửa hàng của khách.
                    .col(schema::uuid_null("org_scope"))
                    .col(schema::string_len("code_hash", 64))
                    .col(schema::string_len("code_last4", 4))
                    .col(schema::big_integer("initial_minor"))
                    .col(schema::big_integer("balance_minor"))
                    .col(schema::string_len("currency", 3))
                    // `active/void/expired`.
                    .col(schema::string_len("status", 16))
                    .col(schema::timestamp_with_time_zone_null("expires_at"))
                    .col(schema::uuid_null("owner_profile_id"))
                    .col(schema::string_len("issued_reference", 200))
                    .col(schema::uuid_null("issued_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        let cards = "customers_gift_cards";
        index(manager, cards, "customers_gift_cards_code", &["tenant_id", "code_hash"], true)
            .await?;
        index(
            manager,
            cards,
            "customers_gift_cards_issued",
            &["tenant_id", "issued_reference"],
            true,
        )
        .await?;
        index(
            manager,
            cards,
            "customers_gift_cards_owner",
            &["tenant_id", "owner_profile_id"],
            false,
        )
        .await?;
        index(
            manager,
            cards,
            "customers_gift_cards_expiry",
            &["tenant_id", "status", "expires_at"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("customers_store_credits")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("profile_id"))
                    .col(schema::big_integer("balance_minor"))
                    .col(schema::string_len("currency", 3))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("tenant_id").col("profile_id"))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table("customers_stored_value_entries")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    // `gift_card` hay `store_credit`.
                    .col(schema::string_len("instrument", 16))
                    .col(schema::uuid("instrument_id"))
                    // `issue/hold/redeem/release/void/expire/adjust`.
                    .col(schema::string_len("kind", 16))
                    .col(schema::big_integer("amount_minor"))
                    .col(schema::string_len("currency", 3))
                    .col(schema::string_len("reference", 200))
                    .col(schema::string_len_null("reason", 500))
                    .col(schema::uuid_null("actor_id"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        let entries = "customers_stored_value_entries";
        index(
            manager,
            entries,
            "customers_sv_entries_ref",
            &["tenant_id", "instrument", "instrument_id", "reference", "kind"],
            true,
        )
        .await?;
        index(
            manager,
            entries,
            "customers_sv_entries_instrument",
            &["tenant_id", "instrument", "instrument_id", "id"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("customers_stored_value_holds")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("reference", 200))
                    .col(schema::string_len("instrument", 16))
                    .col(schema::uuid("instrument_id"))
                    .col(schema::big_integer("amount_minor"))
                    .col(schema::string_len("currency", 3))
                    // `held/confirmed/released`.
                    .col(schema::string_len("status", 16))
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .primary_key(
                        Index::create().col("tenant_id").col("reference").col("instrument"),
                    )
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "customers_stored_value_holds",
            "customers_sv_holds_due",
            &["tenant_id", "status", "expires_at"],
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
