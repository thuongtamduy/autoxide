//! Migration của module `compliance`. Đã commit thì không sửa — thêm migration mới vào cuối [`all`].
//!
//! `compliance_rules` là bảng **mặt điều khiển** (luật dùng chung mọi khách, như `tax_rates`); mọi bảng
//! khác là cấp khách hàng.

use autoxide_kernel::migration::prelude::*;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(KycAndRules),
        Box::new(LegalDocuments),
        Box::new(Consents),
        autoxide_kernel::event::outbox_migration("m0004_create_events", "compliance"),
        autoxide_kernel::event::outbox_actor_migration("m0005_add_event_actor", "compliance"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0006_tenancy_events",
            vec![autoxide_kernel::event::tenant_table("compliance")],
        ),
        autoxide_kernel::event::outbox_access_migration("k0001_event_access", "compliance"),
        autoxide_kernel::job::jobs_migration("m0007_create_jobs", "compliance"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0008_tenancy_jobs",
            vec![autoxide_kernel::job::tenant_table("compliance")],
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

async fn drop_all(manager: &SchemaManager<'_>, tables: &[&str]) -> Result<(), DbErr> {
    for table in tables {
        manager.drop_table(Table::drop().table(Alias::new(*table)).to_owned()).await?;
    }
    Ok(())
}

struct KycAndRules;

impl MigrationName for KycAndRules {
    fn name(&self) -> &str {
        "m0001_kyc_and_verification"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for KycAndRules {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("compliance_rules")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("jurisdiction", 8))
                    .col(schema::string_len("key", 64))
                    .col(schema::big_integer("version"))
                    .col(schema::timestamp_with_time_zone("effective_from"))
                    .col(schema::timestamp_with_time_zone_null("effective_to"))
                    .col(schema::text("body"))
                    .col(schema::text("legal_basis"))
                    .col(schema::boolean("needs_legal_review"))
                    .col(schema::string_len("source", 16))
                    .col(schema::timestamp_with_time_zone("imported_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "compliance_rules",
            "compliance_rules_version_idx",
            &["jurisdiction", "key", "version"],
            true,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("compliance_policies")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("key", 64))
                    .col(schema::text("value"))
                    .col(schema::uuid_null("updated_by"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("tenant_id").col("key"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("compliance_kyc_profiles")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("subject_kind", 16))
                    .col(schema::string_len("subject_id", 200))
                    .col(schema::string_len("party_type", 32))
                    .col(schema::string_len("status", 16))
                    .col(schema::string_len_null("method", 32))
                    .col(schema::string_len_null("provider", 32))
                    .col(schema::string_len_null("provider_reference", 100))
                    .col(schema::string_len_null("verified_name", 200))
                    .col(schema::string_len_null("document_kind", 32))
                    .col(schema::string_len_null("document_hash", 64))
                    .col(schema::string_len_null("document_last4", 4))
                    .col(schema::string_len_null("assurance", 32))
                    .col(schema::text("document_files"))
                    .col(schema::text_null("info_request"))
                    .col(schema::timestamp_with_time_zone_null("verified_at"))
                    .col(schema::timestamp_with_time_zone_null("expires_at"))
                    .col(schema::string_len_null("reason_code", 64))
                    .col(schema::uuid_null("reviewed_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "compliance_kyc_profiles",
            "compliance_kyc_profiles_subject_idx",
            &["tenant_id", "subject_kind", "subject_id"],
            true,
        )
        .await?;
        index(
            manager,
            "compliance_kyc_profiles",
            "compliance_kyc_profiles_status_idx",
            &["tenant_id", "status", "id"],
            false,
        )
        .await?;
        index(
            manager,
            "compliance_kyc_profiles",
            "compliance_kyc_profiles_document_idx",
            &["tenant_id", "document_hash"],
            false,
        )
        .await?;
        index(
            manager,
            "compliance_kyc_profiles",
            "compliance_kyc_profiles_expiry_sweep_idx",
            &["status", "expires_at"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("compliance_kyc_attempts")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("profile_id"))
                    .col(schema::string_len("method", 32))
                    .col(schema::string_len("provider", 32))
                    .col(schema::string_len("provider_reference", 100))
                    .col(schema::string_len("status", 16))
                    .col(schema::string_len_null("result_code", 64))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone_null("completed_at"))
                    .foreign_key(
                        ForeignKey::create()
                            .name("compliance_kyc_attempts_profile_fk")
                            .from("compliance_kyc_attempts", "profile_id")
                            .to("compliance_kyc_profiles", "id"),
                    )
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "compliance_kyc_attempts",
            "compliance_kyc_attempts_reference_idx",
            &["tenant_id", "provider", "provider_reference"],
            true,
        )
        .await?;
        index(
            manager,
            "compliance_kyc_attempts",
            "compliance_kyc_attempts_profile_idx",
            &["tenant_id", "profile_id", "id"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("compliance_kyc_callbacks")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("provider", 32))
                    .col(schema::string_len("provider_reference", 100))
                    .col(schema::text("body"))
                    .col(schema::timestamp_with_time_zone("received_at"))
                    .col(schema::timestamp_with_time_zone_null("processed_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "compliance_kyc_callbacks",
            "compliance_kyc_callbacks_received_idx",
            &["tenant_id", "received_at"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_all(
            manager,
            &[
                "compliance_kyc_callbacks",
                "compliance_kyc_attempts",
                "compliance_kyc_profiles",
                "compliance_policies",
                "compliance_rules",
            ],
        )
        .await
    }
}

struct LegalDocuments;

impl MigrationName for LegalDocuments {
    fn name(&self) -> &str {
        "m0002_legal_and_documents"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for LegalDocuments {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("compliance_documents")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("org_key", 36))
                    .col(schema::string_len("kind", 32))
                    .col(schema::string_len("audience", 16))
                    .col(schema::string_len("title", 200))
                    .col(schema::integer_null("current_version"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "compliance_documents",
            "compliance_documents_kind_idx",
            &["tenant_id", "org_key", "kind"],
            true,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("compliance_document_versions")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("document_id"))
                    .col(schema::integer("version"))
                    .col(schema::string_len("locale", 16))
                    .col(schema::string_len("source", 100))
                    .col(schema::string_len("title", 200))
                    .col(schema::text("body"))
                    .col(schema::string_len("sha256", 64))
                    .col(schema::timestamp_with_time_zone("effective_from"))
                    .col(schema::boolean("requires_reacceptance"))
                    .col(schema::uuid_null("published_by"))
                    .col(schema::timestamp_with_time_zone("published_at"))
                    .foreign_key(
                        ForeignKey::create()
                            .name("compliance_document_versions_document_fk")
                            .from("compliance_document_versions", "document_id")
                            .to("compliance_documents", "id"),
                    )
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "compliance_document_versions",
            "compliance_document_versions_number_idx",
            &["tenant_id", "document_id", "version", "locale"],
            true,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("compliance_acceptances")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("document_id"))
                    .col(schema::integer("version"))
                    .col(schema::uuid("version_id"))
                    .col(schema::string_len("subject_kind", 16))
                    .col(schema::string_len("subject_id", 200))
                    .col(schema::uuid_null("accepted_by"))
                    .col(schema::string_len("channel", 16))
                    .col(schema::string_len("method", 16))
                    .col(schema::string_len_null("ip_fingerprint", 64))
                    .col(schema::string_len_null("user_agent", 300))
                    .col(schema::string_len("sha256", 64))
                    .col(schema::timestamp_with_time_zone("accepted_at"))
                    .foreign_key(
                        ForeignKey::create()
                            .name("compliance_acceptances_version_fk")
                            .from("compliance_acceptances", "version_id")
                            .to("compliance_document_versions", "id"),
                    )
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "compliance_acceptances",
            "compliance_acceptances_once_idx",
            &["tenant_id", "version_id", "subject_kind", "subject_id"],
            true,
        )
        .await?;
        index(
            manager,
            "compliance_acceptances",
            "compliance_acceptances_subject_idx",
            &["tenant_id", "subject_kind", "subject_id", "document_id"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_all(
            manager,
            &["compliance_acceptances", "compliance_document_versions", "compliance_documents"],
        )
        .await
    }
}

struct Consents;

impl MigrationName for Consents {
    fn name(&self) -> &str {
        "m0003_privacy_and_cases"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Consents {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("compliance_consents")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("subject_kind", 16))
                    .col(schema::string_len("subject_key", 64))
                    .col(schema::string_len_null("subject_value", 320))
                    .col(schema::uuid_null("user_id"))
                    .col(schema::string_len("purpose", 64))
                    .col(schema::string_len("scope", 100))
                    .col(schema::string_len("channel", 16))
                    .col(schema::string_len("state", 16))
                    .col(schema::timestamp_with_time_zone_null("granted_at"))
                    .col(schema::timestamp_with_time_zone_null("withdrawn_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "compliance_consents",
            "compliance_consents_key_idx",
            &["tenant_id", "subject_kind", "subject_key", "purpose", "scope", "channel"],
            true,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("compliance_consent_records")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("consent_id"))
                    .col(schema::string_len("action", 16))
                    .col(schema::text("evidence"))
                    .col(schema::string_len("source", 64))
                    .col(schema::string_len("source_ref", 200))
                    .col(schema::timestamp_with_time_zone("recorded_at"))
                    .foreign_key(
                        ForeignKey::create()
                            .name("compliance_consent_records_consent_fk")
                            .from("compliance_consent_records", "consent_id")
                            .to("compliance_consents", "id"),
                    )
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "compliance_consent_records",
            "compliance_consent_records_ref_idx",
            &["tenant_id", "source_ref", "action"],
            true,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_all(manager, &["compliance_consent_records", "compliance_consents"]).await
    }
}
