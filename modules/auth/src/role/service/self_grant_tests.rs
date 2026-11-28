//! PT-9 (D-20261126-no-self-grant, chủ dự án quyết): người có `auth.roles.manage` chỉ cấp, gỡ, hay thêm vào role thứ
//! mình đang có — kể cả cho chính mình; quản trị viên (role `admin`) vẫn làm được mọi việc. Cả SQLite và Postgres.

use super::*;
use autoxide_testkit::TestDatabase;

use crate::testing::Services;
use crate::user::NewUser;

fn input(name: &str, permissions: &[&str]) -> RoleInput {
    RoleInput {
        denies: Vec::new(),
        requires_mfa: false,
        grants: Vec::new(),
        name: name.to_owned(),
        description: String::new(),
        permissions: permissions.iter().map(|p| (*p).to_owned()).collect(),
    }
}

async fn user(services: &Services, email: &str) -> Uuid {
    let input = NewUser { email: email.to_owned(), name: "U".to_owned(), password: None };
    services.users.create(&services.tenant, Actor::system(), input).await.unwrap().id
}

async fn principal_of(services: &Services, user_id: Uuid) -> Principal {
    services.roles.user_principal(&services.tenant, user_id).await.unwrap().unwrap().1
}

#[tokio::test]
async fn nguoi_quan_ly_role_khong_tu_cap_quyen_minh_chua_co() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let (roles, tenant) = (services.roles.clone(), services.tenant.clone());
        let first = PageRequest::first(200, autoxide_kernel::pagination::Order::Asc);
        let page = roles.list(&tenant, &first).await.unwrap();
        let admin = page.data.into_iter().find(|role| role.all_permissions).unwrap();
        let manager = roles
            .create(
                &tenant,
                Actor::system(),
                input("manager", &["auth.roles.manage", "auth.users.read"]),
            )
            .await
            .unwrap();
        let reader = roles
            .create(&tenant, Actor::system(), input("reader", &["auth.users.read"]))
            .await
            .unwrap();
        let deleter = roles
            .create(
                &tenant,
                Actor::system(),
                input("deleter", &["auth.users.read", "auth.users.manage"]),
            )
            .await
            .unwrap();
        let (ann, bob, boss) = (
            user(&services, "ann@example.com").await,
            user(&services, "bob@example.com").await,
            user(&services, "boss@example.com").await,
        );
        roles.set_user_roles(&tenant, Actor::system(), ann, vec![manager.id]).await.unwrap();
        roles.set_user_roles(&tenant, Actor::system(), boss, vec![admin.id]).await.unwrap();
        let ann_p = principal_of(&services, ann).await;

        // Tự gán `admin` hay role có permission mình chưa có: 403, không đổi gì.
        for extra in [admin.id, deleter.id] {
            let error = roles
                .set_user_roles_as(&tenant, &ann_p, ann, vec![manager.id, extra])
                .await
                .unwrap_err();
            assert_eq!(error.code(), "auth.role_beyond_you", "{error:?}");
        }
        let held: Vec<Uuid> =
            roles.user_roles(&tenant, ann).await.unwrap().iter().map(|role| role.id).collect();
        assert_eq!(held, vec![manager.id]);
        // Cho người khác cũng theo cùng luật; role mình có đủ quyền thì cấp được.
        let error =
            roles.set_user_roles_as(&tenant, &ann_p, bob, vec![deleter.id]).await.unwrap_err();
        assert_eq!(error.code(), "auth.role_beyond_you");
        roles.set_user_roles_as(&tenant, &ann_p, bob, vec![reader.id]).await.unwrap();
        // Gỡ role mình không cấp được (của người khác) cũng không được.
        roles.set_user_roles(&tenant, Actor::system(), bob, vec![deleter.id]).await.unwrap();
        let error = roles.set_user_roles_as(&tenant, &ann_p, bob, vec![]).await.unwrap_err();
        assert_eq!(error.code(), "auth.role_beyond_you");

        // Sửa role mình đang giữ: thêm permission mình chưa có, nới phạm vi, hay bỏ cấm → 403.
        let wider =
            input("manager", &["auth.roles.manage", "auth.users.read", "auth.users.manage"]);
        let error = roles.update_as(&tenant, &ann_p, manager.id, wider, None).await.unwrap_err();
        assert_eq!(error.code(), "auth.permission_beyond_you");
        let mut banned = input("reader", &["auth.users.read"]);
        banned.denies = vec!["auth.users.manage".to_owned()];
        roles.update(&tenant, Actor::system(), reader.id, banned, None).await.unwrap();
        let error = roles
            .update_as(&tenant, &ann_p, reader.id, input("reader", &["auth.users.read"]), None)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "auth.permission_beyond_you", "bỏ cấm là mở thêm quyền");
        // Bớt permission, đổi tên: được.
        roles
            .update_as(&tenant, &ann_p, manager.id, input("managers", &["auth.roles.manage"]), None)
            .await
            .unwrap();

        // Quản trị viên vẫn làm được mọi việc (không khoá khách khỏi chính họ).
        let boss_p = principal_of(&services, boss).await;
        roles.set_user_roles_as(&tenant, &boss_p, ann, vec![admin.id]).await.unwrap();
        let wider = input("reader", &["auth.users.read", "auth.users.manage"]);
        roles.update_as(&tenant, &boss_p, reader.id, wider, None).await.unwrap();
        // Admin cuối cùng vẫn được bảo vệ như trước.
        roles.set_user_roles_as(&tenant, &boss_p, ann, vec![]).await.unwrap();
        let error = roles.set_user_roles_as(&tenant, &boss_p, boss, vec![]).await.unwrap_err();
        assert_eq!(error.code(), "auth.last_admin");
    })
    .await
    .unwrap();
}

