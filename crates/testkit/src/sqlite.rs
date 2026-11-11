//! Phần SQLite của [`TestDatabase`](crate::TestDatabase): mỗi database là một
//! file trong một thư mục tạm riêng.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use uuid::Uuid;

use crate::stale::is_stale;

/// Tiền tố tên thư mục tạm của mỗi database test.
pub(crate) const DIRECTORY_PREFIX: &str = "autoxide-test-";

/// Tạo thư mục tạm mới dưới `parent`; trả về (thư mục, URL kết nối).
///
/// Dùng file thật chứ không dùng `sqlite::memory:` vì database trong bộ nhớ là
/// riêng của từng kết nối: pool nhiều kết nối, hay hai pool đọc / ghi tách nhau,
/// sẽ mỗi bên thấy một database khác nhau.
pub(crate) fn create_directory(parent: &Path) -> std::io::Result<(PathBuf, String)> {
    let directory = parent.join(format!("{DIRECTORY_PREFIX}{}", Uuid::now_v7()));
    std::fs::create_dir_all(&directory)?;
    let file = directory.join("test.sqlite");
    // `mode=rwc`: mở đọc-ghi và tạo file nếu chưa có.
    let url = format!("sqlite://{}?mode=rwc", file.display());
    Ok((directory, url))
}

/// Xoá các thư mục test đã quá cũ dưới `parent`. Trả về số thư mục đã xoá.
/// Lỗi (không đọc được thư mục, không xoá được) thì bỏ qua: đây chỉ là dọn rác.
pub(crate) fn remove_stale_directories(parent: &Path, now: SystemTime) -> usize {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return 0;
    };
    let mut removed = 0;
    // `flatten` bỏ qua các mục đọc lỗi.
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_stale(&name, DIRECTORY_PREFIX, now) && std::fs::remove_dir_all(entry.path()).is_ok() {
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
        let missing = std::env::temp_dir().join("autoxide-missing-parent-directory");
        assert_eq!(remove_stale_directories(&missing, SystemTime::now()), 0);
    }
}
