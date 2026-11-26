//! Quyết định quyền theo đơn vị đòi đơn vị có trong cây của khách (PT-8, D-20261125-org-must-exist):
//! `Principal::check` với quyền toàn khách, `own` theo chủ bản ghi, nút gốc khi chưa dựng cây, nút của khách khác,
//! principal dựng ở khách khác; và kết quả "có" được nhớ trong một request (không tra lại mỗi lần). Chạy trên
//! cả SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::access::{Decision, Grant, OrgTarget, Permission, Principal, Scope};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, TenantId, root_of};
use autoxide_testkit::{TestApp, TestDatabase};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query};
use uuid::Uuid;

const READ: Permission = Permission::new("demo.read", "Read records.").scoped();

fn org() -> OrgId {
    OrgId::from_uuid(Uuid::now_v7())
}

#[tokio::test]
async fn don_vi_phai_co_trong_cay_cua_khach() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).build().await.unwrap();
        let database = app.database().clone();
        let read = database.read();
        let admin = Principal::with_all_permissions(Uuid::now_v7());
        let at = |org: OrgId| OrgTarget::new(org, None);

        // Chưa dựng cây: nút gốc của khách vẫn là đơn vị hợp lệ; ID bịa thì không.
        let root = root_of(DEFAULT_TENANT);
        assert_eq!(admin.check(read, &READ, at(root)).await.unwrap(), Decision::Allow);
        assert_eq!(admin.check(read, &READ, at(org())).await.unwrap(), Decision::OutOfScope);
        let error = admin.require_at(read, &READ, at(org())).await.unwrap_err();
        assert_eq!(error.code(), "org_out_of_scope");

        // Dựng cây ở khách mặc định và ở một khách khác (cùng database).
        let other = TenantId::from_uuid(Uuid::now_v7());
        let (mine, theirs) = (org(), org());
        let write = database.begin_write().await.unwrap();
        OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, mine, root, 8).await.unwrap();
        OrgTree::insert_root(&write, other).await.unwrap();
        OrgTree::insert(&write, other, theirs, root_of(other), 8).await.unwrap();
        write.commit().await.unwrap();

        assert_eq!(admin.check(read, &READ, at(mine)).await.unwrap(), Decision::Allow);
        // Nút của khách khác, và nút gốc của khách khác: ngoài phạm vi.
        assert_eq!(admin.check(read, &READ, at(theirs)).await.unwrap(), Decision::OutOfScope);
        assert_eq!(
            admin.check(read, &READ, at(root_of(other))).await.unwrap(),
            Decision::OutOfScope
        );
        // Principal dựng ở khách mặc định dùng cho khách kia: ngoài phạm vi; gắn đúng khách thì qua.
        let there = admin.clone().in_tenant(other);
        assert_eq!(there.check(read, &READ, at(theirs)).await.unwrap(), Decision::Allow);
        assert_eq!(there.check(read, &READ, at(mine)).await.unwrap(), Decision::OutOfScope);

        // Grant `tenant` (không phải admin) cũng vậy.
        let user = Uuid::now_v7();
        let tenant_wide = Principal::with_grants(
            user,
            [(READ.name().to_owned(), Grant::new(root, Scope::Tenant))],
        );
        assert_eq!(tenant_wide.check(read, &READ, at(mine)).await.unwrap(), Decision::Allow);
        assert_eq!(tenant_wide.check(read, &READ, at(org())).await.unwrap(), Decision::OutOfScope);

        // `own` theo chủ bản ghi: bản ghi của mình ở nút có thì qua, ở nút bịa / nút khách khác thì không.
        let own =
            Principal::with_grants(user, [(READ.name().to_owned(), Grant::new(mine, Scope::Own))]);
        let owned = |org: OrgId| OrgTarget::new(org, Some(user));
        assert_eq!(own.check(read, &READ, owned(root)).await.unwrap(), Decision::Allow);
        assert_eq!(own.check(read, &READ, owned(org())).await.unwrap(), Decision::OutOfScope);
        assert_eq!(own.check(read, &READ, owned(theirs)).await.unwrap(), Decision::OutOfScope);
    })
    .await
    .unwrap();
}

/// Một request kiểm nhiều permission ở cùng một đơn vị (điều hướng, việc cần làm, tìm, tổng quan): đơn vị được tra
/// trong cây một lần rồi nhớ trong principal (các bản clone dùng chung). Chứng minh bằng cách xoá dòng closure
/// sau lần kiểm đầu: principal đã nhớ vẫn qua, principal mới thì không.
#[tokio::test]
async fn don_vi_da_tra_duoc_nho_trong_request() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).build().await.unwrap();
        let database = app.database().clone();
        let unit = org();
        let write = database.begin_write().await.unwrap();
        let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, unit, root, 8).await.unwrap();
        write.commit().await.unwrap();

        let admin = Principal::with_all_permissions(Uuid::now_v7());
        let same_request = admin.clone();
        let at = OrgTarget::new(unit, None);
        assert_eq!(admin.check(database.read(), &READ, at).await.unwrap(), Decision::Allow);

        let delete = Query::delete()
            .from_table(Alias::new("kernel_org_closure"))
            .and_where(Expr::col(Alias::new("descendant_id")).eq(unit.as_uuid()))
            .to_owned();
        database.write().execute(&delete).await.unwrap();

        for permission in [&READ, &Permission::new("demo.other", "Other.").scoped()] {
            assert_eq!(
                same_request.check(database.read(), permission, at).await.unwrap(),
                Decision::Allow,
                "đã nhớ: không tra lại"
            );
        }
        let fresh = Principal::with_all_permissions(Uuid::now_v7());
        assert_eq!(
            fresh.check(database.read(), &READ, at).await.unwrap(),
            Decision::OutOfScope,
            "principal mới tra lại cây"
        );
    })
    .await
    .unwrap();
}
