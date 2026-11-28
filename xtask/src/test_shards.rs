//! `cargo xtask test-shards [--tests <file> --filter <file>] [--postgres-18]`: in matrix (JSON) các máy chạy test của
//! CI — mỗi phần tử một shard `{"name", "part", "shard", "postgres"}` cho job `tests` của `.github/workflows/gate.yml`
//! (D-20261127-ci-runner-budget).
//!
//! - Không có `--tests`: gate đầy đủ (push lên main, tag, chạy tay) — cách chia cố định như trước: SQLite 4 phần,
//!   Postgres 6 phần (D-20261121-ci-mold-shards).
//! - `--tests <file> --filter <file>`: pull request — `cargo nextest list --message-format json` với filterset của PR và
//!   chính filterset đó (`./gate.sh --pr --only build` ghi `tests.json` và `filter` cạnh archive). Số phần của mỗi
//!   database tính theo chi phí ước lượng của các test được chọn, tối đa bằng gate đầy đủ: PR chỉ chạm vài crate không
//!   chiếm 10 máy cho vài trăm test. Matrix rỗng (`[]`, workflow bỏ qua job `tests`) chỉ khi filterset là `none()`;
//!   JSON lạ hay không test nào khớp một filterset khác là lỗi (fail-closed).
//!
//! Chỉ đổi SỐ máy, không đổi test nào chạy: `--partition count:k/n` của nextest chia đúng tập test đã chọn thành n phần
//! rời nhau, cộng lại đủ.

use serde::Deserialize;
use std::collections::BTreeMap;

/// Crate có test tích hợp chậm hơn hẳn: mỗi test dựng cả ứng dụng với mọi module và chạy mọi migration.
const APP_PACKAGE: &str = "autoxide";

/// Chi phí ước lượng (giây test, cộng mọi luồng) của một test và ngân sách của một shard, cho một database.
struct Cost {
    app_seconds: f64,
    other_seconds: f64,
    shard_budget: f64,
    max_shards: usize,
}

// Đo trên CI ngày 2026-11-27 (lần chạy 46041097124 trên main, 46014429906 trên PR): trung bình một test của `autoxide`
// 11,5 giây trên SQLite và 37 giây trên Postgres; test của crate khác 0,6 và 3,4 giây. Ngân sách của một shard là tổng
// của gate đầy đủ chia số shard hiện có (~300 test `autoxide` + ~2800 test khác → SQLite ~5200 giây / 4, Postgres
// ~20700 giây / 6), nên shard của PR không dài hơn shard của gate đầy đủ. Số đo cũ đi chỉ làm số máy lệch một chút,
// không đổi test nào chạy.
const SQLITE: Cost =
    Cost { app_seconds: 11.5, other_seconds: 0.6, shard_budget: 1300.0, max_shards: 4 };
const POSTGRES: Cost =
    Cost { app_seconds: 37.0, other_seconds: 3.4, shard_budget: 3450.0, max_shards: 6 };

#[derive(Deserialize)]
struct List {
    #[serde(rename = "rust-suites")]
    suites: BTreeMap<String, Suite>,
}

#[derive(Deserialize)]
struct Suite {
    #[serde(rename = "package-name")]
    package: String,
    status: SuiteStatus,
    // Không `#[serde(default)]`: chương trình đã liệt kê (`listed`) mà thiếu danh sách test là JSON lạ — báo lỗi, không
    // coi là "không có test" (fail-closed: đếm sai thành 0 thì gate của PR xanh mà không chạy test nào).
    testcases: Option<BTreeMap<String, Case>>,
}

/// Giá trị lạ (nextest đổi định dạng) là lỗi đọc JSON, không bị bỏ qua.
#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum SuiteStatus {
    Listed,
    Skipped,
}

#[derive(Deserialize)]
struct Case {
    #[serde(default)]
    ignored: bool,
    #[serde(rename = "filter-match")]
    filter_match: FilterMatch,
}

#[derive(Deserialize)]
struct FilterMatch {
    status: MatchStatus,
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum MatchStatus {
    Matches,
    Mismatch,
}

/// Số test được chọn: của `autoxide` và của mọi crate khác.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Selected {
    pub app: usize,
    pub other: usize,
}

