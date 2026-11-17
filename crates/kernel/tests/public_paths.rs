//! Đường dẫn công khai của kernel (đặc tả 32 A2, D-167, D-247): module ở repo riêng viết
//! `autoxide_kernel::<module>::<kiểu>`. File bên trong kernel được xếp theo nhóm riêng tư
//! (`contracts/`, `data/`, `security/`, `ops/`, `values/`), nhưng đường dẫn công khai thì không đổi.
//! Test này `use` mọi module công khai cùng một kiểu tiêu biểu của nó: mất đường dẫn nào là lỗi
//! biên dịch.

#![allow(unused_imports)]

use autoxide_kernel::access::Permission;
use autoxide_kernel::ai::TextModel;
use autoxide_kernel::antispam::AntispamConfig;
use autoxide_kernel::bulk::BulkAction;
use autoxide_kernel::config::Config;
use autoxide_kernel::csv::line;
use autoxide_kernel::database::Database;
use autoxide_kernel::doctor::CHECK_TIMEOUT;
use autoxide_kernel::einvoice::EInvoiceProvider;
use autoxide_kernel::email::MAX_LENGTH as EMAIL_MAX_LENGTH;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::EventInfo;
use autoxide_kernel::http::ClientInfo;
use autoxide_kernel::i18n::Language;
use autoxide_kernel::idempotency::IdempotencyStore;
use autoxide_kernel::identity::IdentityVerifier;
use autoxide_kernel::ip_hash::prefix;
use autoxide_kernel::job::Job;
use autoxide_kernel::list_exports::ExportSource;
use autoxide_kernel::logging::LogError;
use autoxide_kernel::mail::Mailer;
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::metrics::Metrics;
use autoxide_kernel::migration::MigrationError;
use autoxide_kernel::module::Module;
use autoxide_kernel::money::Currency;
use autoxide_kernel::pagination::PageRequest;
use autoxide_kernel::payment::PaymentGateway;
use autoxide_kernel::privacy::PersonalData;
use autoxide_kernel::public_url::PublicUrl;
use autoxide_kernel::push::{MobilePushSender, WebPushSender};
use autoxide_kernel::reference::MAX_LENGTH as REFERENCE_MAX_LENGTH;
use autoxide_kernel::resolve::CodeResolver;
use autoxide_kernel::resource::ResourceResolver;
use autoxide_kernel::response::Data;
use autoxide_kernel::rows::lock;
use autoxide_kernel::search::normalize;
use autoxide_kernel::secret::SecretKey;
use autoxide_kernel::setup::SetupStep;
use autoxide_kernel::sms::SmsSender;
use autoxide_kernel::storage::Storage;
use autoxide_kernel::task::TaskArgument;
use autoxide_kernel::telemetry::TelemetryError;
use autoxide_kernel::template::TemplateEngine;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::throttle::Outcome;
use autoxide_kernel::tls::install_crypto_provider;
use autoxide_kernel::tokens::Issued;
use autoxide_kernel::transfer::TransferError;
use autoxide_kernel::version::etag;
use autoxide_kernel::zns::ZnsSender;

#[test]
fn moi_duong_dan_cong_khai_cua_kernel_con_nguyen() {
    // Dùng vài thứ để chắc chúng là đúng thứ được kỳ vọng, không chỉ trùng tên.
    install_crypto_provider();
    let permission = Permission::new("notes.read", "Read notes.");
    assert_eq!(permission.name(), "notes.read");
}
