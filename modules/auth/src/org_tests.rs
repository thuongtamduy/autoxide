//! Quyền theo nút của `auth` (bước 34 mục 2): role cấp permission kèm độ sâu, membership gắn
//! nút, API key gắn nút (hàng rào). Chạy trên SQLite và Postgres.

use std::collections::BTreeSet;

use autoxide_kernel::access::{Grant, Permission, Principal, Scope};
use autoxide_kernel::error::ErrorKind;
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, root_of};
use autoxide_testkit::TestDatabase;
use uuid::Uuid;

use crate::api_key::NewApiKey;
use crate::role::{MembershipFilter, Role, RoleInput};
use crate::testing::{Services, services};
use crate::user::NewUser;

const PASSWORD: &str = "correct horse battery staple";
const NOTES_READ: Permission = Permission::new("notes.read", "Read notes.").scoped();
const USERS_READ: Permission = Permission::new("auth.users.read", "Read users.");

/// Cây: gốc → A, B; A → A1.
struct Tree {
    root: OrgId,
    a: OrgId,
    b: OrgId,
    a1: OrgId,
}

async fn tree(services: &Services) -> Tree {
    let write = services.database.begin_write().await.unwrap();
    let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
    let (a, b, a1) = (org(), org(), org());
    OrgTree::insert(&write, DEFAULT_TENANT, a, root, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, b, root, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, a1, a, 8).await.unwrap();
    write.commit().await.unwrap();
    Tree { root, a, b, a1 }
}

fn org() -> OrgId {
    OrgId::from_uuid(Uuid::now_v7())
}

async fn user(services: &Services, email: &str) -> Uuid {
    let input = NewUser {
        email: email.to_owned(),
        name: email.to_owned(),
        password: Some(PASSWORD.to_owned()),
    };
    services.users.create(&services.tenant, Actor::system(), input).await.unwrap().id
}

async fn role(
    services: &Services,
    name: &str,
    permissions: &[&str],
    grants: &[(&str, Scope)],
) -> Role {
    let input = RoleInput {
        requires_mfa: false,
        name: name.to_owned(),
        description: String::new(),
        permissions: permissions.iter().map(|permission| (*permission).to_owned()).collect(),
        grants: grants
            .iter()
            .map(|(permission, scope)| ((*permission).to_owned(), *scope))
            .collect(),
    };
    services.roles.create(&services.tenant, Actor::system(), input).await.unwrap()
}

fn codes(error: &autoxide_kernel::error::Error) -> BTreeSet<String> {
    error.fields().iter().map(|field| field.code.clone()).collect()
}

