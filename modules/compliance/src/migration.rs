//! Migration của module `compliance`. Đã commit thì không sửa — thêm migration mới vào cuối [`all`].
//!
//! `compliance_rules` là bảng **mặt điều khiển** (luật dùng chung mọi khách, như `tax_rates`); mọi bảng
//! khác là cấp khách hàng.

use autoxide_kernel::migration::prelude::*;

mod kyc_document_key;

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
        Box::new(AccountVerification),
        // Bảng `compliance_throttle`: giới hạn số mã một lần theo số, người dùng, IP băm (D-116).
        autoxide_kernel::throttle::throttle_migration("m0010_create_throttle", "compliance"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0011_tenancy_throttle",
            vec![autoxide_kernel::throttle::tenant_table("compliance")],
        ),
        Box::new(MoitWebsites),
        Box::new(Privacy),
        Box::new(Tracking),
        Box::new(Cases),
        // Bảng `compliance_antispam_nonces`: nonce của token form của hai kênh báo công khai (D-363).
        autoxide_kernel::antispam::antispam_migration("m0016_create_antispam_nonces", "compliance"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0017_tenancy_antispam_nonces",
            vec![autoxide_kernel::antispam::tenant_table("compliance")],
        ),
        Box::new(ConsentStatsIndex),
        Box::new(KycReviewNote),
        Box::new(kyc_document_key::KycVerifiedDocument),
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

/// Xác thực tài khoản đăng nội dung (NĐ 147/2024, D-356): số điện thoại đã xác thực của từng người
/// dùng và mã một lần đang chờ nhập. Mức "định danh" đọc thẳng từ hồ sơ KYC, không chép sang đây.
struct AccountVerification;

