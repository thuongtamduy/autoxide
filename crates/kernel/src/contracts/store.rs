//! Mua trong ứng dụng qua App Store và Google Play (bước 69 P4, D-20261119-step69-store): trait trung
//! lập để kiểm giao dịch với máy chủ của cửa hàng ứng dụng và nhận thông báo của họ (App Store Server
//! Notifications v2, Google Real-time Developer Notifications) có kiểm chữ ký.
//!
//! Bộ cài đặt thật (App Store Server API, Google Play Developer API) là module riêng cần tài khoản nhà
//! phát triển và key của chủ dự án, góp bằng `context.contribute::<dyn StorePurchases>(…)` và đăng ký
//! TRƯỚC module `plans`. Test dùng `FakeStorePurchases` của testkit.
//!
//! Tiền do Apple / Google thu và giữ (đối tác có giấy phép) — đúng D-187: core không giữ tiền.

use chrono::{DateTime, Utc};

use crate::error::Error;

/// Cửa hàng ứng dụng.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AppStore {
    /// Apple App Store (`app_store`).
    Apple,
    /// Google Play (`google_play`).
    Google,
}

impl AppStore {
    pub const ALL: [AppStore; 2] = [AppStore::Apple, AppStore::Google];

    /// Tên ổn định trong API và database.
    pub fn as_str(self) -> &'static str {
        match self {
            AppStore::Apple => "app_store",
            AppStore::Google => "google_play",
        }
    }

    pub fn parse(text: &str) -> Option<AppStore> {
        AppStore::ALL.into_iter().find(|store| store.as_str() == text)
    }
}

/// Trạng thái của một giao dịch (thuê bao) theo cửa hàng.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurchaseState {
    /// Đang có hiệu lực tới `expires_at`.
    Active,
    /// Hết hạn, không gia hạn nữa.
    Expired,
    /// Cửa hàng đã hoàn tiền: thu hồi quyền dùng ngay.
    Refunded,
    /// Cửa hàng thu hồi (vd chia sẻ gia đình bị gỡ): thu hồi quyền dùng ngay.
    Revoked,
}

/// Một giao dịch đã được cửa hàng xác nhận (đã kiểm chữ ký / gọi máy chủ của họ).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StorePurchase {
    pub store: AppStore,
    /// Mã sản phẩm khai trên cửa hàng.
    pub product_id: String,
    /// Mã gốc của chuỗi thuê bao (`originalTransactionId` / `purchaseToken` đầu tiên): không đổi qua
    /// các lần gia hạn — khoá để gọi lại không cấp trùng.
    pub original_transaction_id: String,
    /// Mã của lần mua / gia hạn này.
    pub transaction_id: String,
    pub state: PurchaseState,
    /// Hết hiệu lực lúc này (thuê bao); `None` với mua một lần.
    pub expires_at: Option<DateTime<Utc>>,
    /// `true`: môi trường thử của cửa hàng (sandbox / license testing).
    pub sandbox: bool,
}

impl StorePurchase {
    pub fn new(
        store: AppStore,
        product_id: impl Into<String>,
        original_transaction_id: impl Into<String>,
        transaction_id: impl Into<String>,
        state: PurchaseState,
        expires_at: Option<DateTime<Utc>>,
    ) -> Self {
        StorePurchase {
            store,
            product_id: product_id.into(),
            original_transaction_id: original_transaction_id.into(),
            transaction_id: transaction_id.into(),
            state,
            expires_at,
            sandbox: false,
        }
    }

    #[must_use]
    pub fn sandbox(mut self, sandbox: bool) -> Self {
        self.sandbox = sandbox;
        self
    }
}

/// Một thông báo máy chủ của cửa hàng, đã kiểm chữ ký.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StoreNotification {
    /// Mã thông báo của cửa hàng (`notificationUUID` / `messageId`): nhận lại không xử lý hai lần.
    pub id: String,
    /// Trạng thái mới của giao dịch.
    pub purchase: StorePurchase,
}

impl StoreNotification {
    pub fn new(id: impl Into<String>, purchase: StorePurchase) -> Self {
        StoreNotification { id: id.into(), purchase }
    }
}

/// Lỗi 401 khi thông báo của cửa hàng không có chữ ký hợp lệ — bộ kết nối trả khi kiểm chữ ký hỏng.
pub fn signature_invalid() -> Error {
    Error::unauthenticated(
        "store_signature_invalid",
        "The store notification is not signed by the store.",
    )
}

/// Kết nối tới một cửa hàng ứng dụng.
#[async_trait::async_trait]
pub trait StorePurchases: Send + Sync {
    fn store(&self) -> AppStore;

    /// Kiểm bằng chứng mua app gửi lên (giao dịch đã ký của StoreKit 2 / `purchaseToken` của Google
    /// Play) với cửa hàng. Bằng chứng sai, giả hay của ứng dụng khác → lỗi `validation`.
    async fn verify(&self, proof: &str) -> Result<StorePurchase, Error>;

    /// Đọc và kiểm chữ ký một thông báo máy chủ của cửa hàng (`headers`: header của request webhook,
    /// `body`: thân nguyên vẹn). Chữ ký sai → lỗi `unauthenticated`.
    async fn notification(
        &self,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> Result<StoreNotification, Error>;
}
