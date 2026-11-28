//! `m0024_kyc_seller_details`: hồ sơ định danh ghi các mục NĐ 248/2026/NĐ-CP Điều 18 (LG-3c,
//! D-20261128-kyc-article-18) — cần đối chiếu văn bản gốc.

use autoxide_kernel::migration::prelude::*;

/// Năm cột trống được (hồ sơ cũ không có): `birth_date` (`YYYY-MM-DD`, cá nhân), `address` (trụ sở / địa chỉ),
/// `representative_name` (người đại diện của tổ chức, hộ kinh doanh), `document_country` (nước cấp giấy tờ, mã
/// ISO 3166-1 hai chữ), `document_expires_on` (`YYYY-MM-DD`, hạn của giấy tờ — hộ chiếu phải còn hạn đủ lâu).
/// Ngày lưu chữ `YYYY-MM-DD`: không có giờ, so sánh theo chữ đúng thứ tự trên cả SQLite và Postgres.
pub(super) struct KycSellerDetails;

const TABLE: &str = "compliance_kyc_profiles";
const COLUMNS: [(&str, u32); 5] = [
    ("birth_date", 10),
    ("address", 500),
    ("representative_name", 200),
    ("document_country", 2),
    ("document_expires_on", 10),
];

impl MigrationName for KycSellerDetails {
    fn name(&self) -> &str {
        "m0024_kyc_seller_details"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for KycSellerDetails {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite chỉ nhận một `ADD COLUMN` mỗi câu `ALTER TABLE`: thêm từng cột.
        for (column, length) in COLUMNS {
            manager
                .alter_table(
                    Table::alter()
                        .table(TABLE)
                        .add_column(schema::string_len_null(column, length))
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (column, _) in COLUMNS.iter().rev() {
            manager
                .alter_table(Table::alter().table(TABLE).drop_column(*column).to_owned())
                .await?;
        }
        Ok(())
    }
}
