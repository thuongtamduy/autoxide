//! Mỗi lệnh của CLI một file. Hàm `run` của lệnh trả mã thoát của tiến trình.

pub mod config;
pub mod migrate;

use std::future::Future;
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

/// Mã thoát khi lệnh hỏng lúc chạy (không kết nối được database, migration lỗi…).
pub const EXIT_FAILURE: u8 = 1;

/// In lỗi lúc chạy ra stderr và trả mã thoát tương ứng.
pub fn report_failure(error: &str) -> ExitCode {
    eprintln!("autoxide: {error}");
    ExitCode::from(EXIT_FAILURE)
}

/// Chạy một việc `async` tới khi xong, từ hàm thường.
///
/// `main` của chương trình là hàm thường; các lệnh chạm database là `async`. Hàm
/// này dựng runtime tokio (bộ máy chạy code async) rồi chờ `task` chạy xong.
pub fn block_on<T>(task: impl Future<Output = Result<T, String>>) -> Result<T, String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("cannot start the async runtime: {error}"))?;
    runtime.block_on(task)
}
