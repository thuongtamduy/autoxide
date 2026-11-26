// Bất biến của phân quyền (D-20261126-independent-review, mục 71-G9): thử phá bằng property test.
//
// Chỉ thử phần thuần (không database): các phép thu hẹp quyền không bao giờ được CẤP THÊM, "cấm thắng
// cho phép" ở mọi thứ tự áp, và bản chụp người gọi (job chạy sau) khôi phục đúng quyền.
use super::*;
use proptest::prelude::*;
use std::collections::BTreeSet;

const POOL: [&str; 6] =
    ["notes.read", "notes.write", "notes.purge", "notes.export", "notes.share", "notes.audit"];

// `#[cfg(test)]` để luật i18n-field của xtask bỏ qua mô tả quyền dùng riêng cho test.
#[cfg(test)]
fn catalog() -> PermissionCatalog {
    let permissions = [
        Permission::new("notes.read", "Read."),
        Permission::new("notes.write", "Write."),
        Permission::new("notes.purge", "Purge."),
        Permission::new("notes.export", "Export."),
        Permission::new("notes.share", "Share."),
        Permission::new("notes.audit", "Audit."),
    ];
    PermissionCatalog::new(vec![("notes", permissions.to_vec())]).unwrap()
}

// Một tập con của `POOL`.
fn subset() -> impl Strategy<Value = BTreeSet<&'static str>> {
    proptest::collection::btree_set(proptest::sample::select(POOL.to_vec()), 0..=POOL.len())
}

// Người gọi: hoặc có mọi quyền, hoặc đúng một tập quyền ở độ sâu `tenant`.
fn caller() -> impl Strategy<Value = Principal> {
    (any::<bool>(), subset()).prop_map(|(all, names)| {
        let user = Uuid::from_u128(7);
        if all {
            Principal::with_all_permissions(user)
        } else {
            Principal::new(user, names.into_iter().map(str::to_owned))
        }
    })
}

fn granted(principal: &Principal) -> BTreeSet<&'static str> {
    POOL.iter().copied().filter(|name| principal.allows(name)).collect()
}

proptest! {
    // `only` chỉ thu hẹp: kết quả = quyền đang có ∩ danh sách, không bao giờ thêm.
    #[test]
    fn only_la_giao_chu_khong_bao_gio_cap_them(principal in caller(), names in subset()) {
        let before = granted(&principal);
        let after = granted(&principal.clone().only(names.iter().copied()));
        let expected: BTreeSet<_> = before.intersection(&names).copied().collect();
        prop_assert_eq!(after, expected);
    }

    // `without`: quyền bị cấm mất hẳn (kể cả người có mọi quyền), quyền khác giữ nguyên, và `is_denied` ghi đúng.
    #[test]
    fn cam_thang_cho_phep(principal in caller(), banned in subset()) {
        let before = granted(&principal);
        let after = principal.clone().without(banned.iter().map(|name| (*name).to_owned()), &catalog());
        let granted_after = granted(&after);
        let expected: BTreeSet<_> = before.difference(&banned).copied().collect();
        prop_assert_eq!(&granted_after, &expected);
        for name in &banned {
            prop_assert!(!after.allows(name), "{name} bị cấm mà vẫn có");
            let catalog = catalog();
            let permission = catalog.iter().find(|permission| permission.name() == *name).unwrap();
            let error = after.require(permission).unwrap_err();
            prop_assert_eq!(error.code(), "permission_denied");
        }
    }

    // `within` (rào của khách cha): không bao giờ vượt tập cho phép, và không bao giờ cấp thêm.
    #[test]
    fn rao_khong_cap_them_va_khong_vuot_tap_cho_phep(principal in caller(), allowed in subset()) {
        let before = granted(&principal);
        let allowed_owned: BTreeSet<String> = allowed.iter().map(|name| (*name).to_owned()).collect();
        let after = granted(&principal.clone().within(Some(&allowed_owned), &catalog()));
        prop_assert!(after.is_subset(&before));
        prop_assert!(after.is_subset(&allowed));
        let expected: BTreeSet<_> = before.intersection(&allowed).copied().collect();
        prop_assert_eq!(after, expected);
        // Không có rào: giữ nguyên.
        prop_assert_eq!(granted(&principal.clone().within(None, &catalog())), before);
    }

    // Cấm và rào áp theo thứ tự nào cũng ra cùng tập quyền (không có thứ tự "lách" được).
    #[test]
    fn cam_va_rao_khong_phu_thuoc_thu_tu(principal in caller(), banned in subset(), allowed in subset()) {
        let banned_names = || banned.iter().map(|name| (*name).to_owned());
        let allowed_owned: BTreeSet<String> = allowed.iter().map(|name| (*name).to_owned()).collect();
        let catalog = catalog();
        let first = principal.clone().without(banned_names(), &catalog).within(Some(&allowed_owned), &catalog);
        let second = principal.clone().within(Some(&allowed_owned), &catalog).without(banned_names(), &catalog);
        prop_assert_eq!(granted(&first), granted(&second));
        for name in &banned {
            prop_assert!(!first.allows(name) && !second.allows(name));
        }
    }

    // Bản chụp ghi vào payload job rồi khôi phục: đúng quyền, đúng chủ, đúng phạm vi (qua JSON như thật).
    #[test]
    fn ban_chup_khoi_phuc_dung_quyen(principal in caller(), names in subset()) {
        let principal = principal.only(names.iter().copied());
        let json = serde_json::to_value(principal.snapshot()).unwrap();
        let restored = serde_json::from_value::<PrincipalSnapshot>(json).unwrap().restore();
        prop_assert_eq!(granted(&restored), granted(&principal));
        prop_assert_eq!(restored.grant_map(), principal.grant_map());
        prop_assert_eq!(restored.user_id(), principal.user_id());
        prop_assert_eq!(restored.tenant_id(), principal.tenant_id());
    }
}
