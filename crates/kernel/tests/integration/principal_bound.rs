//! `Principal::bounded_by` giữ cả phạm vi (P16, D-20261128-bound-keeps-scope): quyền hiện tại bị cắt theo nút và độ
//! sâu của bản chụp, không chỉ theo tên permission. Chạy trên cả SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::access::{Grant, Permission, PermissionCatalog, Principal, Scope};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, root_of};
use autoxide_testkit::{TestApp, TestDatabase};
use uuid::Uuid;

const READ: Permission = Permission::new("demo.read", "Read records.").scoped();
const WRITE: Permission = Permission::new("demo.write", "Write records.").scoped();
const EXPORT: Permission = Permission::new("demo.export", "Export records.");

fn org() -> OrgId {
    OrgId::from_uuid(Uuid::now_v7())
}

fn grants(principal: &Principal, permission: &Permission) -> Vec<Grant> {
    let mut list =
        principal.grant_map().unwrap().get(permission.name()).cloned().unwrap_or_default();
    list.sort_by_key(|grant| (grant.org.as_uuid(), grant.scope));
    list
}

#[tokio::test]
async fn gioi_han_giu_ca_pham_vi() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).build().await.unwrap();
        let tenant = app.default_tenant();
        let catalog = PermissionCatalog::new(vec![("demo", vec![READ, WRITE, EXPORT])]).unwrap();
        // Cây: gốc → a → a1; gốc → b.
        let root = root_of(DEFAULT_TENANT);
        let (a, a1, b) = (org(), org(), org());
        let write = app.database().begin_write().await.unwrap();
        OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, a, root, 8).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, a1, a, 8).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, b, root, 8).await.unwrap();
        write.commit().await.unwrap();
        let user = Uuid::now_v7();
        let g = |permission: &Permission, org, scope| {
            (permission.name().to_owned(), Grant::new(org, scope))
        };

        // Lúc lưu: đọc cả nhánh a, ghi đúng nút a1, xuất (không theo nút) ở b.
        let limit = Principal::with_grants(
            user,
            [g(&READ, a, Scope::Subtree), g(&WRITE, a1, Scope::Unit), g(&EXPORT, b, Scope::Tenant)],
        );
        // Bây giờ: được nới lên toàn khách, thêm grant ở b, và một permission mới.
        let now = Principal::with_grants(
            user,
            [
                g(&READ, root, Scope::Tenant),
                g(&WRITE, a, Scope::Subtree),
                g(&WRITE, b, Scope::Unit),
                g(&EXPORT, root, Scope::Tenant),
                ("demo.purge".to_owned(), Grant::new(root, Scope::Tenant)),
            ],
        );
        let bounded = now.clone().bounded_by(&tenant, &limit, &catalog).await.unwrap();
        assert_eq!(
            grants(&bounded, &READ),
            vec![Grant::new(a, Scope::Subtree)],
            "no wider than then"
        );
        assert_eq!(
            grants(&bounded, &WRITE),
            vec![Grant::new(a1, Scope::Unit)],
            "b dropped, a1 kept"
        );
        // Permission không theo nút: có ở cả hai bên là đủ, giữ grant hiện tại.
        assert_eq!(grants(&bounded, &EXPORT), vec![Grant::new(root, Scope::Tenant)]);
        assert!(!bounded.allows("demo.purge"), "nothing the snapshot lacked");
        assert_eq!(bounded.user_id(), user);

        // Bây giờ hẹp hơn lúc lưu: giữ phần hẹp hiện tại (sức mạnh = min của hai lúc).
        let narrower =
            Principal::with_grants(user, [g(&READ, a1, Scope::Own), g(&WRITE, b, Scope::Unit)]);
        let bounded = narrower.bounded_by(&tenant, &limit, &catalog).await.unwrap();
        assert_eq!(grants(&bounded, &READ), vec![Grant::new(a1, Scope::Own)]);
        assert!(!bounded.allows(WRITE.name()), "write at b only: apart from a1");

        // Người có mọi quyền bị cắt về đúng bản chụp; bản chụp "mọi quyền" không cắt gì.
        let admin = Principal::with_all_permissions(user);
        let bounded = admin.bounded_by(&tenant, &limit, &catalog).await.unwrap();
        assert_eq!(grants(&bounded, &READ), vec![Grant::new(a, Scope::Subtree)]);
        assert!(!bounded.has_all_permissions());
        let unbounded = now
            .clone()
            .bounded_by(&tenant, &Principal::with_all_permissions(user), &catalog)
            .await
            .unwrap();
        assert_eq!(unbounded, now);
    })
    .await
    .unwrap();
}
