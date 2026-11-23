//! S3 for tests: tests of file storage additionally run against a real S3 service (or a standard
//! emulator such as `moto`) when the `TEST_S3_ENDPOINT` variable is set. The gate starts a fake S3 server
//! with Docker and sets this variable.

/// Address of the S3-compatible service for tests, e.g. `http://127.0.0.1:5000`.
pub const TEST_S3_ENDPOINT: &str = "TEST_S3_ENDPOINT";
/// Bucket used for tests (must already exist). Defaults to `autoxide-test`.
pub const TEST_S3_BUCKET: &str = "TEST_S3_BUCKET";
/// Credentials (the fake service accepts any value). Defaults to `test` / `test`.
pub const TEST_S3_ACCESS_KEY_ID: &str = "TEST_S3_ACCESS_KEY_ID";
pub const TEST_S3_SECRET_ACCESS_KEY: &str = "TEST_S3_SECRET_ACCESS_KEY";

/// S3 configuration for tests as Autoxide configuration variable pairs, with a separate directory in the
/// bucket on each call (tests running in parallel do not clobber each other's files). `None`:
/// `TEST_S3_ENDPOINT` is not set.
pub fn s3_settings() -> Option<Vec<(&'static str, String)>> {
    let endpoint = std::env::var(TEST_S3_ENDPOINT).ok().filter(|value| !value.is_empty())?;
    let read = |name: &str, default: &str| std::env::var(name).unwrap_or_else(|_| default.into());
    Some(vec![
        ("AUTOXIDE_FILES_S3_BUCKET", read(TEST_S3_BUCKET, "autoxide-test")),
        ("AUTOXIDE_FILES_S3_PREFIX", format!("test-{}", uuid::Uuid::now_v7())),
        ("AWS_ENDPOINT_URL", endpoint),
        ("AWS_ACCESS_KEY_ID", read(TEST_S3_ACCESS_KEY_ID, "test")),
        ("AWS_SECRET_ACCESS_KEY", read(TEST_S3_SECRET_ACCESS_KEY, "test")),
    ])
}
