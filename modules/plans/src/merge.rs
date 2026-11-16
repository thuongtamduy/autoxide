//! Gộp quyền dùng của một khách (hàm thuần, test riêng): gói chính ∪ gói thêm, rồi áp ghi đè.
//!
//! - Không có gói chính (không thuê bao, không gói mặc định) → không giới hạn.
//! - Module, tính năng = của gói chính ∪ của các gói thêm, rồi ghi đè bật / tắt.
//! - Hạn mức = của gói chính + Σ (của gói thêm × số lượng); `None` (không giới hạn) thắng mọi số;
//!   rồi ghi đè thay hẳn. Ghi đè hết hạn thì bỏ qua.
//! - Module `always_enabled` luôn bật dù gói không nêu — kernel tự xét, không cần ở đây.

use std::collections::{BTreeMap, BTreeSet};

use autoxide_kernel::entitlement::{TenantAccess, TenantEntitlements};
use chrono::{DateTime, Utc};

/// Phần quyền dùng một gói cấp.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Grants {
    pub modules: BTreeSet<String>,
    pub features: BTreeSet<String>,
    /// `None` = không giới hạn.
    pub limits: BTreeMap<String, Option<i64>>,
}

/// Loại ghi đè.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OverrideKind {
    Module,
    Feature,
    Limit,
}

impl OverrideKind {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        match text {
            "module" => Some(OverrideKind::Module),
            "feature" => Some(OverrideKind::Feature),
            "limit" => Some(OverrideKind::Limit),
            _ => None,
        }
    }
}

/// Một ghi đè riêng của khách.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Override {
    pub kind: OverrideKind,
    pub key: String,
    /// Với module / tính năng.
    pub enabled: bool,
    /// Với hạn mức: `None` = không giới hạn.
    pub max_value: Option<i64>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Quyền dùng của khách từ gói chính `base` (của thuê bao, hay gói mặc định), các gói thêm kèm số
/// lượng, các ghi đè, và quyền ghi `access`, tại thời điểm `now`.
pub(crate) fn merge(
    base: Option<&Grants>,
    addons: &[(Grants, i64)],
    overrides: &[Override],
    access: TenantAccess,
    now: DateTime<Utc>,
) -> TenantEntitlements {
    let Some(base) = base else {
        return TenantEntitlements::unrestricted();
    };
    let mut merged = TenantEntitlements {
        unrestricted: false,
        access,
        modules: base.modules.clone(),
        features: base.features.clone(),
        limits: base.limits.clone(),
    };
    for (addon, quantity) in addons {
        merged.modules.extend(addon.modules.iter().cloned());
        merged.features.extend(addon.features.iter().cloned());
        for (key, value) in &addon.limits {
            let extra = value.map(|value| value.saturating_mul(*quantity));
            let total = match (merged.limits.get(key).copied(), extra) {
                // `None` (không giới hạn) thắng mọi số.
                (Some(None), _) | (_, None) => None,
                (Some(Some(have)), Some(extra)) => Some(have.saturating_add(extra)),
                (None, Some(extra)) => Some(extra),
            };
            merged.limits.insert(key.clone(), total);
        }
    }
    for item in overrides {
        if item.expires_at.is_some_and(|expires_at| expires_at <= now) {
            continue;
        }
        match item.kind {
            OverrideKind::Module => toggle(&mut merged.modules, &item.key, item.enabled),
            OverrideKind::Feature => toggle(&mut merged.features, &item.key, item.enabled),
            OverrideKind::Limit => {
                merged.limits.insert(item.key.clone(), item.max_value);
            }
        }
    }
    merged
}

