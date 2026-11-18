//! Bước 54 mục 1, test `loc_danh_sach_khop_kiem_tung_ban_ghi` (P2): sinh ngẫu nhiên cây tổ chức,
//! bản ghi và grant (`own` / `unit` / `subtree` / `tenant`, hàng rào của API key); với mỗi người gọi,
//! tập bản ghi danh sách trả về (điều kiện SQL) phải BẰNG ĐÚNG tập bản ghi mà kiểm từng bản ghi
//! (`Principal::check`) cho qua. Chạy trên cả SQLite và Postgres.
//!
//! Sinh số bằng SplitMix64 với hạt cố định: lỗi thì in hạt và vòng để chạy lại đúng trường hợp đó.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use autoxide_kernel::access::{Decision, Grant, OrgTarget, Permission, Principal, Scope};
use autoxide_kernel::database::Database;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree};
use autoxide_testkit::{TestApp, TestDatabase};
use sea_orm::ActiveValue::Set;
use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{ColumnDef, Table};

const MAX_DEPTH: u8 = 8;
const READ: Permission = Permission::new("demo.read", "Read records.").scoped();
const OTHER: Permission = Permission::new("demo.other", "Something else.").scoped();
const SEEDS: [u64; 3] = [7, 2026, 0x5eed];
const ORGS: usize = 10;
const USERS: usize = 3;
const RECORDS: usize = 24;
const ROUNDS: usize = 30;

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
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// SplitMix64: đủ đều cho test, không cần thêm thư viện.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())]
    }
}

struct World {
    orgs: Vec<OrgId>,
    users: Vec<Uuid>,
    /// (id, nút, người sở hữu)
    records: Vec<(Uuid, OrgId, Uuid)>,
}

async fn build(database: &Database, rng: &mut Rng) -> World {
    let transaction = database.begin_write().await.unwrap();
    let root = OrgTree::insert_root(&transaction, DEFAULT_TENANT).await.unwrap();
    let mut orgs = vec![root];
    let mut depths = vec![1u8];
    for _ in 0..ORGS {
        // Cha ngẫu nhiên trong các nút còn chỗ cho thêm một tầng.
        let open: Vec<usize> = (0..orgs.len()).filter(|&index| depths[index] < MAX_DEPTH).collect();
        let parent = rng.pick(&open);
        let id = OrgId::from_uuid(Uuid::now_v7());
        OrgTree::insert(&transaction, DEFAULT_TENANT, id, orgs[parent], MAX_DEPTH).await.unwrap();
        orgs.push(id);
        depths.push(depths[parent] + 1);
    }
    transaction.commit().await.unwrap();

    let backend = database.write().get_database_backend();
    let table = Table::create()
        .table("demo_record")
        .col(ColumnDef::new("id").uuid().not_null().primary_key())
        .col(ColumnDef::new("tenant_id").uuid().not_null())
        .col(ColumnDef::new("org_id").uuid().not_null())
        .col(ColumnDef::new("owner_id").uuid().not_null())
        .to_owned();
    database.write().execute_raw(backend.build(&table)).await.unwrap();
    let users: Vec<Uuid> = (0..USERS).map(|_| Uuid::now_v7()).collect();
    let mut records = Vec::with_capacity(RECORDS);
    for _ in 0..RECORDS {
        let (id, org, owner) = (Uuid::now_v7(), rng.pick(&orgs), rng.pick(&users));
        record::ActiveModel {
            id: Set(id),
            tenant_id: Set(DEFAULT_TENANT.as_uuid()),
            org_id: Set(org.as_uuid()),
            owner_id: Set(owner),
        }
        .insert(database.write())
        .await
        .unwrap();
        records.push((id, org, owner));
    }
    World { orgs, users, records }
}