/// Gỡ một role là bỏ cả permission CẤM của nó (cấm thắng mọi role khác): người gỡ phải có mọi permission bị cấm —
/// một luật cho ba đường: thay role của người dùng, thay membership, xoá role. Nới `requires_mfa` của role cũng
/// đòi trao được role đó. Quản trị viên làm được cả.
#[tokio::test]
async fn go_role_co_lenh_cam_doi_co_permission_bi_cam() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let (roles, tenant) = (services.roles.clone(), services.tenant.clone());
        let first = PageRequest::first(200, autoxide_kernel::pagination::Order::Asc);
        let page = roles.list(&tenant, &first).await.unwrap();
        let admin = page.data.into_iter().find(|role| role.all_permissions).unwrap();
        let manager = roles
            .create(
                &tenant,
                Actor::system(),
                input("manager", &["auth.roles.manage", "auth.users.read"]),
            )
            .await
            .unwrap();
        let mut restricted = input("restricted", &["auth.users.read"]);
        restricted.denies = vec!["auth.users.manage".to_owned()];
        let restricted = roles.create(&tenant, Actor::system(), restricted).await.unwrap();
        let (ann, bob, boss) = (
            user(&services, "ann@example.com").await,
            user(&services, "bob@example.com").await,
            user(&services, "boss@example.com").await,
        );
        roles.set_user_roles(&tenant, Actor::system(), ann, vec![manager.id]).await.unwrap();
        roles.set_user_roles(&tenant, Actor::system(), bob, vec![restricted.id]).await.unwrap();
        roles.set_user_roles(&tenant, Actor::system(), boss, vec![admin.id]).await.unwrap();
        let ann_p = principal_of(&services, ann).await;
        // Dựng trước khi role `admin` đòi MFA (boss chưa bật MFA).
        let boss_p = principal_of(&services, boss).await;
        let root = root_of(tenant.id()).as_uuid();

        // (a) thay role, (b) thay membership, (c) xoá role: đều 403, không đổi gì.
        let error = roles.set_user_roles_as(&tenant, &ann_p, bob, vec![]).await.unwrap_err();
        assert_eq!(error.code(), "auth.role_beyond_you");
        let error = roles.set_memberships(&tenant, &ann_p, bob, vec![]).await.unwrap_err();
        assert_eq!(error.code(), "auth.role_beyond_you");
        let error = roles.delete_as(&tenant, &ann_p, restricted.id).await.unwrap_err();
        assert_eq!(error.code(), "auth.role_beyond_you");
        let held: Vec<Uuid> =
            roles.user_roles(&tenant, bob).await.unwrap().iter().map(|role| role.id).collect();
        assert_eq!(held, vec![restricted.id]);

        // (e) Nới `requires_mfa` của role người sửa không trao được: 403; siết thì được.
        let mut strict = input("restricted", &["auth.users.read"]);
        strict.denies = vec!["auth.users.manage".to_owned()];
        strict.requires_mfa = true;
        roles.update_as(&tenant, &ann_p, restricted.id, strict.clone(), None).await.unwrap();
        let mut admin_mfa = input("admin", &[]);
        admin_mfa.description = admin.description.clone();
        admin_mfa.requires_mfa = true;
        roles.update(&tenant, Actor::system(), admin.id, admin_mfa.clone(), None).await.unwrap();
        admin_mfa.requires_mfa = false;
        let error =
            roles.update_as(&tenant, &ann_p, admin.id, admin_mfa.clone(), None).await.unwrap_err();
        assert_eq!(error.code(), "auth.role_beyond_you", "nới MFA của role `admin`");

        // Quản trị viên làm được cả ba.
        roles.update_as(&tenant, &boss_p, admin.id, admin_mfa, None).await.unwrap();
        roles
            .set_memberships(&tenant, &boss_p, bob, vec![(root, vec![restricted.id])])
            .await
            .unwrap();
        roles.set_user_roles_as(&tenant, &boss_p, bob, vec![]).await.unwrap();
        roles.delete_as(&tenant, &boss_p, restricted.id).await.unwrap();
    })
    .await
    .unwrap();
}

