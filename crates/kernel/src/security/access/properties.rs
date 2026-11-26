// Bất biến của phân quyền (D-20261126-independent-review, mục 71-G9): thử phá bằng property test.
//
// Chỉ thử phần thuần (không database): các phép thu hẹp quyền không bao giờ được CẤP THÊM, "cấm thắng
// cho phép" ở mọi thứ tự áp, quyền còn lại giữ đúng phạm vi (nút, độ sâu), và bản chụp người gọi (job chạy
// sau) khôi phục đúng mọi trường. Người gọi được sinh đa dạng: độ sâu Own/Unit/Subtree/Tenant ở nhiều nút,
// nhiều grant cho một permission, hàng rào (fence), API key, khách khác mặc định, tài khoản khách mua hàng,
// và cả tên permission không có trong danh mục.

use super::*;
use proptest::prelude::*;
use std::collections::BTreeSet;

// Permission có trong danh mục.
const CATALOGED: [&str; 6] =
    ["notes.read", "notes.write", "notes.purge", "notes.export", "notes.share", "notes.audit"];
// Tên không có trong danh mục (người gọi có thể mang tên cũ / của module đã gỡ).
const GHOSTS: [&str; 2] = ["ghost.read", "ghost.write"];

// `#[cfg(test)]` để luật i18n-field của xtask bỏ qua mô tả quyền dùng riêng cho test (nó quét cả file này).
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

fn permission_of(name: &str) -> Permission {
    *catalog().iter().find(|permission| permission.name() == name).unwrap()
}

// Một tập con của danh mục.
fn cataloged_subset() -> impl Strategy<Value = BTreeSet<&'static str>> {
    proptest::collection::btree_set(
        proptest::sample::select(CATALOGED.to_vec()),
        0..=CATALOGED.len(),
    )
}

// Một tập tên bất kỳ, kể cả tên ngoài danh mục.
fn any_names() -> impl Strategy<Value = BTreeSet<&'static str>> {
    let pool: Vec<&'static str> = CATALOGED.iter().chain(GHOSTS.iter()).copied().collect();
    proptest::collection::btree_set(proptest::sample::select(pool.clone()), 0..=pool.len())
}

fn org(seed: u8) -> OrgId {
    OrgId::from_uuid(Uuid::from_u128(u128::from(seed) + 1))
}

fn scope() -> impl Strategy<Value = Scope> {
    prop_oneof![Just(Scope::Own), Just(Scope::Unit), Just(Scope::Subtree), Just(Scope::Tenant)]
}

// Người gọi đa dạng (xem đầu file).
fn caller() -> impl Strategy<Value = Principal> {
    let pool: Vec<&'static str> = CATALOGED.iter().chain(GHOSTS.iter()).copied().collect();
    let grants =
        proptest::collection::vec((proptest::sample::select(pool), 0u8..4, scope()), 0..10);
    (0u8..6, grants, proptest::option::of(any::<u128>()), proptest::option::of(0u8..4), 1u128..4)
        .prop_map(|(kind, grants, api_key, fence, tenant)| {
            let user = Uuid::from_u128(7);
            let mut principal = match kind {
                0 => Principal::with_all_permissions(user),
                1 => Principal::customer(user),
                _ => Principal::with_grants(
                    user,
                    grants
                        .into_iter()
                        .map(|(name, node, scope)| (name.to_owned(), Grant::new(org(node), scope))),
                ),
            }
            .in_tenant(TenantId::from_uuid(Uuid::from_u128(tenant)));
            if let Some(key) = api_key {
                principal = principal.via_api_key(Uuid::from_u128(key));
            }
            if let Some(node) = fence {
                principal = principal.with_fence(org(node));
            }
            principal
        })
}

fn granted<'a>(
    principal: &Principal,
    names: impl IntoIterator<Item = &'a str>,
) -> BTreeSet<&'a str> {
    names.into_iter().filter(|name| principal.allows(name)).collect()
}

fn catalog_names() -> impl Iterator<Item = &'static str> {
    CATALOGED.iter().copied()
}

// Các grant (nút, độ sâu) của `name`: người có mọi quyền thì là độ sâu `tenant` ở gốc của khách.
fn grants_of(principal: &Principal, name: &str) -> Option<Vec<Grant>> {
    match principal.grant_map() {
        Some(map) => map.get(name).cloned(),
        None => Some(vec![Grant::new(root_of(principal.tenant_id()), Scope::Tenant)]),
    }
}

// Mỗi quyền còn lại sau phép thu hẹp giữ đúng grant của nó (không đổi nút, không nâng độ sâu).
fn assert_grants_kept(
    before: &Principal,
    after: &Principal,
    names: impl IntoIterator<Item = &'static str>,
) -> Result<(), TestCaseError> {
    for name in names.into_iter().filter(|name| after.allows(name)) {
        prop_assert_eq!(grants_of(after, name), grants_of(before, name), "{}", name);
    }
    Ok(())
}