impl MigrationName for AccountVerification {
    fn name(&self) -> &str {
        "m0009_account_verification"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for AccountVerification {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("compliance_account_verifications")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("user_id"))
                    .col(schema::string_len_null("phone", 16))
                    // SHA-256 của số E.164; `''` khi chưa có số (cột tham gia index không để NULL).
                    .col(schema::string_len("phone_key", 64))
                    .col(schema::timestamp_with_time_zone_null("phone_verified_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("tenant_id").col("user_id"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "compliance_account_verifications",
            "compliance_account_verifications_phone_idx",
            &["tenant_id", "phone_key"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("compliance_phone_challenges")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("user_id"))
                    .col(schema::string_len("phone", 16))
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
            "compliance_phone_challenges",
            "compliance_phone_challenges_user_idx",
            &["tenant_id", "user_id", "created_at"],
            false,
        )
        .await?;
        // Xác nhận khoá mọi mã đang chờ của cùng một số (theo thứ tự ID) để hai người không cùng nhận
        // một số khi chỉ được một tài khoản.
        index(
            manager,
            "compliance_phone_challenges",
            "compliance_phone_challenges_phone_idx",
            &["tenant_id", "phone", "id"],
            false,
        )
        .await?;
        index(
            manager,
            "compliance_phone_challenges",
            "compliance_phone_challenges_sweep_idx",
            &["created_at"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_all(manager, &["compliance_phone_challenges", "compliance_account_verifications"])
            .await
    }
}

/// Hồ sơ pháp lý, website và trang gắn website cho thông báo / đăng ký với Bộ Công Thương (D-357).
/// `org_id` luôn có giá trị: nút gốc (`root_of(tenant)`) là cả khách — không dùng NULL, nên unique
/// và lọc theo phạm vi giống mọi bảng theo nút.
struct MoitWebsites;

impl MigrationName for MoitWebsites {
    fn name(&self) -> &str {
        "m0012_moit_websites"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for MoitWebsites {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("compliance_legal_profiles")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("entity_type", 32))
                    .col(schema::string_len("legal_name", 200))
                    .col(schema::string_len_null("trade_name", 200))
                    .col(schema::string_len_null("tax_code", 14))
                    .col(schema::string_len_null("registration_number", 50))
                    .col(schema::string_len_null("registration_issued_on", 10))
                    .col(schema::string_len_null("registration_issued_by", 200))
                    // `Address` của module `address` (JSON, kèm tên lúc ghi).
                    .col(schema::text_null("address"))
                    .col(schema::string_len_null("representative_name", 200))
                    .col(schema::string_len_null("representative_title", 100))
                    .col(schema::string_len_null("phone", 20))
                    .col(schema::string_len_null("email", 254))
                    .col(schema::uuid_null("kyc_profile_id"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "compliance_legal_profiles",
            "compliance_legal_profiles_org_key",
            &["tenant_id", "org_id"],
            true,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("compliance_websites")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("domain", 253))
                    .col(schema::string_len("channel", 16))
                    .col(schema::string_len("website_kind", 16))
                    .col(schema::string_len("moit_status", 24))
                    .col(schema::string_len_null("moit_reference", 100))
                    .col(schema::timestamp_with_time_zone_null("submitted_at"))
                    .col(schema::timestamp_with_time_zone_null("decided_at"))
                    .col(schema::string_len_null("logo_link_url", 500))
                    .col(schema::string_len_null("logo_image_url", 500))
                    .col(schema::integer_null("checklist_version"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "compliance_websites",
            "compliance_websites_domain_key",
            &["tenant_id", "domain"],
            true,
        )
        .await?;
        index(
            manager,
            "compliance_websites",
            "compliance_websites_org_idx",
            &["tenant_id", "org_id", "id"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("compliance_website_pages")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("website_id"))
                    .col(schema::string_len("page_kind", 32))
                    // Đối tượng của sổ đối tượng (D-123), vd `content.entry:<id>`.
                    .col(schema::string_len("resource", 200))
                    .col(schema::timestamp_with_time_zone("linked_at"))
                    .primary_key(
                        Index::create().col("tenant_id").col("website_id").col("page_kind"),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_all(
            manager,
            &["compliance_website_pages", "compliance_websites", "compliance_legal_profiles"],
        )
        .await
    }
}

/// Sổ mục đích xử lý, sự cố dữ liệu cá nhân (mốc 72 giờ) và sổ yêu cầu của chủ thể qua kênh khác
/// (D-217, D-360).
struct Privacy;

impl MigrationName for Privacy {
    fn name(&self) -> &str {
        "m0013_privacy"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Privacy {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("compliance_purposes")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("key", 64))
                    .col(schema::string_len("category", 16))
                    .col(schema::string_len("legal_basis", 24))
                    .col(schema::text("data_categories"))
                    .col(schema::text("recipients"))
                    .col(schema::text("description"))
                    .col(schema::string_len_null("retention", 200))
                    .col(schema::boolean("active"))
                    .col(schema::uuid_null("updated_by"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "compliance_purposes",
            "compliance_purposes_key",
            &["tenant_id", "key"],
            true,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("compliance_incidents")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("title", 200))
                    .col(schema::text("description"))
                    .col(schema::string_len("severity", 16))
                    .col(schema::string_len("status", 16))
                    .col(schema::timestamp_with_time_zone("detected_at"))
                    .col(schema::timestamp_with_time_zone("report_due_at"))
                    .col(schema::integer("reminded_hours"))
                    .col(schema::timestamp_with_time_zone_null("authority_notified_at"))
                    .col(schema::string_len_null("authority_reference", 100))
                    .col(schema::timestamp_with_time_zone_null("subjects_notified_at"))
                    .col(schema::text("data_categories"))
                    .col(schema::big_integer_null("affected_count"))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "compliance_incidents",
            "compliance_incidents_due_idx",
            &["tenant_id", "status", "report_due_at"],
            false,
        )
        .await?;
        // Job nhắc hạn quét mọi khách của một database.
        index(
            manager,
            "compliance_incidents",
            "compliance_incidents_sweep_idx",
            &["status", "report_due_at"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("compliance_incident_entries")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("incident_id"))
                    .col(schema::text("body"))
                    .col(schema::uuid_null("author_user_id"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "compliance_incident_entries",
            "compliance_incident_entries_incident_idx",
            &["tenant_id", "incident_id", "id"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("compliance_subject_requests")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("kind", 16))
                    .col(schema::string_len("channel", 16))
                    .col(schema::string_len("subject_kind", 8))
                    .col(schema::string_len("subject", 254))
                    .col(schema::timestamp_with_time_zone("received_at"))
                    .col(schema::timestamp_with_time_zone("due_at"))
                    .col(schema::string_len("status", 16))
                    .col(schema::text_null("reason"))
                    .col(schema::text_null("notes"))
                    .col(schema::uuid_null("handled_by"))
                    .col(schema::timestamp_with_time_zone_null("closed_at"))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "compliance_subject_requests",
            "compliance_subject_requests_due_idx",
            &["tenant_id", "status", "due_at"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_all(
            manager,
            &[
                "compliance_subject_requests",
                "compliance_incident_entries",
                "compliance_incidents",
                "compliance_purposes",
            ],
        )
        .await
    }
}

/// Mã theo dõi quảng cáo của website, banner đồng ý cookie và bằng chứng đồng ý ẩn danh (D-235,
/// D-361).
struct Tracking;

impl MigrationName for Tracking {
    fn name(&self) -> &str {
        "m0014_tracking_and_cookies"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Tracking {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("compliance_tracking_tags")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("provider", 32))
                    .col(schema::string_len("tracking_id", 64))
                    .col(schema::string_len("status", 16))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "compliance_tracking_tags",
            "compliance_tracking_tags_key",
            &["tenant_id", "org_id", "provider", "tracking_id"],
            true,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("compliance_cookie_settings")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::boolean("enabled"))
                    .col(schema::text("categories"))
                    .col(schema::integer("policy_version"))
                    .col(schema::string_len("signature", 64))
                    .col(schema::string_len("position", 16))
                    .col(schema::uuid_null("updated_by"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("tenant_id").col("org_id"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("compliance_cookie_consents")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("consent_id", 64))
                    .col(schema::text("categories"))
                    .col(schema::integer("policy_version"))
                    .col(schema::string_len_null("ip_hash", 32))
                    .col(schema::uuid_null("user_id"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "compliance_cookie_consents",
            "compliance_cookie_consents_consent_idx",
            &["tenant_id", "consent_id"],
            false,
        )
        .await?;
        index(
            manager,
            "compliance_cookie_consents",
            "compliance_cookie_consents_org_idx",
            &["tenant_id", "org_id", "created_at"],
            false,
        )
        .await?;
        index(
            manager,
            "compliance_cookie_consents",
            "compliance_cookie_consents_sweep_idx",
            &["created_at"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_all(
            manager,
            &[
                "compliance_cookie_consents",
                "compliance_cookie_settings",
                "compliance_tracking_tags",
            ],
        )
        .await
    }
}

/// Vụ việc (bước 37, D-363): báo vi phạm sở hữu trí tuệ / hàng giả / hàng cấm, khiếu nại của người
/// tiêu dùng, người bị phản ánh, lưu vết (chỉ thêm) và sổ cung cấp thông tin cho cơ quan có thẩm quyền.
struct Cases;

impl MigrationName for Cases {
    fn name(&self) -> &str {
        "m0015_cases"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Cases {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("compliance_cases")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("kind", 24))
                    .col(schema::string_len("target", 200))
                    .col(schema::uuid_null("respondent_org_id"))
                    .col(schema::string_len("reporter_kind", 16))
                    .col(schema::uuid_null("reporter_user_id"))
                    .col(schema::string_len_null("reporter_name", 200))
                    .col(schema::string_len_null("reporter_email", 320))
                    .col(schema::string_len_null("reporter_phone", 32))
                    .col(schema::string_len("channel", 16))
                    .col(schema::text("claim"))
                    .col(schema::text("rights"))
                    .col(schema::string_len("status", 24))
                    .col(schema::boolean("restricted"))
                    .col(schema::string_len("decision", 24))
                    .col(schema::text_null("decision_reason"))
                    .col(schema::timestamp_with_time_zone("ack_due_at"))
                    .col(schema::timestamp_with_time_zone_null("acknowledged_at"))
                    .col(schema::timestamp_with_time_zone("respond_by"))
                    .col(schema::timestamp_with_time_zone("decide_by"))
                    .col(schema::timestamp_with_time_zone_null("decided_at"))
                    .col(schema::uuid_null("decided_by"))
                    .col(schema::string_len("tracking_hash", 64))
                    .col(schema::string_len_null("spam_reason", 100))
                    .col(schema::string_len("overdue_notified", 16))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        for (name, columns) in [
            ("compliance_cases_due_idx", &["tenant_id", "status", "decide_by"][..]),
            ("compliance_cases_target_idx", &["tenant_id", "target"][..]),
            ("compliance_cases_respondent_idx", &["tenant_id", "respondent_org_id", "id"][..]),
            ("compliance_cases_list_idx", &["tenant_id", "id"][..]),
            // Job nhắc hạn quét mọi khách của một database.
            ("compliance_cases_sweep_idx", &["status", "decide_by"][..]),
        ] {
            index(manager, "compliance_cases", name, columns, false).await?;
        }
        manager
            .create_table(
                Table::create()
                    .table("compliance_case_respondents")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("case_id"))
                    .col(schema::uuid("user_id"))
                    .primary_key(Index::create().col("tenant_id").col("case_id").col("user_id"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "compliance_case_respondents",
            "compliance_case_respondents_user_idx",
            &["tenant_id", "user_id", "case_id"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("compliance_case_entries")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("case_id"))
                    .col(schema::string_len("kind", 16))
                    .col(schema::string_len("author_kind", 16))
                    .col(schema::uuid_null("author_user_id"))
                    .col(schema::text("body"))
                    .col(schema::text("files"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "compliance_case_entries",
            "compliance_case_entries_case_idx",
            &["tenant_id", "case_id", "id"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("compliance_disclosures")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid_null("case_id"))
                    .col(schema::string_len("authority", 200))
                    .col(schema::string_len("request_reference", 100))
                    .col(schema::text("legal_basis"))
                    .col(schema::timestamp_with_time_zone("requested_at"))
                    .col(schema::string_len("subject", 64))
                    .col(schema::text("fields"))
                    .col(schema::uuid("disclosed_by"))
                    .col(schema::timestamp_with_time_zone("disclosed_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "compliance_disclosures",
            "compliance_disclosures_at_idx",
            &["tenant_id", "disclosed_at"],
            false,
        )
        .await?;
        index(
            manager,
            "compliance_disclosures",
            "compliance_disclosures_list_idx",
            &["tenant_id", "id"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_all(
            manager,
            &[
                "compliance_disclosures",
                "compliance_case_entries",
                "compliance_case_respondents",
                "compliance_cases",
            ],
        )
        .await
    }
}

/// Index đếm người đồng ý / đã rút theo mục đích (`GET /purposes`, D-382): đọc thẳng từ index, không
/// quét bảng đồng ý.
struct ConsentStatsIndex;

impl MigrationName for ConsentStatsIndex {
    fn name(&self) -> &str {
        "m0018_consent_stats_index"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for ConsentStatsIndex {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        index(
            manager,
            "compliance_consents",
            "compliance_consents_stats_idx",
            &["tenant_id", "purpose", "state", "subject_key"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("compliance_consents_stats_idx")
                    .table(Alias::new("compliance_consents"))
                    .to_owned(),
            )
            .await
    }
}

/// Ghi chú của người duyệt khi từ chối / thu hồi hồ sơ định danh (D-20261118-ui-debt-batch-3).
struct KycReviewNote;

impl MigrationName for KycReviewNote {
    fn name(&self) -> &str {
        "m0019_kyc_review_note"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for KycReviewNote {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("compliance_kyc_profiles")
                    .add_column(schema::text_null("review_note"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("compliance_kyc_profiles")
                    .drop_column("review_note")
                    .to_owned(),
            )
            .await
    }
}
