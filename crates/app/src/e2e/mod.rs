//! Test đầu-cuối của chương trình thật: dựng mọi module như `autoxide serve`, gọi qua HTTP, kiểm
//! cả database, nơi lưu file và audit log. Nằm trong `src/` (không ở `crates/app/tests/`) vì cần
//! phần bên trong của binary, như danh sách module ở `modules.rs`.

pub(crate) mod access;
pub(crate) mod billing;
pub(crate) mod guides;
pub(crate) mod invoices;
pub(crate) mod ledger_flows;
pub(crate) mod media;
pub(crate) mod migrations;
pub(crate) mod notifications;
pub(crate) mod plans;
pub(crate) mod signup;
pub(crate) mod tenant_move;
