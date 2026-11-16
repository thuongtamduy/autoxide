//! API key: cách xác thực cho máy và AI agent, không cần đăng nhập bằng mật khẩu.

mod entity;
mod service;

pub(crate) use service::{
    ApiKey, ApiKeyService, CreatedApiKey, NewApiKey, Reauth, invalid_api_key, revoke_all_in,
    tenant_of,
};
