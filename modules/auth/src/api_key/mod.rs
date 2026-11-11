//! API key: cách xác thực cho máy và AI agent, không cần đăng nhập bằng mật khẩu.

mod entity;
mod service;

pub(crate) use service::{API_KEY_PREFIX, ApiKey, ApiKeyService, NewApiKey};