/// Role chỉ có permission cấm (không cấp gì): gỡ nó vẫn đòi có mọi permission bị cấm.
#[tokio::test]
async fn role_chi_co_lenh_cam() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let (roles, tenant) = (services.roles.clone(), services.tenant.clone());
        let manager = roles
            .create(
                &tenant,
                Actor::system(),
                input("manager", &["auth.roles.manage", "auth.users.read"]),
            )
            .await
            .unwrap();
        let mut ban = input("ban", &[]);
        ban.denies = vec!["auth.users.manage".to_owned()];
        let ban = roles.create(&tenant, Actor::system(), ban).await.unwrap();
        let (ann, bob) =
            (user(&services, "ann@example.com").await, user(&services, "bob@example.com").await);
        roles.set_user_roles(&tenant, Actor::system(), ann, vec![manager.id]).await.unwrap();
        roles.set_user_roles(&tenant, Actor::system(), bob, vec![ban.id]).await.unwrap();
        let ann_p = principal_of(&services, ann).await;
        let error = roles.set_user_roles_as(&tenant, &ann_p, bob, vec![]).await.unwrap_err();
        assert_eq!(error.code(), "auth.role_beyond_you");
        // Cấp role chỉ có lệnh cấm thì được (thu hẹp quyền người nhận).
        roles.set_user_roles_as(&tenant, &ann_p, ann, vec![manager.id, ban.id]).await.unwrap();
    })
    .await
    .unwrap();
}

