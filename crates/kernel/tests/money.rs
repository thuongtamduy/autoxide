//! Nền cho nghiệp vụ tiền (bước 29) trên database thật — gate chạy file này trên cả SQLite và
//! Postgres: `Money` lưu thành hai cột, trừ số dư có điều kiện khi nhiều request chạy cùng lúc,
//! khoá dòng, và mã tham chiếu chống trùng lệnh.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
// Clippy chỉ tự miễn cho hàm `#[test]`, không miễn cho hàm phụ, nên khai ở đây.
// Trong `src/` thì KHÔNG được khai dòng này (`cargo xtask check` chặn).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::reference::{self, Reference, Submitted};
use autoxide_kernel::rows::{self, Bounds};
use autoxide_testkit::TestDatabase;
use sea_orm::sea_query::{ColumnDef, Index, Table};
use sea_orm::{ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

/// Bảng ví mẫu: số dư lưu theo cách chuẩn (`balance_minor` + `balance_currency`), kèm mã tham
/// chiếu duy nhất theo chủ ví.
mod wallet {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "wallet")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub owner: Uuid,
        pub balance_minor: i64,
        pub balance_currency: String,
        pub reference: Option<String>,
        pub reference_fingerprint: Option<String>,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

async fn connect(database: &TestDatabase) -> Database {
    let source = ConfigSource::from_pairs([("DATABASE_URL", database.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    let db = Database::connect(&reader.finish(config).unwrap().0).await.unwrap();
    let create = Table::create()
        .table("wallet")
        .col(ColumnDef::new("id").uuid().not_null().primary_key())
        .col(ColumnDef::new("owner").uuid().not_null())
        .col(ColumnDef::new("balance_minor").big_integer().not_null())
        .col(ColumnDef::new("balance_currency").string_len(3).not_null())
        .col(ColumnDef::new("reference").string_len(100).null())
        .col(ColumnDef::new("reference_fingerprint").string_len(64).null())
        .to_owned();
    db.write().execute(&create).await.unwrap();
    let unique = Index::create()
        .name("wallet_reference_idx")
        .table("wallet")
        .col("owner")
        .col("reference")
        .unique()
        .to_owned();
    db.write().execute(&unique).await.unwrap();
    db
}

async fn open_wallet(db: &Database, balance: Money) -> Uuid {
    let id = Uuid::now_v7();
    wallet::ActiveModel {
        id: Set(id),
        owner: Set(Uuid::now_v7()),
        balance_minor: Set(balance.minor()),
        balance_currency: Set(balance.currency().code().to_owned()),
        reference: Set(None),
        reference_fingerprint: Set(None),
    }
    .insert(db.write())
    .await
    .unwrap();
    id
}

async fn balance(db: &Database, id: Uuid) -> Money {
    let row = wallet::Entity::find_by_id(id).one(db.read()).await.unwrap().unwrap();
    Money::from_columns(row.balance_minor, &row.balance_currency).unwrap()
}

#[tokio::test]
async fn money_luu_hai_cot_khong_sai_lech() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        for money in [
            Money::new(i64::MAX, Currency::USD),
            Money::new(i64::MIN, Currency::EUR),
            Money::new(0, Currency::VND),
            Money::parse("-1", "KWD").unwrap(),
        ] {
            let id = open_wallet(&db, money).await;
            assert_eq!(balance(&db, id).await, money);
        }
    })
    .await
    .unwrap();
}

/// 40 request cùng trừ 30 từ số dư 1000: đúng 33 lần thành công, số dư còn 10, không bao giờ âm.
#[tokio::test(flavor = "multi_thread")]
async fn tru_dong_thoi_khong_bao_gio_am() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let id = open_wallet(&db, Money::new(1000, Currency::USD)).await;
        let succeeded = Arc::new(AtomicUsize::new(0));
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..40 {
            let (db, succeeded) = (db.clone(), succeeded.clone());
            tasks.spawn(async move {
                let transaction = db.begin_write().await.unwrap();
                let changed = rows::adjust::<wallet::Entity, _>(
                    &transaction,
                    wallet::Column::BalanceMinor,
                    wallet::Column::Id.eq(id),
                    -30,
                    Bounds::NON_NEGATIVE,
                )
                .await
                .unwrap();
                transaction.commit().await.unwrap();
                if changed == 1 {
                    succeeded.fetch_add(1, Ordering::SeqCst);
                }
            });
        }
        tasks.join_all().await;
        assert_eq!(succeeded.load(Ordering::SeqCst), 33);
        assert_eq!(balance(&db, id).await, Money::new(10, Currency::USD));
    })
    .await
    .unwrap();
}

