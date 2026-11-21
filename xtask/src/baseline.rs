//! Danh sách vi phạm có sẵn của các luật bảo mật `tenant-scope` và `mcp-principal`
//! (D-20261121-security-lints): luật vào gate khi code còn chỗ chưa sửa, nên chỗ cũ được ghi vào
//! [`FILE`] — giống `file-length-baseline.txt` — để gate chặn chỗ **mới** mà không đỏ vì chỗ cũ.
//!
//! Mỗi dòng: `<luật> <số chỗ tối đa> <đường dẫn>`, kèm `# dòng …` để người sửa tìm. Ghi theo số
//! chỗ trong file chứ không theo số dòng: phiên khác sửa file làm dòng xê dịch thì danh sách không
//! hỏng. File có nhiều chỗ hơn số ghi → mọi chỗ của file đó bị báo. Ít hơn (đã sửa bớt) → chỉ cảnh
//! báo nhắc hạ số / xoá dòng, không làm gate đỏ — để phiên đang vá ở module không phải sửa file này.
//! Không thêm dòng mới cho code mới: sửa code, hay ghi chú thích miễn trừ có lý do.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::boundaries::Violation;

pub const FILE: &str = "xtask/security-baseline.txt";

/// Vi phạm còn lại sau khi trừ danh sách, và cảnh báo (chỉ in).
pub struct Filtered {
    pub violations: Vec<Violation>,
    pub warnings: Vec<String>,
}

pub fn apply(root: &Path, violations: Vec<Violation>) -> Result<Filtered, String> {
    let text = fs::read_to_string(root.join(FILE)).unwrap_or_default();
    Ok(filter(&parse(&text)?, violations))
}

type Key = (String, String);

fn parse(text: &str) -> Result<BTreeMap<Key, usize>, String> {
    let mut entries = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [rule, count, path] = fields.as_slice() else {
            return Err(format!(
                "{FILE}:{}: expected `<rule> <count> <path>`, got `{line}`",
                index + 1
            ));
        };
        let count = count
            .parse::<usize>()
            .map_err(|_| format!("{FILE}:{}: `{count}` is not a count", index + 1))?;
        entries.insert(((*rule).to_owned(), (*path).to_owned()), count);
    }
    Ok(entries)
}

/// Phần không đọc đĩa của [`apply`], để test được.
fn filter(baseline: &BTreeMap<Key, usize>, violations: Vec<Violation>) -> Filtered {
    let mut groups: BTreeMap<Key, Vec<Violation>> = BTreeMap::new();
    for violation in violations {
        let path = violation.location.rsplit_once(':').map_or(&*violation.location, |(p, _)| p);
        groups.entry((violation.rule.to_owned(), path.to_owned())).or_default().push(violation);
    }
    let mut kept = Vec::new();
    let mut warnings = Vec::new();
    for (key, found) in &groups {
        let allowed = baseline.get(key).copied().unwrap_or(0);
        if found.len() > allowed {
            for violation in found {
                let mut violation = violation.clone();
                if allowed > 0 {
                    violation.message.push_str(&format!(
                        " ({FILE} allows {allowed} existing case(s) in this file, now {}: fix the \
                         new one)",
                        found.len()
                    ));
                }
                kept.push(violation);
            }
        } else if found.len() < allowed {
            warnings.push(format!(
                "warning: [{}] {}: {} case(s) left, {FILE} still allows {allowed}: lower the count",
                key.0,
                key.1,
                found.len()
            ));
        }
    }
    for (rule, path) in baseline.keys().filter(|key| !groups.contains_key(*key)) {
        warnings
            .push(format!("warning: [{rule}] {path}: no case left — remove its line from {FILE}"));
    }
    Filtered { violations: kept, warnings }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn violation(location: &str) -> Violation {
        Violation { rule: "tenant-scope", location: location.to_owned(), message: "m".to_owned() }
    }

    #[test]
    fn tru_cho_cu_chan_cho_moi_nhac_ha_so() {
        let baseline = parse(
            "# chú thích\ntenant-scope 2 a.rs  # dòng 3, 9\ntenant-scope 1 b.rs\n\
             tenant-scope 3 c.rs\ntenant-scope 1 gone.rs\n",
        )
        .unwrap();
        let result = filter(
            &baseline,
            vec![
                violation("a.rs:3"),
                violation("a.rs:9"),
                violation("b.rs:1"),
                violation("b.rs:5"),
                violation("c.rs:2"),
                violation("new.rs:7"),
            ],
        );
        let kept: Vec<&str> = result.violations.iter().map(|v| v.location.as_str()).collect();
        // a: đủ số, miễn; b: thêm một chỗ → báo cả hai; new: không có trong danh sách.
        assert_eq!(kept, ["b.rs:1", "b.rs:5", "new.rs:7"]);
        assert!(result.violations[0].message.contains("allows 1 existing case(s)"));
        assert_eq!(result.violations[2].message, "m");
        assert_eq!(result.warnings.len(), 2, "{:?}", result.warnings);
        assert!(result.warnings[0].contains("c.rs: 1 case(s) left"));
        assert!(result.warnings[1].contains("gone.rs: no case left"));
    }

    #[test]
    fn dong_sai_dang_bi_tu_choi() {
        assert!(parse("tenant-scope a.rs\n").is_err());
        assert!(parse("tenant-scope x a.rs\n").is_err());
    }
}
