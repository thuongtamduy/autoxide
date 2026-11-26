//! `m0023_phone_otp_state`: trạng thái chống đoán mã đăng nhập SMS theo số (PT-7, D-20261126-otp-nist). Cả hai bảng chỉ
//! giữ khoá của số (`SecretKey::phone_key`: HMAC-SHA256 theo khách), không giữ số.
//!
//! - `compliance_phone_known_clients`: client (`ip_hash::client_fingerprint`, như
//!   `compliance_phone_challenges.requester_block`) đã đăng nhập đúng bằng mã của số, lần đúng cuối; giữ
//!   [`KNOWN_CLIENT_DAYS`](crate::sign_in_codes::KNOWN_CLIENT_DAYS) ngày.
//! - `compliance_phone_failures`: số lần nhập sai liên tiếp của số kể từ lần đúng gần nhất — không theo cửa sổ thời
//!   gian; đủ [`MAX_CONSECUTIVE_FAILURES`](crate::sign_in_codes::MAX_CONSECUTIVE_FAILURES) thì số bị khoá với mọi
//!   client tới khi chủ số xác thực lại số từ tài khoản đã đăng nhập.

use autoxide_kernel::migration::prelude::*;

pub(super) struct PhoneOtpState;

const KNOWN: &str = "compliance_phone_known_clients";
const FAILURES: &str = "compliance_phone_failures";

impl MigrationName for PhoneOtpState {
    fn name(&self) -> &str {
        "m0023_phone_otp_state"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for PhoneOtpState {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(KNOWN)
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("phone_key", 64))
                    .col(schema::string_len("requester_block", 32))
                    .col(schema::timestamp_with_time_zone("last_success_at"))
                    .to_owned(),
            )
            .await?;
        super::index(
            manager,
            KNOWN,
            "compliance_phone_known_clients_unique",
            &["tenant_id", "phone_key", "requester_block"],
            true,
        )
        .await?;
        super::index(
            manager,
            KNOWN,
            "compliance_phone_known_clients_sweep_idx",
            &["last_success_at"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table(FAILURES)
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("phone_key", 64))
                    .col(schema::integer("failures"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        super::index(
            manager,
            FAILURES,
            "compliance_phone_failures_unique",
            &["tenant_id", "phone_key"],
            true,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(FAILURES).to_owned()).await?;
        manager.drop_table(Table::drop().table(KNOWN).to_owned()).await
    }
}