#[tokio::test]
async fn role_cap_permission_kem_do_sau() {
    TestDatabase::run(async |test| {
        let services = services(test).await;
        let reader =
            role(&services, "reader", &["auth.users.read"], &[("notes.read", Scope::Subtree)])
                .await;
        assert_eq!(reader.permissions, ["auth.users.read", "notes.read"]);
        assert_eq!(
            reader.grants,
            [
                ("auth.users.read".to_owned(), Scope::Tenant),
                ("notes.read".to_owned(), Scope::Subtree)
            ]
        );
        // Permission không theo nút chỉ cấp cho cả khách; nêu một permission hai lần là sai.
        let input = RoleInput {
            requires_mfa: false,
            name: "bad".to_owned(),
            description: String::new(),
            permissions: vec!["notes.read".to_owned()],
            grants: vec![
                ("auth.users.read".to_owned(), Scope::Unit),
                ("notes.read".to_owned(), Scope::Own),
                ("notes.nope".to_owned(), Scope::Unit),
            ],
        };
        let error =
            services.roles.create(&services.tenant, Actor::system(), input).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Validation);
        assert_eq!(
            codes(&error),
            ["duplicate", "scope_not_supported", "unknown"]
                .map(str::to_owned)
                .into_iter()
                .collect()
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn membership_theo_nut_cho_quyen_dung_nut() {
    TestDatabase::run(async |test| {
        let services = services(test).await;
        let tenant = &services.tenant;
        let tree = tree(&services).await;
        let ann = user(&services, "ann@example.com").await;
        let reader = role(&services, "reader", &[], &[("notes.read", Scope::Subtree)]).await;
        let viewer = role(&services, "viewer", &["auth.users.read"], &[]).await;
        let admin = Principal::with_all_permissions(Uuid::now_v7());
        let wanted =
            vec![(tree.a.as_uuid(), vec![reader.id]), (tree.root.as_uuid(), vec![viewer.id])];
        let after = services.roles.set_memberships(tenant, &admin, ann, wanted).await.unwrap();
        assert_eq!(after.len(), 2);

        let principal = services.roles.principal(tenant, ann).await.unwrap();
        assert_eq!(principal.grants(&NOTES_READ), [Grant::new(tree.a, Scope::Subtree)]);
        assert_eq!(principal.grants(&USERS_READ), [Grant::new(tree.root, Scope::Tenant)]);
        let read = services.database.read();
        assert!(principal.reaches(read, tree.a1).await.unwrap());
        assert!(
            principal
                .require_at(
                    read,
                    &NOTES_READ,
                    autoxide_kernel::access::OrgTarget::new(tree.a1, None)
                )
                .await
                .is_ok()
        );
        assert!(
            principal
                .require_at(
                    read,
                    &NOTES_READ,
                    autoxide_kernel::access::OrgTarget::new(tree.b, None)
                )
                .await
                .is_err()
        );

        // Ai làm ở A1: trực tiếp không ai; kể cả kế thừa thì Ann (qua A và qua gốc).
        let page = autoxide_kernel::pagination::PageRequest::try_from(
            autoxide_kernel::pagination::PageQuery::default(),
        )
        .unwrap();
        let direct =
            MembershipFilter { org_id: Some(tree.a1.as_uuid()), ..MembershipFilter::default() };
        assert!(
            services.roles.list_memberships(tenant, &direct, &page).await.unwrap().data.is_empty()
        );
        let inherited = MembershipFilter { inherited: true, ..direct };
        let rows = services.roles.list_memberships(tenant, &inherited, &page).await.unwrap().data;
        let mut seen: Vec<(Uuid, bool)> =
            rows.iter().map(|row| (row.org_id, row.inherited)).collect();
        seen.sort();
        let mut want = vec![(tree.a.as_uuid(), true), (tree.root.as_uuid(), true)];
        want.sort();
        assert_eq!(seen, want);

        // Đường cũ: `set_user_roles` chỉ thay membership ở gốc, giữ membership ở A.
        services.roles.set_user_roles(tenant, Actor::system(), ann, vec![]).await.unwrap();
        let kept = services.roles.memberships(tenant, ann).await.unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].org_id, tree.a.as_uuid());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gan_membership_bi_chan_khi_vuot_quyen() {
    TestDatabase::run(async |test| {
        let services = services(test).await;
        let tenant = &services.tenant;
        let tree = tree(&services).await;
        let ann = user(&services, "ann@example.com").await;
        let reader = role(&services, "reader", &[], &[("notes.read", Scope::Subtree)]).await;
        let admin_role = services
            .roles
            .list(
                tenant,
                &autoxide_kernel::pagination::PageRequest::try_from(
                    autoxide_kernel::pagination::PageQuery::default(),
                )
                .unwrap(),
            )
            .await
            .unwrap()
            .data
            .into_iter()
            .find(|role| role.all_permissions)
            .unwrap();
        let admin = Principal::with_all_permissions(Uuid::now_v7());
        // Role `admin` chỉ ở gốc; nút lạ; một nút nêu hai lần.
        let unknown = org().as_uuid();
        let wanted = vec![
            (tree.a.as_uuid(), vec![admin_role.id]),
            (unknown, vec![reader.id]),
            (tree.b.as_uuid(), vec![reader.id]),
            (tree.b.as_uuid(), vec![]),
        ];
        let error = services.roles.set_memberships(tenant, &admin, ann, wanted).await.unwrap_err();
        assert_eq!(
            codes(&error),
            ["duplicate", "root_only", "unknown"].map(str::to_owned).into_iter().collect()
        );

        // Người gán chỉ quản lý role ở B: không gán được ở A, không gán role có permission mình
        // không có.
        let manager = Principal::with_grants(
            Uuid::now_v7(),
            [("auth.roles.manage".to_owned(), Grant::new(tree.b, Scope::Subtree))],
        );
        let error = services
            .roles
            .set_memberships(tenant, &manager, ann, vec![(tree.a.as_uuid(), vec![])])
            .await
            .unwrap_err();
        assert_eq!(codes(&error), ["out_of_scope".to_owned()].into_iter().collect());
        let error = services
            .roles
            .set_memberships(tenant, &manager, ann, vec![(tree.b.as_uuid(), vec![reader.id])])
            .await
            .unwrap_err();
        assert_eq!(codes(&error), ["more_than_you_have".to_owned()].into_iter().collect());
        assert!(services.roles.memberships(tenant, ann).await.unwrap().is_empty(), "không đổi gì");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gan_membership_dong_thoi() {
    TestDatabase::run(async |test| {
        let services = services(test).await;
        let tree = tree(&services).await;
        let ann = user(&services, "ann@example.com").await;
        let reader = role(&services, "reader", &[], &[("notes.read", Scope::Unit)]).await;
        let admin = Principal::with_all_permissions(Uuid::now_v7());
        let orgs = [tree.a, tree.b, tree.a1, tree.root];
        let mut tasks = Vec::new();
        for index in 0..8 {
            let (roles, tenant, admin) =
                (services.roles.clone(), services.tenant.clone(), admin.clone());
            let org = orgs[index % orgs.len()].as_uuid();
            let reader = reader.id;
            tasks.push(tokio::spawn(async move {
                roles
                    .set_memberships(&tenant, &admin, ann, vec![(org, vec![reader])])
                    .await
                    .map(|_| org)
            }));
        }
        let mut done = Vec::new();
        for task in tasks {
            done.push(task.await.unwrap().unwrap());
        }
        // Mỗi lần gán thay hết: cuối cùng đúng một membership, ở một trong các nút đã gán.
        let last = services.roles.memberships(&services.tenant, ann).await.unwrap();
        assert_eq!(last.len(), 1, "{last:?}");
        assert!(done.contains(&last[0].org_id));
        assert_eq!(last[0].roles.len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn api_key_ngoai_nhanh_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let services = services(test).await;
        let tenant = &services.tenant;
        let tree = tree(&services).await;
        let ann = user(&services, "ann@example.com").await;
        let reader = role(&services, "reader", &[], &[("notes.read", Scope::Subtree)]).await;
        let admin = Principal::with_all_permissions(Uuid::now_v7());
        services
            .roles
            .set_memberships(tenant, &admin, ann, vec![(tree.a.as_uuid(), vec![reader.id])])
            .await
            .unwrap();
        let owner = services.roles.principal(tenant, ann).await.unwrap();
        let new_key = |org: Option<OrgId>| NewApiKey {
            name: "agent".to_owned(),
            permissions: vec!["notes.read".to_owned()],
            org_id: org.map(|org| org.as_uuid()),
            expires_at: None,
            current_password: PASSWORD.to_owned(),
            second_factor: None,
        };
        let error =
            services.api_keys.create(tenant, &owner, new_key(Some(tree.b))).await.unwrap_err();
        assert_eq!(codes(&error), ["out_of_scope".to_owned()].into_iter().collect());
        let error =
            services.api_keys.create(tenant, &owner, new_key(Some(org()))).await.unwrap_err();
        assert_eq!(codes(&error), ["unknown".to_owned()].into_iter().collect());

        let created =
            services.api_keys.create(tenant, &owner, new_key(Some(tree.a1))).await.unwrap();
        assert_eq!(created.key.org_id, tree.a1.as_uuid());
        let principal = services.api_keys.authenticate(tenant, &created.secret).await.unwrap();
        assert_eq!(principal.fence(), Some(tree.a1));
        assert_eq!(principal.grants(&NOTES_READ), [Grant::new(tree.a, Scope::Subtree)]);
        let read = services.database.read();
        let at = |org| autoxide_kernel::access::OrgTarget::new(org, None);
        assert!(principal.require_at(read, &NOTES_READ, at(tree.a1)).await.is_ok());
        assert_eq!(
            principal.require_at(read, &NOTES_READ, at(tree.a)).await.unwrap_err().code(),
            "org_out_of_scope",
            "ngoài hàng rào"
        );
        assert!(!principal.allows("auth.users.read"), "key không có hơn permission của nó");

        // Key không nêu nút: gắn gốc, không rào.
        let whole = services.api_keys.create(tenant, &owner, new_key(None)).await.unwrap();
        assert_eq!(whole.key.org_id, root_of(DEFAULT_TENANT).as_uuid());
        let principal = services.api_keys.authenticate(tenant, &whole.secret).await.unwrap();
        assert_eq!(principal.fence(), None);
    })
    .await
    .unwrap();
}

/// Số người ở mỗi nút: `direct` ở chính nút, `total` gồm người ở nút trên; mỗi người một lần dù
/// nhiều role; nút không ai cũng có dòng; gốc đứng đầu.
#[tokio::test]
async fn dem_so_nguoi_moi_nut() {
    TestDatabase::run(async |test| {
        let services = services(test).await;
        let tenant = &services.tenant;
        let tree = tree(&services).await;
        let (ann, bob) =
            (user(&services, "ann@example.com").await, user(&services, "bob@example.com").await);
        let reader = role(&services, "reader", &[], &[("notes.read", Scope::Subtree)]).await;
        let viewer = role(&services, "viewer", &["auth.users.read"], &[]).await;
        let admin = Principal::with_all_permissions(Uuid::now_v7());
        // Ann: hai role ở A, một ở gốc. Bob: một role ở A1.
        let wanted = vec![
            (tree.a.as_uuid(), vec![reader.id, viewer.id]),
            (tree.root.as_uuid(), vec![viewer.id]),
        ];
        services.roles.set_memberships(tenant, &admin, ann, wanted).await.unwrap();
        services
            .roles
            .set_memberships(tenant, &admin, bob, vec![(tree.a1.as_uuid(), vec![reader.id])])
            .await
            .unwrap();

        let counts = services.roles.member_counts(tenant).await.unwrap();
        assert_eq!(counts.len(), 4, "{counts:?}");
        assert_eq!(counts[0].org_id, tree.root.as_uuid(), "gốc đứng đầu");
        let of = |org: OrgId| {
            let found = counts.iter().find(|count| count.org_id == org.as_uuid()).unwrap();
            (found.direct, found.total)
        };
        assert_eq!(of(tree.root), (1, 1));
        assert_eq!(of(tree.a), (1, 1), "Ann ở A và ở gốc: một người");
        assert_eq!(of(tree.a1), (1, 2), "Bob trực tiếp, Ann từ trên");
        assert_eq!(of(tree.b), (0, 1), "chỉ Ann từ gốc");
    })
    .await
    .unwrap();
}

/// Số người giữ mỗi role: mỗi người một lần dù giữ ở nhiều nút; role không ai giữ là 0.
#[tokio::test]
async fn dem_so_nguoi_giu_moi_role() {
    TestDatabase::run(async |test| {
        let services = services(test).await;
        let tenant = &services.tenant;
        let tree = tree(&services).await;
        let ann = user(&services, "ann@example.com").await;
        let bob = user(&services, "bob@example.com").await;
        let reader = role(&services, "reader", &[], &[("notes.read", Scope::Subtree)]).await;
        let unused = role(&services, "unused", &["auth.users.read"], &[]).await;
        let admin = Principal::with_all_permissions(Uuid::now_v7());
        let wanted = vec![(tree.a.as_uuid(), vec![reader.id]), (tree.b.as_uuid(), vec![reader.id])];
        services.roles.set_memberships(tenant, &admin, ann, wanted).await.unwrap();
        services
            .roles
            .set_memberships(tenant, &admin, bob, vec![(tree.a1.as_uuid(), vec![reader.id])])
            .await
            .unwrap();
        let counts = services.roles.role_member_counts(tenant).await.unwrap();
        let of = |id: Uuid| counts.iter().find(|(role, _)| *role == id).map(|(_, n)| *n);
        assert_eq!(of(reader.id), Some(2), "Ann ở hai nút vẫn là một người");
        assert_eq!(of(unused.id), Some(0));
    })
    .await
    .unwrap();
}
