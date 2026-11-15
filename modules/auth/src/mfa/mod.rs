//! MFA (xác thực nhiều lớp): TOTP — mã 6 chữ số đổi mỗi 30 giây trong ứng dụng
//! authenticator trên điện thoại (RFC 6238) — và recovery code dùng một lần khi mất
//! thiết bị.

pub(crate) mod entity;
mod recovery;
pub(crate) mod service;

pub(crate) use service::{MfaService, MfaStatus, TotpSetup};
