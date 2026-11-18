//! Test đầu-cuối của chương trình thật: dựng mọi module như `autoxide serve`, gọi qua HTTP, kiểm
//! cả database, nơi lưu file và audit log. Nằm trong `src/` (không ở `crates/app/tests/`) vì cần
//! phần bên trong của binary, như danh sách module ở `modules.rs`.

pub(crate) mod access;
pub(crate) mod agents;
pub(crate) mod analytics;
pub(crate) mod automations;
pub(crate) mod billing;
pub(crate) mod bookings;
pub(crate) mod cases;
pub(crate) mod catalog;
pub(crate) mod child_context;
pub(crate) mod guides;
pub(crate) mod invoices;
pub(crate) mod invoices_print;
pub(crate) mod ledger_flows;
pub(crate) mod media;
pub(crate) mod migrations;
pub(crate) mod moit;
pub(crate) mod notifications;
pub(crate) mod plans;
pub(crate) mod posting_verification;
pub(crate) mod privacy;
pub(crate) mod records;
pub(crate) mod relocation;
pub(crate) mod reporting;
pub(crate) mod signup;
pub(crate) mod tenant_move;