/// Người gọi ngẫu nhiên: thường là vài grant của `demo.read`; đôi khi admin, đôi khi chỉ có
/// permission khác; đôi khi có hàng rào của API key.
fn principal(world: &World, rng: &mut Rng) -> Principal {
    let user = rng.pick(&world.users);
    let scopes = [Scope::Own, Scope::Unit, Scope::Subtree, Scope::Tenant];
    let principal = match rng.below(10) {
        0 => Principal::with_all_permissions(user),
        1 => Principal::with_grants(
            user,
            [(OTHER.name().to_owned(), Grant::new(rng.pick(&world.orgs), Scope::Subtree))],
        ),
        _ => {
            // `tenant` hiếm hơn để phần lớn vòng thật sự lọc.
            let count = 1 + rng.below(3);
            let grants: Vec<(String, Grant)> = (0..count)
                .map(|_| {
                    let scope =
                        if rng.below(8) == 0 { Scope::Tenant } else { rng.pick(&scopes[..3]) };
                    (READ.name().to_owned(), Grant::new(rng.pick(&world.orgs), scope))
                })
                .collect();
            Principal::with_grants(user, grants)
        }
    };
    if rng.below(4) == 0 { principal.with_fence(rng.pick(&world.orgs)) } else { principal }
}

async fn listed(
    database: &Database,
    principal: &Principal,
    with_owner: bool,
) -> Option<BTreeSet<Uuid>> {
    let owner = with_owner.then_some(record::Column::OwnerId);
    let condition = match principal.scope_condition(&READ, record::Column::OrgId, owner) {
        Ok(condition) => condition,
        Err(error) => {
            assert_eq!(error.code(), "permission_denied");
            return None;
        }
    };
    let rows = record::Entity::find()
        .filter(record::Column::TenantId.eq(DEFAULT_TENANT.as_uuid()))
        .filter(condition)
        .all(database.read())
        .await
        .unwrap();
    Some(rows.into_iter().map(|row| row.id).collect())
}

async fn checked(
    database: &Database,
    world: &World,
    principal: &Principal,
    with_owner: bool,
) -> Option<BTreeSet<Uuid>> {
    let mut allowed = BTreeSet::new();
    let mut missing = false;
    for &(id, org, owner) in &world.records {
        let target = OrgTarget::new(org, with_owner.then_some(owner));
        match principal.check(database.read(), &READ, target).await.unwrap() {
            Decision::Allow => {
                allowed.insert(id);
            }
            Decision::Missing => missing = true,
            Decision::OutOfScope => {}
        }
    }
    if missing {
        assert!(allowed.is_empty(), "thiếu permission thì không bản ghi nào qua");
        return None;
    }
    Some(allowed)
}

#[tokio::test]
async fn loc_danh_sach_khop_kiem_tung_ban_ghi() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).build().await.unwrap();
        let database = app.database().clone();
        for seed in SEEDS {
            let mut rng = Rng(seed);
            let world = build(&database, &mut rng).await;
            let mut seen_partial = 0;
            for round in 0..ROUNDS {
                let principal = principal(&world, &mut rng);
                for with_owner in [true, false] {
                    let list = listed(&database, &principal, with_owner).await;
                    let each = checked(&database, &world, &principal, with_owner).await;
                    assert_eq!(
                        list, each,
                        "hạt {seed}, vòng {round}, cột người sở hữu {with_owner}: {principal:?}"
                    );
                    if list.as_ref().is_some_and(|set| !set.is_empty() && set.len() < RECORDS) {
                        seen_partial += 1;
                    }
                }
            }
            // Test chỉ có giá trị khi phần lớn vòng thật sự lọc (không phải tất cả / không gì).
            assert!(seen_partial >= ROUNDS / 2, "hạt {seed}: chỉ {seen_partial} vòng lọc một phần");
            // Hạt sau dựng bảng mẫu lại từ đầu (cây cũ ở lại, không ảnh hưởng: nút mới khác ID).
            let backend = database.write().get_database_backend();
            let drop = Table::drop().table("demo_record").to_owned();
            database.write().execute_raw(backend.build(&drop)).await.unwrap();
        }
    })
    .await
    .unwrap();
}
