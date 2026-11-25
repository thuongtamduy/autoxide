//! Service của module `analytics`: phần dùng chung của nhận sự kiện (`ingest.rs`), gộp
//! (`rollup.rs`), báo cáo (`reports.rs`), dọn và quyền riêng tư (`privacy.rs`).

use autoxide_kernel::time::{Date, Timestamp};
use std::sync::Arc;

use autoxide_compliance::api::{ConsentKey, ConsentLedger, ConsentSubject};
use autoxide_kernel::analytics::AnalyticsCatalog;
use autoxide_kernel::error::Error;
use autoxide_kernel::secret::SecretKey;
use autoxide_kernel::tenancy::{DatabaseRegistry, TenantContext};
use uuid::Uuid;

use crate::config::AnalyticsConfig;

pub(crate) use crate::api::{PAGE_VIEW, PERSONAL_PURPOSE};

pub(crate) struct AnalyticsService {
    pub config: AnalyticsConfig,
    /// Loại sự kiện và phễu của mọi module.
    pub catalog: AnalyticsCatalog,
    pub secret: SecretKey,
    /// Sổ đồng ý của `compliance`; không cài thì không theo dõi theo người.
    pub consents: Option<Arc<dyn ConsentLedger>>,
    /// Sổ database: tên miền đã xác minh của khách, ngữ cảnh khách cho job.
    pub registry: DatabaseRegistry,
    /// Cấu hình theo khách (`analytics.allowed_hosts`); `None` trong test dựng tay.
    pub settings: Option<Arc<dyn autoxide_kernel::tenancy::TenantSettings>>,
}

impl AnalyticsService {
    /// Người dùng `user_id` đã đồng ý theo dõi theo người (và chưa rút).
    pub(crate) async fn personal_allowed(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<bool, Error> {
        match &self.consents {
            Some(ledger) => {
                ledger
                    .is_granted(tenant.id(), &ConsentSubject::User(user_id), &personal_key())
                    .await
            }
            None => Ok(false),
        }
    }
}

/// Khoá đồng ý `analytics.personal` (mọi phạm vi, mọi kênh).
pub(crate) fn personal_key() -> ConsentKey {
    ConsentKey::new(PERSONAL_PURPOSE, "", "")
}

/// Ngày UTC `YYYY-MM-DD` của `at`.
pub(crate) fn day_of(at: Timestamp) -> String {
    at.format("%Y-%m-%d").to_string()
}

/// Ngày `YYYY-MM-DD`.
pub(crate) fn day_text(day: Date) -> String {
    day.format("%Y-%m-%d").to_string()
}

pub(crate) fn now() -> Timestamp {
    Timestamp::now()
}