/// Cộng không bao giờ tràn `i64` — SQLite sẽ âm thầm đổi số tràn thành số thực, Postgres báo
/// lỗi; điều kiện chặn trước nên hai bên như nhau.
#[tokio::test]
async fn cong_khong_tran_va_ton_trong_gioi_han() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let id = open_wallet(&db, Money::new(i64::MAX - 5, Currency::USD)).await;
        let adjust = async |delta, bounds| {
            rows::adjust::<wallet::Entity, _>(
                db.write(),
                wallet::Column::BalanceMinor,
                wallet::Column::Id.eq(id),
                delta,
                bounds,
            )
            .await
            .unwrap()
        };
        assert_eq!(adjust(6, Bounds::ANY).await, 0);
        assert_eq!(adjust(5, Bounds::ANY).await, 1);
        assert_eq!(balance(&db, id).await.minor(), i64::MAX);
        assert_eq!(adjust(i64::MIN, Bounds::ANY).await, 1);
        assert_eq!(balance(&db, id).await.minor(), -1);
        // Được âm tới hạn mức -10: -1 - 9 = -10 vừa chạm sàn, trừ thêm 1 thì không.
        assert_eq!(adjust(-9, Bounds { floor: -10, ceiling: 0 }).await, 1);
        assert_eq!(adjust(-1, Bounds { floor: -10, ceiling: 0 }).await, 0);
        assert_eq!(adjust(i64::MIN, Bounds::ANY).await, 0);
        assert_eq!(balance(&db, id).await.minor(), -10);
    })
    .await
    .unwrap();
}

/// Chuyển tiền giữa hai ví theo hai chiều cùng lúc: khoá cả hai dòng theo thứ tự ID trước khi
/// ghi, nên không deadlock và tổng không lệch.
#[tokio::test(flavor = "multi_thread")]
async fn chuyen_qua_lai_khoa_theo_thu_tu_tong_khong_lech() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let a = open_wallet(&db, Money::new(500, Currency::USD)).await;
        let b = open_wallet(&db, Money::new(500, Currency::USD)).await;
        let mut tasks = tokio::task::JoinSet::new();
        for round in 0..30 {
            let db = db.clone();
            let (from, to) = if round % 2 == 0 { (a, b) } else { (b, a) };
            tasks.spawn(async move {
                let transaction = db.begin_write().await.unwrap();
                let locked = rows::lock(
                    &transaction,
                    wallet::Entity::find().filter(wallet::Column::Id.is_in([from, to])),
                    wallet::Column::Id,
                )
                .await
                .unwrap();
                assert_eq!(locked.len(), 2);
                let source = locked.iter().find(|row| row.id == from).unwrap();
                let amount = 7;
                if source.balance_minor >= amount {
                    for (id, delta) in [(from, -amount), (to, amount)] {
                        let changed = rows::adjust::<wallet::Entity, _>(
                            &transaction,
                            wallet::Column::BalanceMinor,
                            wallet::Column::Id.eq(id),
                            delta,
                            Bounds::NON_NEGATIVE,
                        )
                        .await
                        .unwrap();
                        assert_eq!(changed, 1);
                    }
                }
                transaction.commit().await.unwrap();
            });
        }
        tasks.join_all().await;
        let total = balance(&db, a).await.checked_add(balance(&db, b).await).unwrap();
        assert_eq!(total, Money::new(1000, Currency::USD));
        assert_eq!(balance(&db, a).await, Money::new(500, Currency::USD));
    })
    .await
    .unwrap();
}

