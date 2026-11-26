//! `m0023_phone_known_clients`: client đã đăng nhập thành công bằng mã SMS của một số (PT-7, D-20261126-otp-nist).
//!
//! Một dòng mỗi (khách, số, client): dấu HMAC của số (`phone_hash`, không lưu số) và dấu client
//! (`ip_hash::client_fingerprint` như `compliance_phone_challenges.requester_block`), lần đăng nhập đúng cuối. Giữ
//! [`KNOWN_CLIENT_DAYS`](crate::sign_in_codes::KNOWN_CLIENT_DAYS) ngày. Client quen của chủ số không bị trần chung của
//! số chặn — người lạ nhập sai từ nhiều nơi không khoá được chủ số ở mạng họ hay dùng.

use autoxide_kernel::migration::prelude::*;

pub(super) struct PhoneKnownClients;

const TABLE: &str = "compliance_phone_known_clients";

impl MigrationName for PhoneKnownClients {
    fn name(&self) -> &str {
        "m0023_phone_known_clients"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for PhoneKnownClients {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(TABLE)
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("phone_hash", 64))
                    .col(schema::string_len("requester_block", 32))
                    .col(schema::timestamp_with_time_zone("last_success_at"))
                    .to_owned(),
            )
            .await?;
        super::index(
            manager,
            TABLE,
            "compliance_phone_known_clients_unique",
            &["tenant_id", "phone_hash", "requester_block"],
            true,
        )
        .await?;
        super::index(
            manager,
            TABLE,
            "compliance_phone_known_clients_age_idx",
            &["last_success_at"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(TABLE).to_owned()).await
    }
}
