//! P16 (D-20261128-ban-lift-scope): luật "chỉ tác động lên cái mình có" so cả PHẠM VI, không chỉ tên permission —
//! bỏ lệnh cấm (gỡ role có cấm, sửa role bỏ cấm) đòi permission bị cấm ở độ sâu `tenant`; tác động lên tài khoản
//! người khác (`require_covers`) đòi bao được từng grant (nút + độ sâu) của họ. Cả SQLite và Postgres.

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

/// Người làm có `grants` (permission → nút, độ sâu) trong khách `tenant`, như principal auth dựng từ membership.
fn actor(tenant: &TenantContext, grants: &[(&str, OrgId, Scope)]) -> Principal {
    let grants = grants
        .iter()
        .map(|(permission, org, scope)| ((*permission).to_owned(), Grant::new(*org, *scope)));
    Principal::with_grants(Uuid::now_v7(), grants).in_tenant(tenant.id())
}

#[tokio::test]
async fn bo_lenh_cam_doi_permission_bi_cam_o_ca_khach() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let (roles, tenant) = (services.roles.clone(), services.tenant.clone());
        let (root, branch) = (root_of(tenant.id()), OrgId::from_uuid(Uuid::now_v7()));
        let mut restricted = input("restricted", &["auth.users.read"]);
        restricted.denies = vec!["auth.users.manage".to_owned()];
        let restricted = roles.create(&tenant, Actor::system(), restricted).await.unwrap();
        let bob = user(&services, "bob@example.com").await;
        roles.set_user_roles(&tenant, Actor::system(), bob, vec![restricted.id]).await.unwrap();

        // Có `auth.users.manage` nhưng chỉ ở một nhánh: trước đây (so theo tên) gỡ được cấm của cả khách.
        let narrow = actor(
            &tenant,
            &[
                ("auth.roles.manage", root, Scope::Tenant),
                ("auth.users.read", root, Scope::Tenant),
                ("auth.users.manage", branch, Scope::Unit),
            ],
        );
        let error = roles.set_user_roles_as(&tenant, &narrow, bob, vec![]).await.unwrap_err();
        assert_eq!(error.code(), "auth.role_beyond_you");
        let error = roles.delete_as(&tenant, &narrow, restricted.id).await.unwrap_err();
        assert_eq!(error.code(), "auth.role_beyond_you");
        let lifted = input("restricted", &["auth.users.read"]);
        let error = roles
            .update_as(&tenant, &narrow, restricted.id, lifted.clone(), None)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "auth.permission_beyond_you");
        let held: Vec<Uuid> =
            roles.user_roles(&tenant, bob).await.unwrap().iter().map(|role| role.id).collect();
        assert_eq!(held, vec![restricted.id], "không đổi gì");

        // API key gắn nhánh: có tên ở độ sâu `tenant` nhưng bị rào — cũng không bỏ được cấm.
        let fenced = actor(
            &tenant,
            &[
                ("auth.roles.manage", root, Scope::Tenant),
                ("auth.users.read", root, Scope::Tenant),
                ("auth.users.manage", root, Scope::Tenant),
            ],
        )
        .with_fence(branch);
        let error = roles.update_as(&tenant, &fenced, restricted.id, lifted.clone(), None).await;
        assert_eq!(error.unwrap_err().code(), "auth.permission_beyond_you");

        // Có ở độ sâu `tenant`, không rào: được.
        let wide = actor(
            &tenant,
            &[
                ("auth.roles.manage", root, Scope::Tenant),
                ("auth.users.read", root, Scope::Tenant),
                ("auth.users.manage", root, Scope::Tenant),
            ],
        );
        roles.update_as(&tenant, &wide, restricted.id, lifted, None).await.unwrap();
        roles.set_user_roles_as(&tenant, &wide, bob, vec![]).await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tac_dong_tai_khoan_nguoi_khac_doi_bao_ca_pham_vi() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let (roles, tenant) = (services.roles.clone(), services.tenant.clone());
        let (root, branch) = (root_of(tenant.id()), OrgId::from_uuid(Uuid::now_v7()));
        let reader = roles
            .create(&tenant, Actor::system(), input("reader", &["auth.users.read"]))
            .await
            .unwrap();
        let bob = user(&services, "bob@example.com").await;
        roles.set_user_roles(&tenant, Actor::system(), bob, vec![reader.id]).await.unwrap();

        // Bob đọc người dùng ở cả khách; người làm chỉ có ở một nhánh: trước đây (so tên) vẫn qua.
        let narrow = actor(
            &tenant,
            &[("auth.users.manage", root, Scope::Tenant), ("auth.users.read", branch, Scope::Unit)],
        );
        let error = roles.require_covers(&tenant, &narrow, bob).await.unwrap_err();
        assert_eq!(error.code(), "auth.target_has_more_permissions");
        let wide = actor(
            &tenant,
            &[("auth.users.manage", root, Scope::Tenant), ("auth.users.read", root, Scope::Tenant)],
        );
        roles.require_covers(&tenant, &wide, bob).await.unwrap();
        let error = roles.require_covers(&tenant, &wide.with_fence(branch), bob).await;
        assert_eq!(error.unwrap_err().code(), "auth.target_has_more_permissions");
    })
    .await
    .unwrap();
}
