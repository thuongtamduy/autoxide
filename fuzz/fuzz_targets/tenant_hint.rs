//! The `Host` and `Autoxide-Tenant` headers pick the tenant before any authentication, so they are fully untrusted:
//! any header bytes must produce a hint (possibly empty), never a panic.
#![no_main]

use autoxide_kernel::tenancy::TenantHint;
use http::{HeaderMap, HeaderValue, header};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: (&[u8], &[u8])| {
    let (host, tenant) = input;
    let mut headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_bytes(host) {
        headers.insert(header::HOST, value);
    }
    if let Ok(value) = HeaderValue::from_bytes(tenant) {
        headers.insert(autoxide_kernel::tenancy::TENANT_HEADER, value);
    }
    let hint = TenantHint::from_headers(&headers);
    let _ = (hint.host(), hint.tenant());
});
