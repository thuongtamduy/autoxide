//! Bước `database_version` của `autoxide doctor`: bản database đang chạy có nằm trong danh sách được hỗ trợ không
//! (bước 71 G14 "Cam kết nền tảng", D-20261128-platform-support; danh sách công khai ở docs/COMPATIBILITY.md, mục
//! "Supported platforms"). Ngoài danh sách thì chỉ CẢNH BÁO: máy chủ vẫn chạy, người vận hành biết mình đang ở ngoài
//! phần CI đã kiểm.

use autoxide_kernel::config::DatabaseBackend;
use autoxide_kernel::database::Database;
use autoxide_kernel::doctor::Finding;
use autoxide_kernel::time::Date;

/// Các bản lớn PostgreSQL được hỗ trợ và ngày hãng ngừng vá (EOL, theo
/// <https://www.postgresql.org/support/versioning/>). Sửa bảng này cùng `docs/COMPATIBILITY.md` (EN + VI) và ma trận
/// PostgreSQL của CI (`.github/workflows/gate.yml`): thêm bản lớn mới khi CI đã xanh trên nó; bỏ bản cũ theo chính sách
/// ở tài liệu đó (báo trước một bản minor).
const POSTGRES_MAJORS: &[(u32, (i16, i8, i8))] =
    &[(16, (2028, 11, 9)), (17, (2029, 11, 8)), (18, (2030, 11, 14))];

/// Bản SQLite thấp nhất được hỗ trợ. Binary và image chính thức nhúng sẵn SQLite (feature `bundled` của sqlx) nên chỉ
/// bản dựng lại với SQLite của hệ điều hành mới có thể thấp hơn.
const SQLITE_MINIMUM: (u32, u32, u32) = (3, 45, 0);

/// Hỏi bản của database rồi so với danh sách. Không hỏi được thì cảnh báo (bước `database` đã xác nhận kết nối).
pub(super) async fn probe(database: &Database) -> Finding {
    match (database.backend(), database.server_version().await) {
        (_, Err(error)) => {
            Finding::warning(format!("could not read the database version: {error}"))
        }
        (DatabaseBackend::Postgres, Ok(number)) => match number.trim().parse::<u32>() {
            Ok(number) => judge_postgres(number, Date::today_utc()),
            Err(_) => {
                Finding::warning(format!("unrecognised PostgreSQL version number {number:?}"))
            }
        },
        (DatabaseBackend::Sqlite, Ok(version)) => judge_sqlite(&version),
    }
}

fn compatibility_hint() -> &'static str {
    "See \"Supported platforms\" in docs/COMPATIBILITY.md for the versions the CI tests."
}

/// `server_version_num` là `<major>` × 10000 + `<minor>` từ PostgreSQL 10 (vd `160004` = 16.4).
fn judge_postgres(server_version_num: u32, today: Date) -> Finding {
    let major = server_version_num / 10_000;
    let minor = server_version_num % 10_000;
    let shown = format!("PostgreSQL {major}.{minor}");
    let supported: Vec<String> =
        POSTGRES_MAJORS.iter().map(|(major, _)| major.to_string()).collect();
    let supported = supported.join(", ");
    let Some(&(_, (year, month, day))) = POSTGRES_MAJORS.iter().find(|(known, _)| *known == major)
    else {
        let lowest = POSTGRES_MAJORS.first().map_or(0, |(major, _)| *major);
        let reason = if major < lowest { "older than" } else { "newer than" };
        return Finding::warning(format!(
            "{shown} is {reason} the supported PostgreSQL majors ({supported}); it is not tested by the CI."
        ))
        .with_hint(compatibility_hint());
    };
    match Date::new(year, month, day) {
        Some(end_of_life) if today >= end_of_life => Finding::warning(format!(
            "{shown}: the PostgreSQL project stopped fixing this major version on \
             {year:04}-{month:02}-{day:02}; plan an upgrade to a newer major ({supported})."
        ))
        .with_hint(compatibility_hint()),
        _ => Finding::ok(format!("{shown} is a supported version.")),
    }
}

fn judge_sqlite(version: &str) -> Finding {
    let mut parts = version.trim().split('.').map(|part| part.parse::<u32>().unwrap_or(0));
    let found = (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    let (major, minor, patch) = SQLITE_MINIMUM;
    if found >= SQLITE_MINIMUM {
        Finding::ok(format!("SQLite {version} is a supported version."))
    } else {
        Finding::warning(format!(
            "SQLite {version} is older than the lowest supported version {major}.{minor}.{patch}."
        ))
        .with_hint(
            "The official binary and image bundle a recent SQLite; a build linked to the system SQLite needs a \
             newer one. See \"Supported platforms\" in docs/COMPATIBILITY.md.",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::doctor::Status;

    fn day(year: i16, month: i8, day: i8) -> Date {
        Date::new(year, month, day).unwrap()
    }

    #[test]
    fn postgres_trong_danh_sach_va_con_duoc_va() {
        let finding = judge_postgres(160_004, day(2026, 11, 28));
        assert_eq!(finding.status, Status::Ok, "{finding:?}");
        assert!(finding.message.contains("PostgreSQL 16.4"), "{}", finding.message);
        assert_eq!(judge_postgres(180_001, day(2026, 11, 28)).status, Status::Ok);
    }

    #[test]
    fn postgres_cu_moi_hay_het_han_thi_canh_bao() {
        let old = judge_postgres(150_008, day(2026, 11, 28));
        assert_eq!(old.status, Status::Warning);
        assert!(old.message.contains("older than"), "{}", old.message);
        let new = judge_postgres(190_000, day(2026, 11, 28));
        assert_eq!(new.status, Status::Warning);
        assert!(new.message.contains("newer than"), "{}", new.message);
        let ended = judge_postgres(160_010, day(2028, 11, 9));
        assert_eq!(ended.status, Status::Warning);
        assert!(ended.message.contains("2028-11-09"), "{}", ended.message);
        assert!(ended.hint.is_some());
    }

    #[test]
    fn sqlite_so_theo_tung_phan_cua_ban() {
        assert_eq!(judge_sqlite("3.45.0").status, Status::Ok);
        assert_eq!(judge_sqlite("3.50.4").status, Status::Ok);
        // So số chứ không so chuỗi: "3.9" < "3.45".
        assert_eq!(judge_sqlite("3.9.2").status, Status::Warning);
        assert_eq!(judge_sqlite("3.44.2").status, Status::Warning);
    }
}