/// Đếm test khớp filterset (không tính test `#[ignore]`: nextest không chạy chúng) trong JSON của `cargo nextest list`.
///
/// `filter`: filterset đã dùng để liệt kê (file `filter` cạnh archive). Fail-closed: JSON không có test nào, hay không
/// test nào khớp mà filterset không phải `none()`, là lỗi — matrix rỗng chỉ khi PR thật sự không chọn crate nào.
pub fn count(list_json: &str, filter: &str) -> Result<Selected, String> {
    let list: List = serde_json::from_str(list_json).map_err(|error| {
        format!("not the JSON of `cargo nextest list --message-format json`: {error}")
    })?;
    let mut selected = Selected { app: 0, other: 0 };
    let mut listed = 0;
    for (id, suite) in &list.suites {
        let cases = match (&suite.status, &suite.testcases) {
            (SuiteStatus::Listed, None) => {
                return Err(format!("test binary {id} is listed but has no `testcases`"));
            }
            (_, None) => continue,
            (_, Some(cases)) => cases,
        };
        listed += cases.len();
        let matching = cases
            .values()
            .filter(|case| !case.ignored && case.filter_match.status == MatchStatus::Matches)
            .count();
        if suite.package == APP_PACKAGE {
            selected.app += matching;
        } else {
            selected.other += matching;
        }
    }
    if listed == 0 {
        return Err(
            "the nextest list has no tests at all: refusing to plan zero test machines".to_owned()
        );
    }
    if selected.app + selected.other == 0 && filter.trim() != "none()" {
        return Err(format!(
            "no listed test matches the filterset `{}`, which is not `none()`: refusing to plan zero test machines",
            filter.trim()
        ));
    }
    Ok(selected)
}

/// Số shard cho một database: 0 khi không có test, còn lại từ 1 tới `max_shards`.
fn shards(selected: Selected, cost: &Cost) -> usize {
    if selected.app + selected.other == 0 {
        return 0;
    }
    // Ép kiểu số nguyên nhỏ (số test) sang f64 là chính xác; kết quả làm tròn lên rồi kẹp trong [1, max].
    let seconds =
        selected.app as f64 * cost.app_seconds + selected.other as f64 * cost.other_seconds;
    let needed = (seconds / cost.shard_budget).ceil();
    if needed >= cost.max_shards as f64 { cost.max_shards } else { (needed as usize).max(1) }
}

/// Matrix cho job `tests`. `selected`: `None` là gate đầy đủ (chia cố định).
pub fn matrix(selected: Option<Selected>, postgres_18: bool) -> String {
    let (sqlite, postgres) = match selected {
        None => (SQLITE.max_shards, POSTGRES.max_shards),
        Some(selected) => (shards(selected, &SQLITE), shards(selected, &POSTGRES)),
    };
    let mut entries = Vec::new();
    for k in 1..=sqlite {
        entries.push(entry(&format!("sqlite {k}/{sqlite}"), "sqlite", k, sqlite, "16"));
    }
    let versions: &[&str] = if postgres_18 { &["16", "18"] } else { &["16"] };
    for version in versions {
        for k in 1..=postgres {
            let name = format!("postgres {version} {k}/{postgres}");
            entries.push(entry(&name, "postgres", k, postgres, version));
        }
    }
    serde_json::Value::Array(entries).to_string()
}

