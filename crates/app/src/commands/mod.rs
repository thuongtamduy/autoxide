//! Mỗi lệnh của CLI một file. Hàm `run` của lệnh trả mã thoát của tiến trình.

pub mod config;

use std::process::ExitCode;

use autoxide_kernel::config::ConfigErrors;

/// Mã thoát khi cấu hình sai: tách khỏi lỗi lúc chạy (1) để script phân biệt được.
pub const EXIT_INVALID_CONFIGURATION: u8 = 2;

/// In lỗi cấu hình ra stderr và trả mã thoát tương ứng. Mọi lệnh dùng chung hàm
/// này để thông báo giống nhau.
pub fn report_invalid_configuration(errors: &ConfigErrors) -> ExitCode {
    eprintln!("autoxide: {errors}");
    eprintln!();
    eprintln!("Settings come from environment variables and from `.env` in the current");
    eprintln!("directory. See `.env.example` for every setting and its default.");
    ExitCode::from(EXIT_INVALID_CONFIGURATION)
}
