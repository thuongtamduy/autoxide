//! Service layer của `orders`: mọi logic ở đây (và ở các file tính năng `carts.rs`, …); handler
//! REST, tool MCP, job và event đều gọi vào đây.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_address::api::AdministrativeUnits;
use autoxide_kernel::event::Outbox;
use autoxide_kernel::i18n::Translations;
use autoxide_kernel::reasons::ReasonCodes;
use autoxide_kernel::secret::SecretKey;
use autoxide_kernel::tenancy::DatabaseRegistry;
use autoxide_ledger::api::Ledger;
use autoxide_media::api::Media;
use autoxide_organizations::api::OrgDirectory;
use autoxide_shipping::api::AddressBook;
use autoxide_support::api::SupportIntake;

use crate::config::OrdersConfig;
use crate::extensions::Extensions;
use crate::pricing::Sources;

/// Các phần service cần, lấy lúc `start`.
pub(crate) struct Parts {
    pub outbox: Outbox,
    pub sources: Sources,
    pub address_book: Option<Arc<dyn AddressBook>>,
    pub units: Option<Arc<dyn AdministrativeUnits>>,
    pub extensions: Arc<Extensions>,
    pub databases: DatabaseRegistry,
    pub ledger: Option<Arc<dyn Ledger>>,
    pub secret: SecretKey,
    pub orgs: Option<Arc<dyn OrgDirectory>>,
    pub reasons: ReasonCodes,
    pub media: Option<Arc<dyn Media>>,
    pub sms: Option<Arc<dyn autoxide_kernel::sms::SmsSender>>,
    pub support: Option<Arc<dyn SupportIntake>>,
    pub translations: Translations,
    pub invoices: Option<Arc<dyn autoxide_invoices::api::Invoices>>,
    pub notifier: Option<Arc<dyn autoxide_notifications::api::Notifier>>,
    pub posting: autoxide_kernel::access::PostingVerification,
    pub text_model: Option<Arc<dyn autoxide_kernel::ai::TextModel>>,
    pub custom_fields: autoxide_kernel::custom_fields::CustomFields,
}

pub(crate) struct OrdersService {
    pub outbox: Outbox,
    pub sources: Sources,
    pub address_book: Option<Arc<dyn AddressBook>>,
    pub units: Option<Arc<dyn AdministrativeUnits>>,
    pub extensions: Arc<Extensions>,
    pub databases: DatabaseRegistry,
    pub ledger: Option<Arc<dyn Ledger>>,
    pub secret: SecretKey,
    pub orgs: Option<Arc<dyn OrgDirectory>>,
    pub reasons: ReasonCodes,
    pub media: Option<Arc<dyn Media>>,
    /// Gửi mã một lần cho người mua vãng lai (chưa cài bộ gửi SMS: không tra đơn được).
    pub sms: Option<Arc<dyn autoxide_kernel::sms::SmsSender>>,
    /// Mở phiếu hỗ trợ từ trang đơn (chưa cài `support`: 404).
    pub support: Option<Arc<dyn SupportIntake>>,
    pub translations: Translations,
    /// Lập hoá đơn điện tử của đơn (chưa cài `invoices`: không lập).
    pub invoices: Option<Arc<dyn autoxide_invoices::api::Invoices>>,
    /// Thư xác nhận, cập nhật trạng thái (chưa cài `notifications`: không gửi).
    pub notifier: Option<Arc<dyn autoxide_notifications::api::Notifier>>,
    /// Tài khoản đã xác thực để đăng nội dung chưa (NĐ 147/2024) — đánh giá.
    pub posting: autoxide_kernel::access::PostingVerification,
    /// Bản nháp tóm tắt đánh giá (chưa cài `ai`: không tóm tắt).
    pub text_model: Option<Arc<dyn autoxide_kernel::ai::TextModel>>,
    /// Ô người mua nhập lúc đặt hàng (trường tự định nghĩa, D-235).
    pub custom_fields: autoxide_kernel::custom_fields::CustomFields,
    pub config: OrdersConfig,
}

impl OrdersService {
    pub(crate) fn new(parts: Parts, config: OrdersConfig) -> Self {
        OrdersService {
            outbox: parts.outbox,
            sources: parts.sources,
            address_book: parts.address_book,
            units: parts.units,
            extensions: parts.extensions,
            databases: parts.databases,
            ledger: parts.ledger,
            secret: parts.secret,
            orgs: parts.orgs,
            reasons: parts.reasons,
            media: parts.media,
            sms: parts.sms,
            support: parts.support,
            translations: parts.translations,
            invoices: parts.invoices,
            notifier: parts.notifier,
            posting: parts.posting,
            text_model: parts.text_model,
            custom_fields: parts.custom_fields,
            config,
        }
    }
}

/// Bây giờ, cắt tới micro giây: Postgres lưu tới micro giây, SQLite lưu nguyên — cắt trước để hai
/// database trả cùng một giá trị (`ETag` / `If-Match` so `updated_at`).
pub(crate) fn now() -> Timestamp {
    Timestamp::now()
}
