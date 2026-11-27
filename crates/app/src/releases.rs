//! Bảng "migration nào có từ bản phát hành nào" nhúng trong binary (`crates/app/migration-releases.json`,
//! D-20261126-upgrade-rollback): `autoxide rollback --to X.Y.Z` giữ đúng các migration có trong bản X.Y.Z và gỡ phần
//! còn lại. Dựa vào TÊN migration (không đổi khi viết lại migration, D-20261126-migration-rewrite-proof).
//!
//! File ghi mỗi migration một lần, kèm bản phát hành đầu tiên có nó: migration đã phát hành không bao giờ bị bỏ, nên
//! tập migration của bản X = mọi migration có bản đầu tiên ≤ X. Migration chưa có trong file là của bản đang phát
//! triển (chưa phát hành). `cargo xtask migration-releases record <X.Y.Z>` ghi thêm lúc phát hành.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use serde::Deserialize;

/// Nội dung file, nhúng lúc biên dịch.
const EMBEDDED: &str = include_str!("../migration-releases.json");

/// Bảng đã đọc.
#[derive(Debug, Deserialize)]
pub(crate) struct MigrationReleases {
    /// Các bản phát hành có trong bảng, tăng dần.
    releases: Vec<String>,
    /// `<module>/<tên>` → bản phát hành đầu tiên có nó.
    introduced: BTreeMap<String, String>,
}

impl MigrationReleases {
    pub(crate) fn embedded() -> Result<Self, String> {
        serde_json::from_str(EMBEDDED)
            .map_err(|error| format!("crates/app/migration-releases.json: {error}"))
    }

    /// Bảng từ một chuỗi JSON (test).
    #[cfg(test)]
    pub(crate) fn parse(text: &str) -> Self {
        serde_json::from_str(text)
            .unwrap_or(MigrationReleases { releases: Vec::new(), introduced: BTreeMap::new() })
    }

    /// `version` là một bản phát hành có trong bảng.
    pub(crate) fn knows(&self, version: &str) -> bool {
        self.releases.iter().any(|release| release == version)
    }

    pub(crate) fn releases(&self) -> &[String] {
        &self.releases
    }

    /// Migration `module/name` có trong bản `target` (đã phát hành ở `target` hay trước đó).
    pub(crate) fn in_release(&self, target: &str, module: &str, name: &str) -> bool {
        self.introduced
            .get(&format!("{module}/{name}"))
            .is_some_and(|first| compare(first, target) != Ordering::Greater)
    }
}

/// So hai phiên bản `X.Y.Z` hay `X.Y.Z-pre` theo SemVer (bản `-pre` đứng trước bản chính cùng số; hai bản `-pre`
/// so theo chuỗi). Không so chuỗi trực tiếp: "0.9.0" > "0.10.0" khi so chuỗi.
pub(crate) fn compare(left: &str, right: &str) -> Ordering {
    let parse = |version: &str| {
        let (core, pre) = match version.split_once('-') {
            Some((core, pre)) => (core, Some(pre.to_owned())),
            None => (version, None),
        };
        let numbers: Vec<u64> = core.split('.').map(|part| part.parse().unwrap_or(0)).collect();
        (numbers, pre)
    };
    let (left_numbers, left_pre) = parse(left);
    let (right_numbers, right_pre) = parse(right);
    left_numbers.cmp(&right_numbers).then_with(|| match (left_pre, right_pre) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(left), Some(right)) => left.cmp(&right),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn so_phien_ban_theo_semver() {
        assert_eq!(compare("0.9.0", "0.10.0"), Ordering::Less);
        assert_eq!(compare("1.0.0-rc.1", "1.0.0"), Ordering::Less);
        assert_eq!(compare("0.19.0", "0.19.0"), Ordering::Equal);
    }

    #[test]
    fn bang_nhung_doc_duoc_va_tang_dan() {
        let table = MigrationReleases::embedded().unwrap();
        assert!(
            table.releases().windows(2).all(|pair| compare(&pair[0], &pair[1]) == Ordering::Less)
        );
        for first in table.introduced.values() {
            assert!(table.knows(first), "{first} is not in `releases`");
        }
    }

    #[test]
    fn giu_dung_migration_cua_ban_dich() {
        let table: MigrationReleases = serde_json::from_str(
            r#"{"releases": ["0.9.0", "0.10.0"],
                "introduced": {"shop/m0001": "0.9.0", "shop/m0002": "0.10.0"}}"#,
        )
        .unwrap();
        assert!(table.in_release("0.9.0", "shop", "m0001"));
        assert!(!table.in_release("0.9.0", "shop", "m0002"));
        assert!(table.in_release("0.10.0", "shop", "m0002"));
        assert!(!table.in_release("0.10.0", "shop", "m0003"), "chưa phát hành");
    }
}
