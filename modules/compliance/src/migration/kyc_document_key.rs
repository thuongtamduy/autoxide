//! `m0020_kyc_verified_document`: một giấy tờ chỉ xác thực tự động cho một hồ sơ (rà đồng thời mục 14).

use autoxide_kernel::migration::prelude::*;

/// Cột `verified_document_hash` = `document_hash` khi hồ sơ được nhà cung cấp xác thực, trống ở mọi
/// trạng thái khác; unique `(tenant_id, verified_document_hash)` (NULL không đụng nhau trên cả SQLite
/// và Postgres). Trước đây hai hồ sơ cùng một CCCD xác thực cùng lúc đều thấy "chưa ai dùng giấy tờ
/// này" và cùng `verified`. Dữ liệu cũ: mỗi giấy tờ chỉ hồ sơ đã xác thực có ID nhỏ nhất nhận khoá.
pub(super) struct KycVerifiedDocument;

const TABLE: &str = "compliance_kyc_profiles";
const INDEX: &str = "compliance_kyc_profiles_verified_document_idx";

impl MigrationName for KycVerifiedDocument {
    fn name(&self) -> &str {
        "m0020_kyc_verified_document"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for KycVerifiedDocument {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(TABLE)
                    .add_column(schema::string_len_null("verified_document_hash", 64))
                    .to_owned(),
            )
            .await?;
        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE compliance_kyc_profiles SET verified_document_hash = document_hash \
                 WHERE status = 'verified' AND document_hash IS NOT NULL AND NOT EXISTS ( \
                 SELECT 1 FROM compliance_kyc_profiles AS older \
                 WHERE older.tenant_id = compliance_kyc_profiles.tenant_id \
                 AND older.document_hash = compliance_kyc_profiles.document_hash \
                 AND older.status = 'verified' AND older.id < compliance_kyc_profiles.id)",
            )
            .await?;
        super::index(manager, TABLE, INDEX, &["tenant_id", "verified_document_hash"], true).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_index(Index::drop().name(INDEX).table(Alias::new(TABLE)).to_owned()).await?;
        manager
            .alter_table(
                Table::alter().table(TABLE).drop_column("verified_document_hash").to_owned(),
            )
            .await
    }
}