/// Đọc và khoá một dòng có điều kiện: không khớp thì `None`.
#[tokio::test]
async fn khoa_mot_dong_co_dieu_kien() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let id = open_wallet(&db, Money::new(5, Currency::USD)).await;
        let transaction = db.begin_write().await.unwrap();
        let found = rows::lock_one(&transaction, wallet::Entity::find_by_id(id)).await.unwrap();
        assert_eq!(found.unwrap().balance_minor, 5);
        let rich = wallet::Entity::find_by_id(id).filter(wallet::Column::BalanceMinor.gte(10));
        assert!(rows::lock_one(&transaction, rich).await.unwrap().is_none());
        transaction.commit().await.unwrap();
    })
    .await
    .unwrap();
}

/// Mở ví bằng lệnh mang `reference`: gửi lại cùng nội dung trả ví cũ, nội dung khác bị từ chối,
/// 16 request đồng thời chỉ tạo một ví.
#[tokio::test(flavor = "multi_thread")]
async fn ma_tham_chieu_chi_tao_mot_lan() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let owner = Uuid::now_v7();
        // Mở ví theo cách service sẽ viết: tìm theo (chủ ví, reference), so dấu vân tay, chưa có
        // thì tạo — mọi thứ trong một transaction ghi.
        let open =
            async move |db: &Database, raw: &str, amount: i64| -> Result<Submitted<Uuid>, Error> {
                let mut problems = Vec::new();
                let reference = Reference::checked(raw, "reference", &mut problems).unwrap();
                let money = Money::new(amount, Currency::USD);
                let fingerprint = reference::fingerprint(&money).unwrap();
                let (reference, fingerprint) = (&reference, &fingerprint);
                reference::submit_once(db, move || async move {
                    let transaction = db.begin_write().await?;
                    let stored = wallet::Entity::find()
                        .filter(wallet::Column::Owner.eq(owner))
                        .filter(wallet::Column::Reference.eq(reference.as_str()))
                        .one(&transaction)
                        .await?
                        .map(|row| (row.id, row.reference_fingerprint.unwrap_or_default()));
                    if let Some(id) = reference::replayed(stored, reference, fingerprint)? {
                        return Ok(Submitted::Replayed(id));
                    }
                    let id = Uuid::now_v7();
                    wallet::ActiveModel {
                        id: Set(id),
                        owner: Set(owner),
                        balance_minor: Set(money.minor()),
                        balance_currency: Set(money.currency().code().to_owned()),
                        reference: Set(Some(reference.as_str().to_owned())),
                        reference_fingerprint: Set(Some(fingerprint.clone())),
                    }
                    .insert(&transaction)
                    .await?;
                    transaction.commit().await?;
                    Ok(Submitted::Created(id))
                })
                .await
            };

        let first = open(&db, "INV-1", 100).await.unwrap();
        let Submitted::Created(id) = first else { panic!("{first:?}") };
        assert_eq!(open(&db, "INV-1", 100).await.unwrap(), Submitted::Replayed(id));
        let reused = open(&db, "INV-1", 999).await.unwrap_err();
        assert_eq!(reused.code(), "reference_reused");

        let created = Arc::new(AtomicUsize::new(0));
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..16 {
            let (db, created) = (db.clone(), created.clone());
            tasks.spawn(async move {
                match open(&db, "INV-2", 50).await.unwrap() {
                    Submitted::Created(_) => created.fetch_add(1, Ordering::SeqCst),
                    Submitted::Replayed(_) => 0,
                };
            });
        }
        tasks.join_all().await;
        assert_eq!(created.load(Ordering::SeqCst), 1);
        let rows = wallet::Entity::find().filter(wallet::Column::Owner.eq(owner)).all(db.read());
        assert_eq!(rows.await.unwrap().len(), 2);
    })
    .await
    .unwrap();
}
