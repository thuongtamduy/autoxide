//! Service của `shipping`: MỌI logic nằm ở đây và ở các file con (`addresses`, …). Handler REST và
//! tool MCP chỉ lấy dữ liệu vào, kiểm permission, gọi service — nên hai đường cho cùng một kết quả
//! (luật kiến trúc 8).
//!
//! Mọi hàm chạm database nhận `&TenantContext` và lọc mọi truy vấn bằng `.scoped(tenant)` (bước
//! 33). Không gọi hãng vận chuyển (mạng) trong transaction ghi.

use std::sync::Arc;

use autoxide_address::api::AdministrativeUnits;
use autoxide_kernel::error::FieldError;
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::{DbErr, SqlErr};

use crate::config::ShippingConfig;

/// Mọi việc của module. Không giữ database: database là của khách, đi theo `TenantContext` của
/// từng lời gọi.
pub(crate) struct ShippingService {
    pub(crate) config: ShippingConfig,
    /// `dyn AdministrativeUnits` của module `address` khi đã cài: kiểm địa chỉ Việt Nam.
    pub(crate) units: Option<Arc<dyn AdministrativeUnits>>,
}

impl ShippingService {
    pub(crate) fn new(config: ShippingConfig, units: Option<Arc<dyn AdministrativeUnits>>) -> Self {
        ShippingService { config, units }
    }
}

/// Bây giờ, cắt tới micro giây: Postgres lưu tới micro giây, SQLite lưu nguyên. Cắt trước để hai
/// database trả về cùng một giá trị và so bằng nhau được.
pub(crate) fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

/// Kiểm độ dài chuỗi theo ký tự.
pub(crate) fn check_length(
    problems: &mut Vec<FieldError>,
    field: &str,
    value: &str,
    min: usize,
    max: usize,
) {
    let count = value.chars().count();
    if count < min || count > max {
        problems.push(
            FieldError::new(field, "length", "must be {min} to {max} characters long")
                .with_param("min", min)
                .with_param("max", max),
        );
    }
}

/// Lỗi vi phạm unique: thông báo của database.
pub(crate) fn unique_violation(error: &DbErr) -> Option<String> {
    match error.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(message)) => Some(message),
        _ => None,
    }
}
