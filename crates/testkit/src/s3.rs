//! S3 cho test: test của nơi lưu file chạy thêm trên một dịch vụ S3 thật (hay giả lập
//! chuẩn, như `moto`) khi biến `TEST_S3_ENDPOINT` được đặt. Gate tự dựng một máy chủ S3
//! giả bằng Docker và đặt biến này.

/// Địa chỉ của dịch vụ tương thích S3 cho test, vd `http://127.0.0.1:5000`.
pub const TEST_S3_ENDPOINT: &str = "TEST_S3_ENDPOINT";
/// Bucket dùng cho test (phải có sẵn). Mặc định `autoxide-test`.
pub const TEST_S3_BUCKET: &str = "TEST_S3_BUCKET";
/// Thông tin đăng nhập (dịch vụ giả nhận giá trị bất kỳ). Mặc định `test` / `test`.
pub const TEST_S3_ACCESS_KEY_ID: &str = "TEST_S3_ACCESS_KEY_ID";
pub const TEST_S3_SECRET_ACCESS_KEY: &str = "TEST_S3_SECRET_ACCESS_KEY";

/// Cấu hình S3 cho test dưới dạng cặp biến cấu hình của Autoxide, mỗi lần gọi một thư
/// mục riêng trong bucket (test chạy song song không giẫm file của nhau). `None`: chưa
/// đặt `TEST_S3_ENDPOINT`.
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
