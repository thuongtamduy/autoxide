//! API key: cách xác thực cho máy và AI agent, không cần đăng nhập bằng mật khẩu.

mod entity;
mod service;

pub(crate) use service::{
    ApiKey, ApiKeyService, CreatedApiKey, NewApiKey, Reauth, invalid_api_key, revoke_all_in,
    tenant_of,
};

/// Tính năng theo gói (bước 35): tạo API key — khách bị gói giới hạn cần tính năng này.
pub(crate) const API_KEYS_FEATURE: autoxide_kernel::entitlement::Feature =
    autoxide_kernel::entitlement::Feature::new(
        "auth.api_keys",
        "Create API keys, so machines and AI agents can call the API without signing in.",
    );
