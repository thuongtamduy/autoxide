//! Cây tổ chức và quyền theo độ sâu (bước 34 mục 1, đặc tả "Test bắt buộc" phần kernel): bảng tổ
//! tiên – con cháu luôn khớp với cây, không tạo được vòng, độ sâu có giới hạn, khoá một nút thì cả
//! nhánh ngừng ghi, và `Principal` lọc đúng theo `own` / `unit` / `subtree` / `tenant`. Chạy trên
//! cả SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use autoxide_kernel::access::{Grant, OrgTarget, Permission, Principal, Scope};
use autoxide_kernel::database::Database;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, nearest, root_of};
use autoxide_testkit::{TestApp, TestDatabase};
use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{ColumnDef, Table};
use sea_orm::{ActiveValue::Set, QueryOrder};

const MAX_DEPTH: u8 = 8;
const READ: Permission = Permission::new("demo.read", "Read records.").scoped();
const SETTINGS: Permission = Permission::new("demo.settings", "Change settings of the tenant.");

mod record {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "demo_record")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub org_id: Uuid,
        pub owner_id: Uuid,
        pub name: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// Cây mẫu: gốc → A, B (anh em) → A1 dưới A.
struct Tree {
    database: Database,
    root: OrgId,
    a: OrgId,
    b: OrgId,
    a1: OrgId,
}

fn org() -> OrgId {
    OrgId::from_uuid(Uuid::now_v7())
}

async fn add(
    database: &Database,
    id: OrgId,
    parent: OrgId,
) -> Result<(), autoxide_kernel::error::Error> {
    let transaction = database.begin_write().await?;
    OrgTree::insert(&transaction, DEFAULT_TENANT, id, parent, MAX_DEPTH).await?;
    transaction.commit().await?;
    Ok(())
}

async fn relocate(
    database: &Database,
    id: OrgId,
    parent: OrgId,
) -> Result<(), autoxide_kernel::error::Error> {
    let transaction = database.begin_write().await?;
    OrgTree::move_subtree(&transaction, DEFAULT_TENANT, id, parent, MAX_DEPTH).await?;
    transaction.commit().await?;
    Ok(())
}

async fn tree(test: &TestDatabase) -> (TestApp, Tree) {
    let app = TestApp::builder(test).build().await.unwrap();
    let database = app.database().clone();
    let transaction = database.begin_write().await.unwrap();
    let root = OrgTree::insert_root(&transaction, DEFAULT_TENANT).await.unwrap();
    assert_eq!(
        OrgTree::insert_root(&transaction, DEFAULT_TENANT).await.unwrap(),
        root,
        "chạy lại được"
    );
    transaction.commit().await.unwrap();
    assert_eq!(root, root_of(DEFAULT_TENANT));
    let (a, b, a1) = (org(), org(), org());
    add(&database, a, root).await.unwrap();
    add(&database, b, root).await.unwrap();
    add(&database, a1, a).await.unwrap();
    (app, Tree { database, root, a, b, a1 })
}

async fn create_records(database: &Database) {
    let backend = database.write().get_database_backend();
    let table = Table::create()
        .table("demo_record")
        .col(ColumnDef::new("id").uuid().not_null().primary_key())
        .col(ColumnDef::new("tenant_id").uuid().not_null())
        .col(ColumnDef::new("org_id").uuid().not_null())
        .col(ColumnDef::new("owner_id").uuid().not_null())
        .col(ColumnDef::new("name").string().not_null())
        .to_owned();
    database.write().execute_raw(backend.build(&table)).await.unwrap();
}

async fn insert_record<C: ConnectionTrait>(connection: &C, org: OrgId, owner: Uuid, name: &str) {
    record::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(DEFAULT_TENANT.as_uuid()),
        org_id: Set(org.as_uuid()),
        owner_id: Set(owner),
        name: Set(name.to_owned()),
    }
    .insert(connection)
    .await
    .unwrap();
}

async fn visible(
    database: &Database,
    principal: &Principal,
    permission: &Permission,
) -> BTreeSet<String> {
    let condition = principal
        .scope_condition(permission, record::Column::OrgId, Some(record::Column::OwnerId))
        .unwrap();
    record::Entity::find()
        .filter(record::Column::TenantId.eq(DEFAULT_TENANT.as_uuid()))
        .filter(condition)
        .order_by_asc(record::Column::Name)
        .all(database.read())
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.name)
        .collect()
}

