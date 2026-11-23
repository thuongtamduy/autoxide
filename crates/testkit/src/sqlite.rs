//! SQLite part of [`TestDatabase`](crate::TestDatabase): each database is a
//! file in its own temporary directory.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use uuid::Uuid;

use crate::stale::{is_stale, is_stale_scratch};

/// Name prefix of the temporary directory of each test database.
pub(crate) const DIRECTORY_PREFIX: &str = "autoxide-test-";

/// Creates a new temporary directory under `parent`; returns (directory, connection URL).
///
/// Uses a real file rather than `sqlite::memory:` because an in-memory database
/// is private to each connection: a pool with several connections, or separate read / write pools,
/// would each see a different database.
pub(crate) fn create_directory(parent: &Path) -> std::io::Result<(PathBuf, String)> {
    let directory = parent.join(format!("{DIRECTORY_PREFIX}{}", Uuid::now_v7()));
    std::fs::create_dir_all(&directory)?;
    let file = directory.join("test.sqlite");
    // `mode=rwc`: mở đọc-ghi và tạo file nếu chưa có.
    let url = format!("sqlite://{}?mode=rwc", file.display());
    Ok((directory, url))
}

/// Deletes test directories that are too old under `parent` — the testkit's own (`autoxide-test-…`) and
/// temporary directories created by tests themselves (`autoxide-<task>-<UUIDv7>`). Returns the number of
/// directories deleted. Errors (directory unreadable, deletion failed) are ignored: this is only garbage
/// cleanup.
pub(crate) fn remove_stale_directories(parent: &Path, now: SystemTime) -> usize {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return 0;
    };
    let mut removed = 0;
    // `flatten` bỏ qua các mục đọc lỗi.
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // Chỉ thư mục (không phải file): `remove_dir_all` trên file sẽ lỗi và bị bỏ qua.
        let stale = is_stale(&name, DIRECTORY_PREFIX, now) || is_stale_scratch(&name, now);
        if stale && entry.path().is_dir() && std::fs::remove_dir_all(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stale::STALE_AFTER;
    use crate::stale::tests::uuid_created_at;
    use std::time::Duration;

    #[test]
    fn thu_muc_cu_bi_xoa_thu_muc_moi_va_thu_muc_la_duoc_giu() {
        // Thư mục cha riêng cho bài test này để không đụng test khác.
        let (parent, _) = create_directory(&std::env::temp_dir()).unwrap();
        let now = SystemTime::now();

        let stale = parent.join(format!(
            "{DIRECTORY_PREFIX}{}",
            uuid_created_at(now - STALE_AFTER - Duration::from_secs(1))
        ));
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(stale.join("test.sqlite"), b"x").unwrap();
        let (fresh, _) = create_directory(&parent).unwrap();
        let unrelated = parent.join("someone-elses-directory");
        std::fs::create_dir_all(&unrelated).unwrap();

        assert_eq!(remove_stale_directories(&parent, now), 1);

        assert!(!stale.exists());
        assert!(fresh.exists());
        assert!(unrelated.exists());
        std::fs::remove_dir_all(&parent).unwrap();
    }

    #[test]
    fn thu_muc_cha_khong_ton_tai_thi_khong_loi() {
        // Tên riêng cho mỗi lần chạy: tên cố định thì lần chạy khác (hay ai đó) có thể đã tạo nó.
        let missing = std::env::temp_dir()
            .join(format!("autoxide-missing-parent-directory-{}", Uuid::now_v7()));
        assert_eq!(remove_stale_directories(&missing, SystemTime::now()), 0);
    }
}
