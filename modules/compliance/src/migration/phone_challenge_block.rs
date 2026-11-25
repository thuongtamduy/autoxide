//! `m0021_phone_challenge_block`: client (một IPv4 hay một khối /64 IPv6) của người xin mã đăng nhập (PT-7, D-20261125-otp-wrong-by-block).

use autoxide_kernel::migration::prelude::*;

/// Cột `requester_block`: dấu (HMAC, 22 ký tự, `ip_hash::client_fingerprint`) của client xin mã — một địa chỉ IPv4
/// hay một khối /64 IPv6; trống khi
/// không biết địa chỉ và với mã xác thực số của người đã đăng nhập. Lần nhập sai của một số đếm theo khối
/// này, nên người lạ đoán sai trên mã họ xin không khoá chủ số ở địa chỉ khác.
pub(super) struct PhoneChallengeBlock;

const TABLE: &str = "compliance_phone_challenges";

impl MigrationName for PhoneChallengeBlock {
    fn name(&self) -> &str {
        "m0021_phone_challenge_block"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for PhoneChallengeBlock {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(TABLE)
                    .add_column(schema::string_len_null("requester_block", 32))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(Table::alter().table(TABLE).drop_column("requester_block").to_owned())
            .await
    }
}
