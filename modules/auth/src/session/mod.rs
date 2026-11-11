//! Phiên đăng nhập: cấp access token và refresh token, xoay vòng refresh token,
//! phát hiện token bị dùng lại, đăng xuất; khoá tạm việc đoán mật khẩu.

mod challenge;
pub(crate) mod entity;
mod service;
mod throttle;

pub(crate) use service::{LoginOutcome, SessionService, Tokens, bearer_token};
pub(crate) use throttle::LoginThrottle;