proptest! {
    // `only` chỉ thu hẹp: kết quả = quyền đang có ∩ danh sách, grant giữ nguyên, không bao giờ thêm.
    #[test]
    fn only_la_giao_chu_khong_bao_gio_cap_them(principal in caller(), names in any_names()) {
        let everything = CATALOGED.iter().chain(GHOSTS.iter()).copied();
        let before = granted(&principal, everything.clone());
        let after_principal = principal.clone().only(names.iter().copied());
        let after = granted(&after_principal, everything.clone());
        let expected: BTreeSet<_> = before.intersection(&names).copied().collect();
        prop_assert_eq!(&after, &expected);
        for name in after.iter().copied() {
            prop_assert_eq!(grants_of(&after_principal, name), grants_of(&principal, name), "{}", name);
        }
        // Quyền giữ lại thì `require` cho qua (danh mục).
        for name in catalog_names().filter(|name| after.contains(name)) {
            prop_assert!(after_principal.require(&permission_of(name)).is_ok(), "{}", name);
        }
    }

    // `without`: quyền bị cấm mất hẳn (kể cả người có mọi quyền), quyền khác giữ nguyên cả grant.
    #[test]
    fn cam_thang_cho_phep(principal in caller(), banned in cataloged_subset()) {
        let before = granted(&principal, catalog_names());
        let after = principal.clone().without(banned.iter().map(|name| (*name).to_owned()), &catalog());
        let expected: BTreeSet<_> = before.difference(&banned).copied().collect();
        prop_assert_eq!(granted(&after, catalog_names()), expected.clone());
        for name in &banned {
            prop_assert!(!after.allows(name), "{name} bị cấm mà vẫn có");
            let error = after.require(&permission_of(name)).unwrap_err();
            prop_assert_eq!(error.code(), "permission_denied");
            prop_assert!(after.is_denied(name));
        }
        for name in &expected {
            prop_assert!(after.require(&permission_of(name)).is_ok(), "{}", name);
        }
        assert_grants_kept(&principal, &after, catalog_names())?;
    }

    // `within` (rào của khách cha): không vượt tập cho phép, không cấp thêm, grant giữ nguyên.
    #[test]
    fn rao_khong_cap_them_va_khong_vuot_tap_cho_phep(principal in caller(), allowed in cataloged_subset()) {
        let before = granted(&principal, catalog_names());
        let allowed_owned: BTreeSet<String> = allowed.iter().map(|name| (*name).to_owned()).collect();
        let after = principal.clone().within(Some(&allowed_owned), &catalog());
        let granted_after = granted(&after, catalog_names());
        prop_assert!(granted_after.is_subset(&before));
        prop_assert!(granted_after.is_subset(&allowed));
        let expected: BTreeSet<_> = before.intersection(&allowed).copied().collect();
        prop_assert_eq!(&granted_after, &expected);
        for name in &expected {
            prop_assert!(after.require(&permission_of(name)).is_ok(), "{}", name);
        }
        assert_grants_kept(&principal, &after, catalog_names())?;
        // Không có rào: giữ nguyên.
        prop_assert_eq!(granted(&principal.clone().within(None, &catalog()), catalog_names()), before);
    }

    // Cấm và rào áp theo thứ tự nào cũng ra cùng quyền và cùng grant (không có thứ tự "lách" được).
    #[test]
    fn cam_va_rao_khong_phu_thuoc_thu_tu(
        principal in caller(),
        banned in cataloged_subset(),
        allowed in cataloged_subset(),
    ) {
        let banned_names = || banned.iter().map(|name| (*name).to_owned());
        let allowed_owned: BTreeSet<String> = allowed.iter().map(|name| (*name).to_owned()).collect();
        let catalog = catalog();
        let first = principal.clone().without(banned_names(), &catalog).within(Some(&allowed_owned), &catalog);
        let second = principal.clone().within(Some(&allowed_owned), &catalog).without(banned_names(), &catalog);
        prop_assert_eq!(granted(&first, catalog_names()), granted(&second, catalog_names()));
        prop_assert_eq!(first.grant_map(), second.grant_map());
        for name in &banned {
            prop_assert!(!first.allows(name) && !second.allows(name));
        }
    }

    // Bản chụp ghi vào payload job rồi khôi phục (qua JSON như thật): đúng quyền, grant, chủ, khách, key, hàng rào.
    #[test]
    fn ban_chup_khoi_phuc_dung_moi_truong(principal in caller(), names in any_names()) {
        for principal in [principal.clone(), principal.clone().only(names.iter().copied())] {
            let json = serde_json::to_value(principal.snapshot()).unwrap();
            let restored = serde_json::from_value::<PrincipalSnapshot>(json).unwrap().restore();
            let everything = CATALOGED.iter().chain(GHOSTS.iter()).copied();
            prop_assert_eq!(granted(&restored, everything.clone()), granted(&principal, everything));
            prop_assert_eq!(restored.grant_map(), principal.grant_map());
            prop_assert_eq!(restored.has_all_permissions(), principal.has_all_permissions());
            prop_assert_eq!(restored.user_id(), principal.user_id());
            prop_assert_eq!(restored.tenant_id(), principal.tenant_id());
            prop_assert_eq!(restored.api_key_id(), principal.api_key_id());
            prop_assert_eq!(restored.fence(), principal.fence());
            prop_assert_eq!(restored.realm(), principal.realm());
            prop_assert_eq!(restored.snapshot(), principal.snapshot());
        }
    }
}