fn entry(name: &str, part: &str, k: usize, n: usize, postgres: &str) -> serde_json::Value {
    serde_json::json!({ "name": name, "part": part, "shard": format!("{k}/{n}"), "postgres": postgres })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(matrix: &str) -> Vec<String> {
        let value: serde_json::Value = serde_json::from_str(matrix).unwrap();
        value.as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap().to_owned()).collect()
    }

    #[test]
    fn gate_day_du_chia_co_dinh_nhu_truoc() {
        let full = names(&matrix(None, false));
        assert_eq!(full.len(), 10);
        assert_eq!(full[0], "sqlite 1/4");
        assert_eq!(full[9], "postgres 16 6/6");
        let with_18 = names(&matrix(None, true));
        assert_eq!(with_18.len(), 16);
        assert_eq!(with_18[15], "postgres 18 6/6");
    }

    #[test]
    fn moi_phan_tu_co_du_truong_cho_workflow() {
        let value: serde_json::Value = serde_json::from_str(&matrix(None, false)).unwrap();
        assert_eq!(
            value[4],
            serde_json::json!({"name": "postgres 16 1/6", "part": "postgres", "shard": "1/6", "postgres": "16"})
        );
    }

    #[test]
    fn pr_cham_moi_crate_van_chia_toi_da() {
        let all = Selected { app: 300, other: 2800 };
        assert_eq!(shards(all, &SQLITE), 4);
        assert_eq!(shards(all, &POSTGRES), 6);
    }

    #[test]
    fn pr_cham_mot_module_dung_it_may_hon() {
        // Module + crate phụ thuộc + `autoxide` (luôn phụ thuộc mọi module).
        let leaf = Selected { app: 300, other: 100 };
        assert_eq!(shards(leaf, &SQLITE), 3);
        assert_eq!(shards(leaf, &POSTGRES), 4);
        assert_eq!(names(&matrix(Some(leaf), false)).len(), 7);
    }

    #[test]
    fn pr_chi_cham_cong_cu_mot_may_moi_database() {
        let tool = Selected { app: 0, other: 120 };
        assert_eq!(names(&matrix(Some(tool), false)), ["sqlite 1/1", "postgres 16 1/1"]);
    }

    #[test]
    fn khong_test_nao_duoc_chon_thi_matrix_rong() {
        assert_eq!(matrix(Some(Selected { app: 0, other: 0 }), false), "[]");
    }

    const LIST: &str = r#"{
      "rust-build-meta": {}, "test-count": 4,
      "rust-suites": {
        "autoxide::integration": {"package-name": "autoxide", "status": "listed", "testcases": {
          "a": {"kind": "test", "ignored": false, "filter-match": {"status": "matches"}},
          "b": {"kind": "test", "ignored": true, "filter-match": {"status": "matches"}}
        }},
        "autoxide-kernel": {"package-name": "autoxide-kernel", "status": "listed", "testcases": {
          "c": {"kind": "test", "ignored": false, "filter-match": {"status": "matches"}},
          "d": {"kind": "test", "ignored": false, "filter-match": {"status": "mismatch", "reason": "expression"}}
        }},
        "autoxide-notes": {"package-name": "autoxide-notes", "status": "skipped"}
      }
    }"#;

    #[test]
    fn dem_dung_test_khop_bo_qua_test_ignore() {
        assert_eq!(count(LIST, "all()").unwrap(), Selected { app: 1, other: 1 });
        assert!(count("not json", "all()").is_err());
    }

    #[test]
    fn json_la_thi_bao_loi_khong_dem_thanh_0() {
        // Trạng thái khớp lạ (nextest đổi định dạng).
        let odd = LIST.replace(r#""status": "matches""#, r#""status": "selected""#);
        assert!(count(&odd, "all()").is_err());
        // Trạng thái chương trình lạ.
        let odd = LIST.replace(r#""status": "skipped""#, r#""status": "omitted""#);
        assert!(count(&odd, "all()").is_err());
        // Chương trình đã liệt kê mà không có danh sách test.
        let missing = r#"{"rust-suites": {"x": {"package-name": "x", "status": "listed"}}}"#;
        assert!(count(missing, "all()").is_err());
        // Không có test nào cả.
        assert!(count(r#"{"rust-suites": {}}"#, "none()").is_err());
    }

    #[test]
    fn khong_test_nao_khop_chi_hop_le_khi_filter_la_none() {
        let none = LIST
            .replace(r#""status": "matches""#, r#""status": "mismatch", "reason": "expression""#);
        assert_eq!(count(&none, "none()\n").unwrap(), Selected { app: 0, other: 0 });
        assert!(count(&none, "package(=autoxide-kernel)").is_err());
    }
}
