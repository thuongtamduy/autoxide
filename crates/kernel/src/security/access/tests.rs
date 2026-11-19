//! Test của `Principal` và danh mục permission.

use super::*;
use crate::error::ErrorKind;

#[test]
fn ban_chup_nguoi_goi_dung_lai_dung_quyen() {
    let org = OrgId::from_uuid(Uuid::now_v7());
    let principal = Principal::with_grants(
        Uuid::now_v7(),
        [
            ("content.read".to_owned(), Grant::new(org, Scope::Subtree)),
            ("content.write".to_owned(), Grant::new(org, Scope::Own)),
        ],
    )
    .via_api_key(Uuid::now_v7());
    let snapshot: PrincipalSnapshot =
        serde_json::from_value(serde_json::to_value(principal.snapshot()).unwrap()).unwrap();
    let restored = snapshot.restore();
    assert_eq!(restored.user_id(), principal.user_id());
    assert_eq!(restored.grant_map(), principal.grant_map());
    assert_eq!(restored.snapshot(), principal.snapshot());
    let admin = Principal::with_all_permissions(Uuid::now_v7());
    assert!(admin.snapshot().restore().has_all_permissions());
}


const READ: Permission = Permission::new("notes.read", "Read notes.");

struct OnlyOne(Uuid);

#[async_trait::async_trait]
impl PostingVerifier for OnlyOne {
    async fn is_verified(
        &self,
        _tenant: &crate::tenancy::TenantContext,
        user_id: Uuid,
    ) -> Result<bool, Error> {
        Ok(user_id == self.0)
    }
}

#[tokio::test]
async fn xac_thuc_tai_khoan_mac_dinh_cho_qua_khi_chua_ai_dang_ky() {
    let slot = PostingVerification::default();
    let early = slot.clone();
    let (verified, other) = (Uuid::now_v7(), Uuid::now_v7());
    assert!(!slot.is_registered());
    assert!(
        slot.is_verified(&crate::tenancy::test_tenant(), other).await.unwrap(),
        "no verifier: as before D-202"
    );
    slot.register("auth", Arc::new(OnlyOne(verified))).unwrap();
    assert!(slot.register("compliance", Arc::new(OnlyOne(verified))).is_err(), "one only");
    assert!(early.is_registered(), "clones see a later registration");
    assert!(early.is_verified(&crate::tenancy::test_tenant(), verified).await.unwrap());
    assert!(!early.is_verified(&crate::tenancy::test_tenant(), other).await.unwrap());
}
const WRITE: Permission = Permission::new("notes.write", "Write notes.");

#[test]
fn danh_muc_hop_le_sap_theo_ten() {
    let catalog = PermissionCatalog::new(vec![
        ("notes", vec![WRITE, READ]),
        ("auth", vec![Permission::new("auth.roles.manage", "Manage roles.")]),
    ])
    .unwrap();

    let names: Vec<&str> = catalog.iter().map(Permission::name).collect();
    assert_eq!(names, ["auth.roles.manage", "notes.read", "notes.write"]);
    assert!(catalog.contains("notes.read"));
    assert!(!catalog.contains("notes.delete"));
}

#[test]
fn ten_sai_trung_hoac_thieu_mo_ta_bi_bao_cung_luc() {
    let problems = PermissionCatalog::new(vec![(
        "notes",
        vec![
            Permission::new("notes", "No action."),
            Permission::new("tags.read", "Wrong module."),
            Permission::new("notes.Write", "Upper case."),
            Permission::new("notes..read", "Empty segment."),
            Permission::new("notes.read", "First."),
            Permission::new("notes.read", " "),
        ],
    )])
    .unwrap_err();

    assert_eq!(problems.len(), 6, "{problems:#?}");
    assert!(problems[0].contains("must be named `notes.<action>`"), "{problems:#?}");
    assert!(problems.iter().any(|problem| problem.contains("declared twice")));
    assert!(problems.iter().any(|problem| problem.contains("has no description")));
}

#[test]
fn principal_kiem_permission() {
    let user = Uuid::now_v7();
    let reader = Principal::new(user, ["notes.read".to_owned()]);
    let admin = Principal::with_all_permissions(user);

    assert!(reader.require(&READ).is_ok());
    let denied = reader.require(&WRITE).unwrap_err();
    assert_eq!(denied.kind(), ErrorKind::Forbidden);
    assert_eq!(denied.code(), "permission_denied");
    assert_eq!(
        denied.message(),
        "This action needs the permission `notes.write`. Ask an administrator for a role \
         that has it."
    );
    assert!(admin.require(&WRITE).is_ok());
    assert!(admin.has_all_permissions());
    assert_eq!(reader.permissions(), ["notes.read"]);
    assert!(reader.allows("notes.read") && !reader.allows("notes.write"));
    assert_eq!(reader.api_key_id(), None);
    let key = Uuid::now_v7();
    assert_eq!(reader.clone().via_api_key(key).api_key_id(), Some(key));
    assert_eq!(reader.user_id(), user);
}
