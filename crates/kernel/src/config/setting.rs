//! Mô tả một biến cấu hình.

/// Một biến cấu hình, khai bằng `const` ở cạnh code dùng nó.
///
/// Khai MỘT lần, dùng cho bốn việc: đọc giá trị, báo lỗi, in ra ở lệnh
/// `autoxide config`, và sinh file `.env.example`. Nhờ vậy tài liệu cấu hình
/// không thể lệch với code.
///
/// ```
/// use autoxide_kernel::config::Setting;
///
/// const HTTP_BIND: Setting =
///     Setting::new("AUTOXIDE_HTTP_BIND", "Address the HTTP server listens on.")
///         .with_default("127.0.0.1:8080");
/// assert_eq!(HTTP_BIND.default, Some("127.0.0.1:8080"));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Setting {
    /// Tên biến môi trường. Tên chuẩn ngành nếu có (`DATABASE_URL`), còn lại
    /// mang tiền tố `AUTOXIDE_`.
    pub name: &'static str,
    /// Mô tả cho người cài đặt, tiếng Anh, một hai câu.
    pub description: &'static str,
    /// Giá trị dùng khi biến không được đặt. `None` nghĩa là bắt buộc phải đặt
    /// (hoặc là biến tuỳ chọn — xem [`ConfigReader::optional`](super::ConfigReader::optional)).
    pub default: Option<&'static str>,
    /// Giá trị là secret: không bao giờ in ra trong thông báo lỗi hay ở lệnh
    /// `autoxide config`.
    pub secret: bool,
    /// Giá trị minh hoạ cho `.env.example` khi không có giá trị mặc định.
    pub example: Option<&'static str>,
}

impl Setting {
    // `const fn`: gọi được lúc biên dịch, nên `Setting` khai được bằng `const`.
    pub const fn new(name: &'static str, description: &'static str) -> Self {
        Setting { name, description, default: None, secret: false, example: None }
    }

    pub const fn with_default(mut self, default: &'static str) -> Self {
        self.default = Some(default);
        self
    }

    pub const fn secret(mut self) -> Self {
        self.secret = true;
        self
    }

    pub const fn with_example(mut self, example: &'static str) -> Self {
        self.example = Some(example);
        self
    }
}
