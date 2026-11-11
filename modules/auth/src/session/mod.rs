//! Phiên đăng nhập: cấp access token và refresh token, xoay vòng refresh token,
//! phát hiện token bị dùng lại, đăng xuất.

pub(crate) mod entity;
mod service;

pub(crate) use service::{SessionService, Tokens};