/// P16 (iv) (D-20261128-role-narrow-rule): sửa role mà BỎ, THU HẸP hay CẤM thêm một permission là gỡ nó khỏi mọi người
/// giữ role — người sửa phải có permission đó đủ rộng, như khi gỡ role. Permission mình có thì bỏ được.
#[tokio::test]
async fn thu_hep_role_nguoi_khac_doi_co_permission_do() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let (roles, tenant) = (services.roles.clone(), services.tenant.clone());
        let manager = roles
            .create(
                &tenant,
                Actor::system(),
                input("manager", &["auth.roles.manage", "auth.users.read"]),
            )
            .await
            .unwrap();
        let deleter = roles
            .create(
                &tenant,
                Actor::system(),
                input("deleter", &["auth.users.read", "auth.users.manage"]),
            )
            .await
            .unwrap();
        let ann = user(&services, "ann@example.com").await;
        roles.set_user_roles(&tenant, Actor::system(), ann, vec![manager.id]).await.unwrap();
        let ann_p = principal_of(&services, ann).await;

        // Bỏ `auth.users.manage` (ann không có) khỏi role người khác giữ: 403, role giữ nguyên.
        let error = roles
            .update_as(&tenant, &ann_p, deleter.id, input("deleter", &["auth.users.read"]), None)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "auth.permission_beyond_you", "{error:?}");
        // Cấm thêm `auth.scim.manage` (ann không có): cũng là gỡ nó ở cả khách khỏi người giữ role.
        let mut banned = input("deleter", &["auth.users.read", "auth.users.manage"]);
        banned.denies = vec!["auth.scim.manage".to_owned()];
        let error = roles.update_as(&tenant, &ann_p, deleter.id, banned, None).await.unwrap_err();
        assert_eq!(error.code(), "auth.permission_beyond_you");
        let kept = roles.get(&tenant, deleter.id).await.unwrap();
        assert!(kept.permissions.contains(&"auth.users.manage".to_owned()), "{kept:?}");
        assert!(kept.denies.is_empty(), "{kept:?}");
        // Bỏ permission mình có (`auth.users.read`) thì được.
        roles
            .update_as(&tenant, &ann_p, deleter.id, input("deleter", &["auth.users.manage"]), None)
            .await
            .unwrap();
    })
    .await
    .unwrap();
}

/// Khách có guardrail (D-20261128-ban-lift-guardrail): role cấm một permission nằm ngoài rào — không ai trong khách có
/// được permission đó, kể cả quản trị viên. Bỏ cấm ấy không mở thêm gì, nên quản trị viên trong rào gỡ được role (trước
/// đây không ai gỡ được). Permission trong rào mà người gỡ không có thì vẫn bị chặn.
#[tokio::test]
async fn go_role_cam_permission_ngoai_rao() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let (roles, tenant) = (services.roles.clone(), services.tenant.clone());
        let catalog = roles.catalog().clone();
        let first = PageRequest::first(200, autoxide_kernel::pagination::Order::Asc);
        let admin = roles.list(&tenant, &first).await.unwrap().data.into_iter();
        let admin = admin.into_iter().find(|role| role.all_permissions).unwrap();
        let mut banner = input("banner", &["auth.users.read"]);
        banner.denies = vec!["auth.users.manage".to_owned()];
        let banner = roles.create(&tenant, Actor::system(), banner).await.unwrap();
        let (boss, bob) =
            (user(&services, "boss@example.com").await, user(&services, "bob@example.com").await);
        roles.set_user_roles(&tenant, Actor::system(), boss, vec![admin.id]).await.unwrap();
        roles.set_user_roles(&tenant, Actor::system(), bob, vec![banner.id]).await.unwrap();
        let everything: std::collections::BTreeSet<String> =
            catalog.iter().map(|permission| permission.name().to_owned()).collect();

        // Rào bỏ `auth.users.manage`: quản trị viên không có nó, nhưng gỡ được role cấm nó.
        let mut fenced = everything.clone();
        fenced.remove("auth.users.manage");
        let boss_p = principal_of(&services, boss).await.within(Some(&fenced), &catalog);
        assert!(boss_p.outside_guardrail("auth.users.manage"));
        roles.set_user_roles_as(&tenant, &boss_p, bob, vec![]).await.unwrap();

        // Rào bỏ permission khác: `auth.users.manage` trong rào mà người gỡ không có (role cấm của chính họ) → 403.
        roles.set_user_roles(&tenant, Actor::system(), bob, vec![banner.id]).await.unwrap();
        let boss_p = principal_of(&services, boss)
            .await
            .without(["auth.users.manage".to_owned()], &catalog)
            .within(Some(&everything), &catalog);
        let error = roles.set_user_roles_as(&tenant, &boss_p, bob, vec![]).await.unwrap_err();
        assert_eq!(error.code(), "auth.role_beyond_you", "{error:?}");
    })
    .await
    .unwrap();
}
