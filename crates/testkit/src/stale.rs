//! Recognizes the "garbage" left behind by a test run that died.
//!
//! Test databases are named `<prefix><UUIDv7>`. A UUIDv7 contains its creation time,
//! so the name alone tells whether it is old or new — no ledger to keep anywhere.
//! Normally each database is dropped when its test finishes; only when the test process
//! is killed midway does one survive, and the next run cleans up those that are too old.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use uuid::Uuid;

/// Older than this counts as garbage. Long enough that it never touches the database
/// of another test run still in progress on the same machine.
pub(crate) const STALE_AFTER: Duration = Duration::from_secs(60 * 60);

/// Does `name` have the form `<prefix><UUIDv7>` and was it created more than [`STALE_AFTER`] before `now`?
/// A name of any other form is NOT testkit garbage — it must not be touched.
pub(crate) fn is_stale(name: &str, prefix: &str, now: SystemTime) -> bool {
    let Some(created_at) = name.strip_prefix(prefix).and_then(created_at) else {
        return false;
    };
    // `duration_since` trả `Err` khi `created_at` nằm sau `now` (đồng hồ lệch):
    // coi như mới.
    now.duration_since(created_at).is_ok_and(|age| age > STALE_AFTER)
}

/// Does `name` have the form `<prefix><UUIDv7>` (old or new): a testkit database.
pub(crate) fn has_shape(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix).and_then(created_at).is_some()
}

/// Is `name` the temporary directory of a test, of the form `autoxide-<task>-<UUIDv7>` (e.g.
/// `autoxide-catalog-…`, `autoxide-plans-…`), and older than [`STALE_AFTER`]?
///
/// Many tests create temporary directories under `std::env::temp_dir()` without deleting them; previously
/// only the testkit's `autoxide-test-…` directories were cleaned, so garbage piled up until the disk filled.
/// The UUIDv7 at the end of the name carries the creation time, so it is recognizable without a ledger. A
/// name of any other form is not touched.
pub(crate) fn is_stale_scratch(name: &str, now: SystemTime) -> bool {
    const UUID_LEN: usize = 36;
    let Some(rest) = name.strip_prefix("autoxide-") else {
        return false;
    };
    // Phải còn `<việc>-` trước UUID: không nhận `autoxide-<UUID>` trần.
    if rest.len() <= UUID_LEN + 1 || !rest.is_char_boundary(rest.len() - UUID_LEN) {
        return false;
    }
    let (label, uuid_text) = rest.split_at(rest.len() - UUID_LEN);
    let Some(label) = label.strip_suffix('-') else {
        return false;
    };
    if label.is_empty() || !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return false;
    }
    let Some(created_at) = created_at(uuid_text) else {
        return false;
    };
    now.duration_since(created_at).is_ok_and(|age| age > STALE_AFTER)
}

/// Creation time recorded in a UUIDv7 (written with or without hyphens).
fn created_at(uuid_text: &str) -> Option<SystemTime> {
    let uuid = Uuid::parse_str(uuid_text).ok()?;
    // `get_timestamp` trả `None` với UUID không mang thời gian (vd v4).
    let (seconds, nanos) = uuid.get_timestamp()?.to_unix();
    Some(UNIX_EPOCH + Duration::new(seconds, nanos))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use uuid::{NoContext, Timestamp};

    /// A UUIDv7 carrying the time `at`.
    pub(crate) fn uuid_created_at(at: SystemTime) -> Uuid {
        let since_epoch = at.duration_since(UNIX_EPOCH).unwrap();
        Uuid::new_v7(Timestamp::from_unix(
            NoContext,
            since_epoch.as_secs(),
            since_epoch.subsec_nanos(),
        ))
    }

    #[test]
    fn cu_hon_nguong_la_rac_moi_hon_thi_khong() {
        let now = SystemTime::now();
        let old = uuid_created_at(now - STALE_AFTER - Duration::from_secs(60));
        let fresh = uuid_created_at(now - STALE_AFTER + Duration::from_secs(60));

        assert!(is_stale(&format!("autoxide-test-{old}"), "autoxide-test-", now));
        assert!(is_stale(&format!("autoxide_test_{}", old.simple()), "autoxide_test_", now));
        assert!(!is_stale(&format!("autoxide-test-{fresh}"), "autoxide-test-", now));
        assert!(!is_stale(&format!("autoxide_test_{}", fresh.simple()), "autoxide_test_", now));
    }

    #[test]
    fn ten_khong_dung_dang_thi_khong_bao_gio_la_rac() {
        let now = SystemTime::now();
        let old = uuid_created_at(now - STALE_AFTER * 10);
        for name in [
            // Sai tiền tố.
            format!("other-{old}"),
            // Đúng tiền tố nhưng phần sau không phải UUID.
            "autoxide-test-".to_owned(),
            "autoxide-test-production".to_owned(),
            // UUID v4 không mang thời gian.
            "autoxide-test-936da01f-9abd-4d9d-80c7-02af85c822a8".to_owned(),
        ] {
            assert!(!is_stale(&name, "autoxide-test-", now), "{name}");
        }
    }

    #[test]
    fn thu_muc_tam_cua_test_cu_la_rac_moi_hay_sai_dang_thi_khong() {
        let now = SystemTime::now();
        let old = uuid_created_at(now - STALE_AFTER - Duration::from_secs(60));
        let fresh = uuid_created_at(now - STALE_AFTER + Duration::from_secs(60));

        assert!(is_stale_scratch(&format!("autoxide-catalog-{old}"), now));
        assert!(is_stale_scratch(&format!("autoxide-test-{old}"), now));
        assert!(is_stale_scratch(&format!("autoxide-missing-parent-directory-{old}"), now));
        assert!(!is_stale_scratch(&format!("autoxide-catalog-{fresh}"), now));
        for name in [
            format!("autoxide-{old}"),
            format!("other-catalog-{old}"),
            format!("autoxide-catalog{old}"),
            format!("autoxide-cat alog-{old}"),
            "autoxide-catalog-936da01f-9abd-4d9d-80c7-02af85c822a8".to_owned(),
            "autoxide-production".to_owned(),
        ] {
            assert!(!is_stale_scratch(&name, now), "{name}");
        }
    }

    #[test]
    fn dong_ho_lech_ve_tuong_lai_thi_coi_la_moi() {
        let now = SystemTime::now();
        let future = uuid_created_at(now + Duration::from_secs(24 * 60 * 60));
        assert!(!is_stale(&format!("autoxide-test-{future}"), "autoxide-test-", now));
    }
}
