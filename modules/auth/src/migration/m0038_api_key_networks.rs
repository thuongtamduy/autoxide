//! API key chi tiết (bước 54 P4): dải IP được gọi (`allowed_ips`, danh sách CIDR cách nhau bằng dấu
//! phẩy; NULL là mọi địa chỉ) và hạn bắt buộc — key cũ chưa có hạn được đặt hạn 365 ngày kể từ lúc
//! nâng cấp (CHANGELOG báo trước), để không key nào sống mãi.

use autoxide_kernel::migration::prelude::*;
use sea_orm::sea_query::{Expr, Query};

pub(crate) struct Migration;

/// Hạn đặt cho key cũ không có hạn: trùng mặc định của `AUTOXIDE_AUTH_API_KEY_MAX_DAYS`.
const LEGACY_KEY_DAYS: i64 = 365;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0038_api_key_networks"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("auth_api_key")
                    .add_column(schema::text_null("allowed_ips"))
                    .to_owned(),
            )
            .await?;
        // Giá trị tính trong ứng dụng (UTC), không dùng hàm thời gian của database: hai database
        // viết khác nhau.
        let expires_at = chrono::Utc::now() + chrono::Duration::days(LEGACY_KEY_DAYS);
        let update = Query::update()
            .table("auth_api_key")
            .value("expires_at", expires_at)
            .and_where(Expr::col("expires_at").is_null())
            .to_owned();
        manager.exec_stmt(update).await
    }

    // Hạn đã đặt cho key cũ giữ nguyên: bản cũ đọc được cột này như mọi hạn khác.
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(Table::alter().table("auth_api_key").drop_column("allowed_ips").to_owned())
            .await
    }
}