fn names(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

fn member(user: Uuid, permission: &Permission, org: OrgId, scope: Scope) -> Principal {
    Principal::with_grants(user, [(permission.name().to_owned(), Grant::new(org, scope))])
}

#[test]
fn nearest_lay_to_tien_gan_nhat() {
    let (root, a, a1, b) = (org(), org(), org(), org());
    let lineage = [a1, a, root];
    assert_eq!(
        nearest(&lineage, [(root, "UTC"), (a, "Asia/Ho_Chi_Minh")]),
        Some((a, "Asia/Ho_Chi_Minh"))
    );
    assert_eq!(nearest(&lineage, [(a1, 1), (root, 2), (a, 3)]), Some((a1, 1)), "chính nút trước");
    assert_eq!(nearest(&lineage, [(b, 1)]), None, "nút ngoài chuỗi không tính");
    assert_eq!(nearest(&lineage, Vec::<(OrgId, u8)>::new()), None);
}

#[tokio::test]
async fn do_sau_quyet_dinh_dieu_kien_loc() {
    TestDatabase::run(async |test| {
        let (_app, tree) = tree(test).await;
        create_records(&tree.database).await;
        let (ann, bob, cat) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let write = tree.database.write();
        insert_record(write, tree.root, ann, "root-ann").await;
        insert_record(write, tree.a, bob, "a-bob").await;
        insert_record(write, tree.a1, ann, "a1-ann").await;
        insert_record(write, tree.b, cat, "b-cat").await;
        let database = &tree.database;

        // Từng ô của bảng nghĩa (membership của Ann ở A).
        let own = member(ann, &READ, tree.a, Scope::Own);
        assert_eq!(
            visible(database, &own, &READ).await,
            names(&["a1-ann", "root-ann"]),
            "own: bản ghi của mình ở mọi nút"
        );
        let unit = member(ann, &READ, tree.a, Scope::Unit);
        assert_eq!(visible(database, &unit, &READ).await, names(&["a-bob"]), "unit: đúng nút A");
        let subtree = member(ann, &READ, tree.a, Scope::Subtree);
        assert_eq!(
            visible(database, &subtree, &READ).await,
            names(&["a-bob", "a1-ann"]),
            "subtree: A và A1"
        );
        let tenant = member(ann, &READ, tree.a, Scope::Tenant);
        assert_eq!(
            visible(database, &tenant, &READ).await,
            names(&["a-bob", "a1-ann", "b-cat", "root-ann"])
        );
        let admin = Principal::with_all_permissions(ann);
        assert_eq!(visible(database, &admin, &READ).await.len(), 4);

        // Hợp nhiều grant: unit ở B + subtree ở A.
        let both = Principal::with_grants(
            ann,
            [
                (READ.name().to_owned(), Grant::new(tree.b, Scope::Unit)),
                (READ.name().to_owned(), Grant::new(tree.a, Scope::Subtree)),
            ],
        );
        assert_eq!(visible(database, &both, &READ).await, names(&["a-bob", "a1-ann", "b-cat"]));

        // Không có grant: lỗi, không phải danh sách rỗng.
        let none = Principal::new(ann, []);
        let error = none.scope_condition(&READ, record::Column::OrgId, None).unwrap_err();
        assert_eq!(error.code(), "permission_denied");

        // Kiểm một bản ghi: cùng nghĩa.
        let read = database.read();
        let at = |org: OrgId, owner: Uuid| OrgTarget::new(org, Some(owner));
        assert!(own.require_at(read, &READ, at(tree.b, ann)).await.is_ok(), "own: của mình");
        assert_eq!(
            own.require_at(read, &READ, at(tree.a, bob)).await.unwrap_err().code(),
            "org_out_of_scope"
        );
        assert!(unit.require_at(read, &READ, at(tree.a, bob)).await.is_ok());
        assert_eq!(
            unit.require_at(read, &READ, at(tree.a1, bob)).await.unwrap_err().code(),
            "org_out_of_scope"
        );
        assert!(subtree.require_at(read, &READ, at(tree.a1, bob)).await.is_ok());
        assert_eq!(
            subtree.require_at(read, &READ, at(tree.b, cat)).await.unwrap_err().code(),
            "org_out_of_scope"
        );
        assert_eq!(
            subtree.require_at(read, &READ, at(tree.root, ann)).await.unwrap_err().code(),
            "org_out_of_scope",
            "không lên cha"
        );
        assert!(tenant.require_at(read, &READ, at(tree.b, cat)).await.is_ok());
        assert_eq!(
            none.require_at(read, &READ, at(tree.a, ann)).await.unwrap_err().code(),
            "permission_denied"
        );
        // Bản ghi không có người sở hữu: `own` hiểu như `unit`.
        assert!(own.require_at(read, &READ, OrgTarget::new(tree.a, None)).await.is_ok());
        assert!(own.require_at(read, &READ, OrgTarget::new(tree.b, None)).await.is_err());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hang_rao_api_key_cat_quyen() {
    TestDatabase::run(async |test| {
        let (_app, tree) = tree(test).await;
        create_records(&tree.database).await;
        let ann = Uuid::now_v7();
        for (org, name) in [(tree.root, "root"), (tree.a, "a"), (tree.a1, "a1"), (tree.b, "b")] {
            insert_record(tree.database.write(), org, ann, name).await;
        }
        let key = Principal::with_all_permissions(ann).with_fence(tree.a);
        assert_eq!(key.fence(), Some(tree.a));
        assert_eq!(
            visible(&tree.database, &key, &READ).await,
            names(&["a", "a1"]),
            "admin qua key gắn A chỉ thấy nhánh A"
        );
        let read = tree.database.read();
        assert!(key.require_at(read, &READ, OrgTarget::new(tree.a1, None)).await.is_ok());
        assert_eq!(
            key.require_at(read, &READ, OrgTarget::new(tree.b, None)).await.unwrap_err().code(),
            "org_out_of_scope"
        );
        // Key gắn gốc: không rào.
        let unfenced = Principal::with_all_permissions(ann).with_fence(tree.root);
        assert_eq!(unfenced.fence(), None);
        assert_eq!(visible(&tree.database, &unfenced, &READ).await.len(), 4);
        // Rào cắt cả grant hẹp: subtree ở gốc + rào ở B = chỉ B.
        let narrow = member(ann, &READ, tree.root, Scope::Subtree).with_fence(tree.b);
        assert_eq!(visible(&tree.database, &narrow, &READ).await, names(&["b"]));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn permission_khong_scoped_co_hieu_luc_toan_khach() {
    TestDatabase::run(async |test| {
        let (_app, tree) = tree(test).await;
        let ann = Uuid::now_v7();
        let principal = member(ann, &SETTINGS, tree.a1, Scope::Unit);
        assert_eq!(principal.grants(&SETTINGS), vec![Grant::new(tree.root, Scope::Tenant)]);
        let read = tree.database.read();
        assert!(principal.require_at(read, &SETTINGS, OrgTarget::new(tree.b, None)).await.is_ok());
        let condition = principal.scope_condition(&SETTINGS, record::Column::OrgId, None).unwrap();
        assert!(condition.is_empty(), "không giới hạn");
        // Permission scoped thì giữ đúng nút.
        let scoped = member(ann, &READ, tree.a1, Scope::Unit);
        assert_eq!(scoped.grants(&READ), vec![Grant::new(tree.a1, Scope::Unit)]);
        assert!(principal.require(&SETTINGS).is_ok() && scoped.require(&READ).is_ok());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nut_bi_khoa_thi_ca_nhanh_khong_ghi_duoc() {
    TestDatabase::run(async |test| {
        let (_app, tree) = tree(test).await;
        let admin = Principal::with_all_permissions(Uuid::now_v7());
        let transaction = tree.database.begin_write().await.unwrap();
        OrgTree::set_writable(&transaction, DEFAULT_TENANT, tree.a, false).await.unwrap();
        transaction.commit().await.unwrap();

        let transaction = tree.database.begin_write().await.unwrap();
        for frozen in [tree.a, tree.a1] {
            let error = admin
                .require_write_at(&transaction, &READ, OrgTarget::new(frozen, None))
                .await
                .unwrap_err();
            assert_eq!(error.code(), "org_frozen", "kể cả admin");
        }
        admin.require_write_at(&transaction, &READ, OrgTarget::new(tree.b, None)).await.unwrap();
        admin.require_write_at(&transaction, &READ, OrgTarget::new(tree.root, None)).await.unwrap();
        // Nút không có trong cây (khách chưa dựng cây cho bản ghi này) coi như ghi được.
        OrgTree::ensure_writable(&transaction, DEFAULT_TENANT, org()).await.unwrap();
        transaction.commit().await.unwrap();

        let transaction = tree.database.begin_write().await.unwrap();
        OrgTree::set_writable(&transaction, DEFAULT_TENANT, tree.a, true).await.unwrap();
        OrgTree::ensure_writable(&transaction, DEFAULT_TENANT, tree.a1).await.unwrap();
        transaction.commit().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khong_tao_duoc_vong() {
    TestDatabase::run(async |test| {
        let (_app, tree) = tree(test).await;
        let error = relocate(&tree.database, tree.a, tree.a1).await.unwrap_err();
        assert_eq!(error.code(), "org_cycle");
        assert_eq!(relocate(&tree.database, tree.a, tree.a).await.unwrap_err().code(), "org_cycle");
        assert_eq!(
            relocate(&tree.database, tree.root, tree.b).await.unwrap_err().code(),
            "org_root_immutable"
        );
        assert_eq!(
            relocate(&tree.database, org(), tree.b).await.unwrap_err().code(),
            "org_not_found"
        );
        assert_eq!(add(&tree.database, org(), org()).await.unwrap_err().code(), "org_not_found");
        // Dời hợp lệ: A (kèm A1) xuống dưới B.
        relocate(&tree.database, tree.a, tree.b).await.unwrap();
        let lineage =
            OrgTree::ancestors(tree.database.read(), DEFAULT_TENANT, tree.a1).await.unwrap();
        assert_eq!(lineage, vec![tree.a1, tree.a, tree.b, tree.root]);
        assert!(
            OrgTree::is_within(tree.database.read(), DEFAULT_TENANT, tree.a1, &[tree.b])
                .await
                .unwrap()
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn vuot_do_sau_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let (_app, tree) = tree(test).await;
        // Chuỗi dưới A1 (độ sâu 2) tới độ sâu tối đa.
        let mut parent = tree.a1;
        for _ in 3..=MAX_DEPTH {
            let next = org();
            add(&tree.database, next, parent).await.unwrap();
            parent = next;
        }
        let error = add(&tree.database, org(), parent).await.unwrap_err();
        assert_eq!(error.code(), "org_depth_exceeded");
        assert!(error.message().contains('8'), "{}", error.message());
        // Dời B (một nút) xuống đáy chuỗi: độ sâu 9, quá. Dời A (nhánh cao 7) xuống dưới B: đáy ở
        // độ sâu 2 + 7 = 9, quá. Dời A1 (nhánh cao 6) xuống dưới B: đáy ở 2 + 6 = 8, vừa.
        for (id, parent) in [(tree.b, parent), (tree.a, tree.b)] {
            assert_eq!(
                relocate(&tree.database, id, parent).await.unwrap_err().code(),
                "org_depth_exceeded"
            );
        }
        relocate(&tree.database, tree.a1, tree.b).await.unwrap();
        let deepest =
            OrgTree::ancestors(tree.database.read(), DEFAULT_TENANT, parent).await.unwrap();
        assert_eq!(deepest.len(), usize::from(MAX_DEPTH) + 1);
        let leaf = org();
        assert_eq!(
            add(&tree.database, leaf, parent).await.unwrap_err().code(),
            "org_depth_exceeded"
        );
    })
    .await
    .unwrap();
}

/// Bộ sinh số giả ngẫu nhiên đơn giản (LCG) — đủ để trộn thứ tự dời, lặp lại được.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, below: usize) -> usize {
        self.0 =
            self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) as usize) % below
    }
}

#[tokio::test]
async fn doi_nhanh_cap_nhat_bang_to_tien() {
    TestDatabase::run(async |test| {
        let (_app, tree) = tree(test).await;
        let mut parents: HashMap<OrgId, OrgId> =
            HashMap::from([(tree.a, tree.root), (tree.b, tree.root), (tree.a1, tree.a)]);
        let mut nodes = vec![tree.a, tree.b, tree.a1];
        let mut random = Lcg(34);
        for _ in 0..12 {
            let id = org();
            let parent = nodes[random.next(nodes.len())];
            if add(&tree.database, id, parent).await.is_ok() {
                parents.insert(id, parent);
                nodes.push(id);
            }
        }
        let chain = |parents: &HashMap<OrgId, OrgId>, mut id: OrgId| {
            let mut lineage = vec![id];
            while let Some(parent) = parents.get(&id) {
                lineage.push(*parent);
                id = *parent;
            }
            lineage
        };
        let (mut moved, mut refused) = (0, 0);
        for _ in 0..200 {
            let id = nodes[random.next(nodes.len())];
            let target =
                if random.next(5) == 0 { tree.root } else { nodes[random.next(nodes.len())] };
            match relocate(&tree.database, id, target).await {
                Ok(()) => {
                    moved += 1;
                    parents.insert(id, target);
                }
                Err(error) => {
                    refused += 1;
                    assert!(
                        ["org_cycle", "org_depth_exceeded"].contains(&error.code()),
                        "{}",
                        error.code()
                    );
                    let would_cycle = chain(&parents, target).contains(&id);
                    assert_eq!(would_cycle, error.code() == "org_cycle");
                }
            }
        }
        assert!(moved > 50 && refused > 0, "moved {moved}, refused {refused}");
        // Bảng tổ tiên khớp với cây tính lại từ `parent_id`, từng nút.
        for id in nodes.iter().copied().chain([tree.root]) {
            let stored =
                OrgTree::ancestors(tree.database.read(), DEFAULT_TENANT, id).await.unwrap();
            assert_eq!(stored, chain(&parents, id), "tổ tiên của {id:?}");
            assert!(stored.len() <= usize::from(MAX_DEPTH) + 1);
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doi_nut_cheo_nhau_dong_thoi_khong_tao_vong() {
    TestDatabase::run(async |test| {
        let (_app, tree) = tree(test).await;
        for _ in 0..4 {
            let (x, y) = (org(), org());
            add(&tree.database, x, tree.root).await.unwrap();
            add(&tree.database, y, tree.root).await.unwrap();
            let (first, second) =
                tokio::join!(relocate(&tree.database, x, y), relocate(&tree.database, y, x));
            let codes: Vec<&str> = [&first, &second]
                .iter()
                .filter_map(|result| result.as_ref().err().map(|error| error.code()))
                .collect();
            assert_eq!(codes, ["org_cycle"], "đúng một bên thành công: {first:?} {second:?}");
            let lx = OrgTree::ancestors(tree.database.read(), DEFAULT_TENANT, x).await.unwrap();
            let ly = OrgTree::ancestors(tree.database.read(), DEFAULT_TENANT, y).await.unwrap();
            assert_eq!(lx.last(), Some(&tree.root));
            assert_eq!(ly.last(), Some(&tree.root));
            assert!(lx.len() == 3 || ly.len() == 3, "một nút nằm dưới nút kia: {lx:?} {ly:?}");
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khoa_nut_va_ghi_du_lieu_dong_thoi() {
    TestDatabase::run(async |test| {
        let (_app, tree) = tree(test).await;
        create_records(&tree.database).await;
        let database = Arc::new(tree.database.clone());
        let locked = Arc::new(AtomicBool::new(false));
        let late_writes = Arc::new(AtomicUsize::new(0));
        let admin = Principal::with_all_permissions(Uuid::now_v7());
        let mut writers = Vec::new();
        for writer in 0..8 {
            let (database, locked, late_writes, admin) =
                (database.clone(), locked.clone(), late_writes.clone(), admin.clone());
            let target = OrgTarget::new(tree.a1, None);
            writers.push(tokio::spawn(async move {
                for attempt in 0..40 {
                    // Lần ghi bắt đầu SAU khi lệnh khoá đã commit thì phải bị từ chối.
                    let after_lock = locked.load(Ordering::SeqCst);
                    let Ok(transaction) = database.begin_write().await else { continue };
                    let allowed = admin.require_write_at(&transaction, &READ, target).await;
                    match allowed {
                        Ok(()) => {
                            insert_record(
                                &transaction,
                                target.org,
                                admin.user_id(),
                                &format!("{writer}-{attempt}"),
                            )
                            .await;
                            if transaction.commit().await.is_ok() && after_lock {
                                late_writes.fetch_add(1, Ordering::SeqCst);
                            }
                        }
                        Err(error) => {
                            assert_eq!(error.code(), "org_frozen");
                            transaction.rollback().await.unwrap();
                        }
                    }
                }
            }));
        }
        tokio::task::yield_now().await;
        let transaction = database.begin_write().await.unwrap();
        OrgTree::set_writable(&transaction, DEFAULT_TENANT, tree.a, false).await.unwrap();
        transaction.commit().await.unwrap();
        locked.store(true, Ordering::SeqCst);
        for writer in writers {
            writer.await.unwrap();
        }
        assert_eq!(
            late_writes.load(Ordering::SeqCst),
            0,
            "không lệnh ghi nào bắt đầu sau khi khoá mà thành công"
        );
        let transaction = database.begin_write().await.unwrap();
        let error = admin
            .require_write_at(&transaction, &READ, OrgTarget::new(tree.a1, None))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "org_frozen");
        transaction.rollback().await.unwrap();
    })
    .await
    .unwrap();
}
