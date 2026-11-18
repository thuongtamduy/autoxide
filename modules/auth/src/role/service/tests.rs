use std::sync::Arc;

use super::*;
use autoxide_kernel::access::Permission;
use autoxide_kernel::error::ErrorKind;
use autoxide_kernel::pagination::Order;
use autoxide_testkit::TestDatabase;

use crate::testing::Services;
use crate::user::NewUser;

const NOTES_WRITE: Permission = Permission::new("notes.write", "Write notes.");

fn input(name: &str, permissions: &[&str]) -> RoleInput {
    RoleInput {
        requires_mfa: false,
        grants: Vec::new(),
        name: name.to_owned(),
        description: String::new(),
        permissions: permissions.iter().map(|p| (*p).to_owned()).collect(),
    }
}

async fn user(services: &Services, email: &str) -> Uuid {
    let tenant = services.tenant.clone();
    let input = NewUser { email: email.to_owned(), name: "U".to_owned(), password: None };
    services.users.create(&tenant, Actor::system(), input).await.unwrap().id
}

async fn admin_role(roles: &RoleService, tenant: &TenantContext) -> Role {
    let page = roles.list(tenant, &PageRequest::first(200, Order::Asc)).await.unwrap();
    page.data.into_iter().find(|role| role.all_permissions).unwrap()
}

#[tokio::test]
async fn role_admin_dung_san_va_khong_sua_xoa_duoc() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let (roles, tenant) = (services.roles.clone(), services.tenant.clone());

        let admin = admin_role(&roles, &tenant).await;

        assert_eq!(admin.name, "admin");
        assert!(admin.permissions.is_empty());
        let changed = roles
            .update(&tenant, Actor::system(), admin.id, input("boss", &[]), None)
            .await
            .unwrap_err();
        let deleted = roles.delete(&tenant, Actor::system(), admin.id).await.unwrap_err();
        assert_eq!(changed.code(), "auth.role_builtin");
        assert_eq!(deleted.code(), "auth.role_builtin");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tao_sua_xoa_role() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let (roles, tenant) = (services.roles.clone(), services.tenant.clone());

        let created = roles
            .create(
                &tenant,
                Actor::system(),
                input(" editor ", &["notes.write", "auth.roles.read", "notes.write"]),
            )
            .await
            .unwrap();
        assert_eq!(created.name, "editor");
        assert_eq!(created.permissions, ["auth.roles.read", "notes.write"]);
        assert!(!created.all_permissions);

        let changed = roles
            .update(&tenant, Actor::system(), created.id, input("writer", &["notes.write"]), None)
            .await
            .unwrap();
        assert_eq!(changed.name, "writer");
        assert_eq!(changed.permissions, ["notes.write"]);
        assert_eq!(roles.get(&tenant, created.id).await.unwrap(), changed);

        roles.delete(&tenant, Actor::system(), created.id).await.unwrap();
        assert_eq!(roles.get(&tenant, created.id).await.unwrap_err().code(), "auth.role_not_found");
        assert_eq!(
            roles.delete(&tenant, Actor::system(), created.id).await.unwrap_err().code(),
            "auth.role_not_found"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn du_lieu_sai_va_ten_trung() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let (roles, tenant) = (services.roles.clone(), services.tenant.clone());

        let invalid = roles
            .create(&tenant, Actor::system(), input("Bad Name", &["notes.write", "nope.read"]))
            .await
            .unwrap_err();
        let fields: Vec<(&str, &str)> =
            invalid.fields().iter().map(|f| (f.field.as_str(), f.code.as_str())).collect();
        assert_eq!(fields, [("name", "pattern"), ("permissions[1]", "unknown")]);

        roles.create(&tenant, Actor::system(), input("editor", &[])).await.unwrap();
        let taken = roles.create(&tenant, Actor::system(), input("editor", &[])).await.unwrap_err();
        assert_eq!(taken.kind(), ErrorKind::Conflict);
        assert_eq!(taken.code(), "auth.role_name_taken");
        let admin = roles.create(&tenant, Actor::system(), input("admin", &[])).await.unwrap_err();
        assert_eq!(admin.code(), "auth.role_name_taken");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn quyen_cua_nguoi_dung_la_hop_cac_role() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let roles = &services.roles;
        let reader = roles
            .create(&tenant, Actor::system(), input("reader", &["auth.roles.read"]))
            .await
            .unwrap();
        let writer = roles
            .create(&tenant, Actor::system(), input("writer", &["notes.write"]))
            .await
            .unwrap();
        let ada = user(&services, "ada@example.com").await;

        assert_eq!(
            roles.principal(&tenant, ada).await.unwrap().permissions(),
            Vec::<String>::new()
        );
        let assigned = roles
            .set_user_roles(&tenant, Actor::system(), ada, vec![writer.id, reader.id])
            .await
            .unwrap();

        let names: Vec<&str> = assigned.iter().map(|role| role.name.as_str()).collect();
        assert_eq!(names, ["reader", "writer"]);
        let principal = roles.principal(&tenant, ada).await.unwrap();
        assert_eq!(principal.permissions(), ["auth.roles.read", "notes.write"]);
        assert!(principal.has(&NOTES_WRITE));
        assert_eq!(roles.user_roles(&tenant, ada).await.unwrap(), assigned);

        // Sửa role: người có role nhận permission mới ngay.
        roles
            .update(&tenant, Actor::system(), writer.id, input("writer", &[]), None)
            .await
            .unwrap();
        assert!(!roles.principal(&tenant, ada).await.unwrap().has(&NOTES_WRITE));

        let admin = admin_role(roles, &tenant).await;
        roles.set_user_roles(&tenant, Actor::system(), ada, vec![admin.id]).await.unwrap();
        assert!(roles.principal(&tenant, ada).await.unwrap().has_all_permissions());
    })
    .await
    .unwrap();
}

