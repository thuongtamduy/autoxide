//! Lệnh `autoxide initial-password`: in (hay xoá) file mật khẩu của quản trị viên đầu tiên mà `autoxide serve` đã ghi
//! (`AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD_OUT`, mặc định `data/initial-admin-password`).
//!
//! Image Docker không có shell nên không có `cat` hay `rm`: hướng dẫn quickstart dùng lệnh này (`docker compose … exec autoxide
//! autoxide initial-password`), và nó chạy được cả ngoài Docker. Mật khẩu chỉ ra stdout khi người dùng tự gọi lệnh; không
//! bao giờ vào log. Đường dẫn lấy từ đúng cài đặt mà `serve` dùng để ghi.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use super::{EXIT_FAILURE, report_invalid_configuration};
use crate::settings;
use crate::startup::BOOTSTRAP_ADMIN_PASSWORD_OUT;

#[derive(Debug, clap::Args)]
pub struct Arguments {
    /// Delete the file instead of printing it. Do this once you have signed in and changed the password.
    #[arg(long)]
    remove: bool,
}

pub fn run(arguments: &Arguments) -> ExitCode {
    // Chỉ đọc đúng `AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD_OUT`, cùng khai báo `Setting` mà `serve` dùng để ghi file: lệnh chạy được cả
    // khi cấu hình khác thiếu, và không đi qua cấu trúc cài đặt có chứa secret.
    let path = match settings::source().and_then(|source| {
        settings::read_one(&source, |reader| reader.get::<PathBuf>(&BOOTSTRAP_ADMIN_PASSWORD_OUT))
    }) {
        Ok(path) => path,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    let path = path.as_path();
    if arguments.remove {
        match remove(path) {
            Ok(()) => {
                eprintln!("autoxide: removed {}", path.display());
                ExitCode::SUCCESS
            }
            Err(message) => failure(&message),
        }
    } else {
        match show(path) {
            Ok(text) => {
                let mut stdout = std::io::stdout().lock();
                match stdout.write_all(text.as_bytes()).and_then(|()| stdout.flush()) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(error) => failure(&format!("cannot write to the output: {error}")),
                }
            }
            Err(message) => failure(&message),
        }
    }
}

fn failure(message: &str) -> ExitCode {
    eprintln!("autoxide: {message}");
    ExitCode::from(EXIT_FAILURE)
}

/// Nội dung file (email, rồi mật khẩu), hay lời giải thích vì sao không có.
fn show(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|error| explain(path, &error))
}

/// Xoá file.
fn remove(path: &Path) -> Result<(), String> {
    std::fs::remove_file(path).map_err(|error| explain(path, &error))
}

fn explain(path: &Path, error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        format!(
            "{} does not exist: an administrator existed before the server first started, you set \
             AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD yourself, or the file was already removed",
            path.display()
        )
    } else {
        format!("cannot use {}: {error}", path.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str, content: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("autoxide-initial-password-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("initial-admin-password");
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn in_noi_dung_file_va_xoa_duoc() {
        let path = temp_file("show", "admin@example.com\ns3cret-generated\n");
        assert_eq!(show(&path).unwrap(), "admin@example.com\ns3cret-generated\n");
        remove(&path).unwrap();
        assert!(!path.exists());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn thieu_file_thi_giai_thich_va_khong_loi_mat_khau() {
        let path = temp_file("missing", "x");
        remove(&path).unwrap();
        let shown = show(&path).unwrap_err();
        let removed = remove(&path).unwrap_err();
        for message in [&shown, &removed] {
            assert!(message.contains("does not exist"), "{message}");
            assert!(message.contains("AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD"), "{message}");
        }
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