fn toggle(set: &mut BTreeSet<String>, key: &str, enabled: bool) {
    if enabled {
        set.insert(key.to_owned());
    } else {
        set.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grants(modules: &[&str], features: &[&str], limits: &[(&str, Option<i64>)]) -> Grants {
        Grants {
            modules: modules.iter().map(|m| (*m).to_owned()).collect(),
            features: features.iter().map(|f| (*f).to_owned()).collect(),
            limits: limits.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
        }
    }

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap()
    }

    #[test]
    fn khong_thue_bao_khong_goi_mac_dinh_la_khong_gioi_han() {
        let merged = merge(None, &[], &[], TenantAccess::Full, now());
        assert!(merged.unrestricted);
        assert!(merged.has_module("content") && merged.has_feature("content.ai_suggestions"));
        assert_eq!(merged.limit("files.storage_bytes"), None);
    }

    #[test]
    fn khach_van_hanh_luon_khong_gioi_han() {
        // Khách nền tảng không bao giờ tới hàm gộp: tay cầm của kernel trả "không giới hạn" trước
        // khi hỏi `plans` (D-323). Ở đây chỉ giữ hợp đồng: gộp với gói vẫn có giới hạn.
        let base = grants(&["content"], &[], &[("files.storage_bytes", Some(10))]);
        let merged = merge(Some(&base), &[], &[], TenantAccess::Full, now());
        assert!(!merged.unrestricted);
        assert_eq!(merged.limit("files.storage_bytes"), Some(10));
    }

    #[test]
    fn goi_them_cong_han_muc_va_none_thang() {
        let base = grants(
            &["content"],
            &["content.ai_suggestions"],
            &[
                ("organizations.units", Some(3)),
                ("files.storage_bytes", Some(1000)),
                ("auth.members", None),
            ],
        );
        let more_units =
            grants(&["forms"], &[], &[("organizations.units", Some(5)), ("auth.members", Some(2))]);
        let unlimited_files = grants(&[], &["forms.export"], &[("files.storage_bytes", None)]);
        let merged = merge(
            Some(&base),
            &[(more_units, 2), (unlimited_files, 1)],
            &[],
            TenantAccess::Full,
            now(),
        );
        assert_eq!(merged.limit("organizations.units"), Some(13), "3 + 5 × 2");
        assert_eq!(merged.limit("files.storage_bytes"), None, "gói thêm không giới hạn thắng");
        assert_eq!(merged.limit("auth.members"), None, "gói chính không giới hạn thắng");
        assert!(merged.has_module("content") && merged.has_module("forms"));
        assert!(merged.has_feature("forms.export") && merged.has_feature("content.ai_suggestions"));
        assert_eq!(merged.limit("notes.notes"), Some(0), "không gói nào nêu: không được dùng");
    }

    #[test]
    fn ghi_de_thay_han_va_het_han_thi_bo() {
        let base = grants(
            &["content", "forms"],
            &["content.ai_suggestions"],
            &[("organizations.units", Some(3))],
        );
        let later = Some(now() + chrono::Duration::days(1));
        let earlier = Some(now() - chrono::Duration::days(1));
        let overrides = [
            Override {
                kind: OverrideKind::Limit,
                key: "organizations.units".into(),
                enabled: true,
                max_value: Some(1),
                expires_at: later,
            },
            Override {
                kind: OverrideKind::Module,
                key: "forms".into(),
                enabled: false,
                max_value: None,
                expires_at: None,
            },
            Override {
                kind: OverrideKind::Module,
                key: "menus".into(),
                enabled: true,
                max_value: None,
                expires_at: None,
            },
            // Hết hạn: bỏ qua.
            Override {
                kind: OverrideKind::Feature,
                key: "content.ai_suggestions".into(),
                enabled: false,
                max_value: None,
                expires_at: earlier,
            },
            Override {
                kind: OverrideKind::Limit,
                key: "files.storage_bytes".into(),
                enabled: true,
                max_value: None,
                expires_at: None,
            },
        ];
        let merged = merge(Some(&base), &[], &overrides, TenantAccess::ReadOnly, now());
        assert_eq!(merged.limit("organizations.units"), Some(1), "ghi đè thay hẳn, kể cả nhỏ hơn");
        assert!(!merged.has_module("forms") && merged.has_module("menus"));
        assert!(merged.has_feature("content.ai_suggestions"), "ghi đè hết hạn bị bỏ");
        assert_eq!(merged.limit("files.storage_bytes"), None, "ghi đè thành không giới hạn");
        assert_eq!(merged.access, TenantAccess::ReadOnly);
    }

    #[test]
    fn module_always_enabled_luon_bat() {
        // Gói không nêu `auth`; kernel xét `always_enabled` trước khi hỏi quyền dùng.
        let base = grants(&["content"], &[], &[]);
        let merged = merge(Some(&base), &[], &[], TenantAccess::Full, now());
        assert!(!merged.has_module("auth"));
        // Phần kernel (`EntitlementsHandle::module_enabled` trả `true` cho `auth`) có test ở bài
        // HTTP `module_nen_luon_bat_du_goi_khong_neu` của module này.
    }
}