// Câu gộp của đường nóng (D-319) cho cùng quyền với `principal`, kèm đúng dòng người dùng.
#[tokio::test]
async fn nguoi_dung_kem_quyen_trong_mot_cau_khop_principal() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let roles = &services.roles;
        let ada = user(&services, "ada@example.com").await;
        assert!(roles.user_principal(&tenant, Uuid::now_v7()).await.unwrap().is_none());

        // Chưa có role: vẫn ra người dùng (LEFT JOIN), không permission nào.
        let (found, principal) = roles.user_principal(&tenant, ada).await.unwrap().unwrap();
        assert_eq!((found.id, found.email.as_str()), (ada, "ada@example.com"));
        assert!(principal.permissions().is_empty());
        assert!(!principal.has_all_permissions());

        let reader = roles
            .create(&tenant, Actor::system(), input("reader", &["auth.roles.read"]))
            .await
            .unwrap();
        let writer = roles
            .create(&tenant, Actor::system(), input("writer", &["notes.write"]))
            .await
            .unwrap();
        let empty = roles.create(&tenant, Actor::system(), input("empty", &[])).await.unwrap();
        let mut guarded = input("guarded", &["auth.users.read"]);
        guarded.requires_mfa = true;
        let guarded = roles.create(&tenant, Actor::system(), guarded).await.unwrap();
        let held = vec![reader.id, writer.id, empty.id, guarded.id];
        roles.set_user_roles(&tenant, Actor::system(), ada, held).await.unwrap();
        let (_, principal) = roles.user_principal(&tenant, ada).await.unwrap().unwrap();
        // Role bắt buộc MFA mà chưa bật MFA: quyền của role đó bị giữ, như `principal`.
        assert_eq!(principal.permissions(), ["auth.roles.read", "notes.write"]);
        assert_eq!(
            principal.permissions(),
            roles.principal(&tenant, ada).await.unwrap().permissions()
        );

        let admin = admin_role(roles, &tenant).await;
        roles.set_user_roles(&tenant, Actor::system(), ada, vec![admin.id]).await.unwrap();
        let (_, principal) = roles.user_principal(&tenant, ada).await.unwrap().unwrap();
        assert!(principal.has_all_permissions());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gan_role_cho_nguoi_hay_role_khong_ton_tai() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let ada = user(&services, "ada@example.com").await;

        let no_user =
            services.roles.set_user_roles(&tenant, Actor::system(), Uuid::now_v7(), vec![]).await;
        let no_role = services
            .roles
            .set_user_roles(&tenant, Actor::system(), ada, vec![Uuid::now_v7()])
            .await;
        let read_no_user = services.roles.user_roles(&tenant, Uuid::now_v7()).await;

        assert_eq!(no_user.unwrap_err().code(), "auth.user_not_found");
        assert_eq!(no_role.unwrap_err().fields()[0].code, "unknown");
        assert_eq!(read_no_user.unwrap_err().code(), "auth.user_not_found");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khong_bo_duoc_admin_cuoi_cung() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let roles = &services.roles;
        let admin = admin_role(roles, &tenant).await;
        let ada = user(&services, "ada@example.com").await;
        let bob = user(&services, "bob@example.com").await;
        roles.set_user_roles(&tenant, Actor::system(), ada, vec![admin.id]).await.unwrap();

        let error = roles.set_user_roles(&tenant, Actor::system(), ada, vec![]).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Conflict);
        assert_eq!(error.code(), "auth.last_admin");
        assert!(roles.principal(&tenant, ada).await.unwrap().has_all_permissions());

        // Có admin khác thì bỏ được.
        roles.set_user_roles(&tenant, Actor::system(), bob, vec![admin.id]).await.unwrap();
        roles.set_user_roles(&tenant, Actor::system(), ada, vec![]).await.unwrap();
    })
    .await
    .unwrap();
}

/// Nhiều request thay role của CÙNG một người dùng cùng lúc: mọi request thành
/// công, và kết quả cuối là đúng danh sách của một request nào đó (không trộn) —
/// trên cả hai database.
#[tokio::test]
async fn gan_role_dong_thoi_khong_tron_lan() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let roles = services.roles.clone();
        let ada = user(&services, "ada@example.com").await;
        let mut sets = Vec::new();
        for index in 0..4 {
            let first = roles
                .create(&tenant, Actor::system(), input(&format!("a{index}"), &[]))
                .await
                .unwrap()
                .id;
            let second = roles
                .create(&tenant, Actor::system(), input(&format!("b{index}"), &[]))
                .await
                .unwrap()
                .id;
            sets.push(BTreeSet::from([first, second]));
        }

        let attempts: Vec<_> = sets
            .iter()
            .map(|set| {
                let roles: Arc<RoleService> = roles.clone();
                let ids: Vec<Uuid> = set.iter().copied().collect();
                let tenant = tenant.clone();
                tokio::spawn(async move {
                    roles.set_user_roles(&tenant, Actor::system(), ada, ids).await
                })
            })
            .collect();
        for attempt in attempts {
            attempt.await.unwrap().unwrap();
        }

        let last: BTreeSet<Uuid> =
            roles.user_roles(&tenant, ada).await.unwrap().into_iter().map(|role| role.id).collect();
        assert!(sets.contains(&last), "{last:?}");
    })
    .await
    .unwrap();
}
