//! `Database` (kết nối tách đọc / ghi) phải cho cùng hành vi trên SQLite và
//! Postgres. Gate chạy file này trên cả hai.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
// Clippy chỉ tự miễn cho hàm `#[test]`, không miễn cho hàm phụ, nên khai ở đây.
// Trong `src/` thì KHÔNG được khai dòng này (`cargo xtask check` chặn).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseBackend, DatabaseConfig};
use autoxide_kernel::database::{Database, is_transient};
use autoxide_kernel::error::Error;
use autoxide_testkit::TestDatabase;
use sea_orm::sea_query::{ColumnDef, Expr, ExprTrait as _, ForeignKey, Query, Table};
use sea_orm::{ConnectionTrait, DbBackend, Statement};

/// Cấu hình database trỏ vào `url`, cộng các biến `extra`.
fn config_for(url: &str, extra: &[(&str, &str)]) -> DatabaseConfig {
    let mut pairs = vec![("DATABASE_URL", url)];
    pairs.extend_from_slice(extra);
    let source = ConfigSource::from_pairs(pairs);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    reader.finish(config).unwrap().0
}

async fn connect(database: &TestDatabase) -> Database {
    Database::connect(&config_for(database.url(), &[])).await.unwrap()
}

/// Bảng `note(id, body)`.
async fn create_note_table(connection: &impl ConnectionTrait) {
    let create = Table::create()
        .table("note")
        .col(ColumnDef::new("id").integer().not_null().primary_key())
        .col(ColumnDef::new("body").string().not_null())
        .to_owned();
    connection.execute(&create).await.unwrap();
}

async fn insert_note(connection: &impl ConnectionTrait, id: i32) -> Result<(), sea_orm::DbErr> {
    let insert = Query::insert()
        .into_table("note")
        .columns(["id", "body"])
        .values_panic([id.into(), format!("note {id}").into()])
        .to_owned();
    connection.execute(&insert).await.map(|_| ())
}

async fn count_notes(connection: &impl ConnectionTrait) -> usize {
    let select = Query::select().column("id").from("note").to_owned();
    connection.query_all(&select).await.unwrap().len()
}

#[tokio::test]
async fn ghi_o_phia_ghi_thi_phia_doc_thay() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        create_note_table(db.write()).await;

        insert_note(db.write(), 1).await.unwrap();

        assert_eq!(count_notes(db.read()).await, 1);
        let expected = match database.backend() {
            DbBackend::Postgres => DatabaseBackend::Postgres,
            _ => DatabaseBackend::Sqlite,
        };
        assert_eq!(db.backend(), expected);
        db.ping().await.unwrap();
        db.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ghi_qua_ket_noi_doc_bi_tu_choi() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        create_note_table(db.write()).await;

        // Cả thêm dữ liệu lẫn đổi cấu trúc đều không qua được kết nối đọc.
        assert!(insert_note(db.read(), 1).await.is_err());
        let create = Table::create()
            .table("other")
            .col(ColumnDef::new("id").integer().not_null().primary_key())
            .to_owned();
        assert!(db.read().execute(&create).await.is_err());

        // Phía ghi không bị ảnh hưởng.
        insert_note(db.write(), 1).await.unwrap();
        assert_eq!(count_notes(db.read()).await, 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn transaction_ghi_chi_co_hieu_luc_sau_commit() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        create_note_table(db.write()).await;

        let transaction = db.begin_write().await.unwrap();
        insert_note(&transaction, 1).await.unwrap();
        insert_note(&transaction, 2).await.unwrap();
        // Chưa commit: phía đọc chưa thấy gì, và không bị chặn.
        assert_eq!(count_notes(db.read()).await, 0);
        transaction.commit().await.unwrap();

        assert_eq!(count_notes(db.read()).await, 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn transaction_khong_commit_thi_rollback() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        create_note_table(db.write()).await;

        {
            let transaction = db.begin_write().await.unwrap();
            insert_note(&transaction, 1).await.unwrap();
            // Ra khỏi khối này `transaction` bị drop mà chưa commit.
        }
        let transaction = db.begin_write().await.unwrap();
        insert_note(&transaction, 2).await.unwrap();
        transaction.rollback().await.unwrap();

        // Phía ghi dùng lại được ngay và không còn dấu vết của hai transaction trên.
        insert_note(db.write(), 3).await.unwrap();
        assert_eq!(count_notes(db.read()).await, 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn foreign_key_duoc_kiem() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        create_note_table(db.write()).await;
        let create = Table::create()
            .table("comment")
            .col(ColumnDef::new("id").integer().not_null().primary_key())
            .col(ColumnDef::new("note_id").integer().not_null())
            .foreign_key(ForeignKey::create().from("comment", "note_id").to("note", "id"))
            .to_owned();
        db.write().execute(&create).await.unwrap();
        insert_note(db.write(), 1).await.unwrap();

        let comment = |note_id: i32| {
            Query::insert()
                .into_table("comment")
                .columns(["id", "note_id"])
                .values_panic([note_id.into(), note_id.into()])
                .to_owned()
        };

        db.write().execute(&comment(1)).await.unwrap();
        // SQLite chỉ kiểm foreign key khi được bật trên kết nối; quên bật thì dòng
        // dưới đây lặng lẽ thành công trên SQLite và chỉ hỏng trên Postgres.
        assert!(db.write().execute(&comment(999)).await.is_err());
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn nhieu_lenh_ghi_dong_thoi_deu_thanh_cong() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        create_note_table(db.write()).await;

        // 64 task cùng ghi, mỗi task một transaction, xen với lệnh đọc. Trên SQLite
        // chúng xếp hàng qua một kết nối ghi; không task nào được phép nhận lỗi
        // "database is locked".
        let mut tasks = tokio::task::JoinSet::new();
        for id in 0..64 {
            let db = db.clone();
            tasks.spawn(async move {
                let transaction = db.begin_write().await.unwrap();
                insert_note(&transaction, id).await.unwrap();
                let _ = count_notes(db.read()).await;
                transaction.commit().await.unwrap();
            });
        }
        tasks.join_all().await;

        assert_eq!(count_notes(db.read()).await, 64);
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn hai_tien_trinh_cung_ghi_mot_database() {
    TestDatabase::run(async |database| {
        // Hai `Database` độc lập trên cùng một database, như khi `api` và `worker`
        // chạy thành hai tiến trình. Trên SQLite đây là HAI kết nối ghi tranh nhau
        // một file: `BEGIN IMMEDIATE` cộng thời gian chờ lock phải làm chúng xếp
        // hàng thay vì hỏng.
        let api = connect(database).await;
        let worker = connect(database).await;
        create_note_table(api.write()).await;

        let mut tasks = tokio::task::JoinSet::new();
        for id in 0..40 {
            let db = if id % 2 == 0 { api.clone() } else { worker.clone() };
            tasks.spawn(async move {
                let transaction = db.begin_write().await.unwrap();
                // Đọc rồi mới ghi trong cùng transaction: đúng tình huống mà kiểu
                // transaction mặc định của SQLite (DEFERRED) hỏng giữa chừng.
                let _ = count_notes(&transaction).await;
                insert_note(&transaction, id).await.unwrap();
                transaction.commit().await.unwrap();
            });
        }
        tasks.join_all().await;

        assert_eq!(count_notes(api.read()).await, 40);
        assert_eq!(count_notes(worker.read()).await, 40);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn sqlite_bat_wal_va_tu_tao_thu_muc() {
    // Nhánh chỉ SQLite có: ép SQLite bất kể gate đang chạy lượt nào.
    TestDatabase::run_for(None, async |database| {
        // File nằm trong một thư mục con CHƯA tồn tại.
        let directory = database.url().trim_start_matches("sqlite://");
        let directory = directory.split('?').next().unwrap().trim_end_matches("test.sqlite");
        let url = format!("sqlite://{directory}nested/deeper/app.sqlite");

        let db = Database::connect(&config_for(&url, &[])).await.unwrap();

        assert!(std::path::Path::new(directory).join("nested/deeper/app.sqlite").exists());
        let pragma = Statement::from_string(DbBackend::Sqlite, "PRAGMA journal_mode");
        let row = db.write().query_one_raw(pragma).await.unwrap().unwrap();
        assert_eq!(row.try_get_by_index::<String>(0).unwrap(), "wal");
        db.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn loi_ket_noi_khong_lo_mat_khau() {
    // Port 1 không có gì nghe: kết nối bị từ chối ngay, không cần database thật.
    let config = config_for(
        "postgres://app:s3cr3t@127.0.0.1:1/app",
        &[("AUTOXIDE_DATABASE_WAIT_SECONDS", "1")],
    );

    let message = Database::connect(&config).await.unwrap_err().to_string();

    assert!(
        message.starts_with("database: cannot connect to `postgres://app:***@127.0.0.1:1/app`"),
        "{message}"
    );
    assert!(!message.contains("s3cr3t"), "{message}");
}

/// Đổi `body` của note `id` trong `connection`; lỗi trả nguyên cho bên gọi.
async fn touch_note(connection: &impl ConnectionTrait, id: i32) -> Result<(), Error> {
    let update = Query::update()
        .table("note")
        .value("body", format!("touched {id}"))
        .and_where(Expr::col("id").eq(id))
        .to_owned();
    connection.execute(&update).await?;
    Ok(())
}

/// Hai transaction khoá hai dòng theo thứ tự ngược nhau. Postgres phát hiện deadlock và huỷ một
/// bên (`40P01`) — `write_with_retry` chạy lại bên đó và cả hai cùng xong. SQLite vốn chỉ một bên
/// ghi nên không deadlock; cả hai cũng phải xong.
#[tokio::test(flavor = "multi_thread")]
async fn chay_lai_khi_deadlock() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        create_note_table(db.write()).await;
        insert_note(db.write(), 1).await.unwrap();
        insert_note(db.write(), 2).await.unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));
        // Điểm hẹn của hai transaction (chỉ Postgres): cả hai khoá xong dòng đầu rồi mới khoá dòng
        // thứ hai, nên chắc chắn deadlock dù máy nhanh hay chậm. Trước đây chỉ ngủ 300 ms — máy
        // bận thì một bên xong trước khi bên kia kịp khoá, không có deadlock, test hỏng ngẫu nhiên.
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let mut tasks = tokio::task::JoinSet::new();
        for (first, second) in [(1, 2), (2, 1)] {
            let (db, attempts, barrier) = (db.clone(), attempts.clone(), barrier.clone());
            tasks.spawn(async move {
                let (db, attempts, barrier) = (&db, &attempts, &barrier);
                db.write_with_retry(move || async move {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    let transaction = db.begin_write().await?;
                    touch_note(&transaction, first).await?;
                    match db.backend() {
                        // Lần chạy lại sau deadlock không còn ai tới điểm hẹn: hết hạn chờ thì đi
                        // tiếp (bên kia đã bị huỷ, dòng đã nhả).
                        DatabaseBackend::Postgres => {
                            let _ =
                                tokio::time::timeout(Duration::from_secs(2), barrier.wait()).await;
                        }
                        // SQLite: `begin_write` giữ quyền ghi cả database, bên kia chờ ở
                        // `begin_write` — điểm hẹn sẽ chờ nhau mãi, nên chỉ ngủ như cũ.
                        DatabaseBackend::Sqlite => {
                            tokio::time::sleep(Duration::from_millis(300)).await;
                        }
                    }
                    touch_note(&transaction, second).await?;
                    transaction.commit().await?;
                    Ok(())
                })
                .await
                .unwrap();
            });
        }
        tasks.join_all().await;
        let attempts = attempts.load(Ordering::SeqCst);
        match db.backend() {
            DatabaseBackend::Postgres => assert!(attempts >= 3, "{attempts} attempts"),
            DatabaseBackend::Sqlite => assert_eq!(attempts, 2),
        }
    })
    .await
    .unwrap();
}

/// Một tiến trình khác giữ quyền ghi lâu hơn thời gian chờ: SQLite báo bận, `write_with_retry`
/// chờ rồi chạy lại và xong. Postgres không khoá cả database nên xong ngay lần đầu.
#[tokio::test(flavor = "multi_thread")]
async fn chay_lai_khi_database_ban() {
    TestDatabase::run(async |database| {
        let config = config_for(database.url(), &[("AUTOXIDE_DATABASE_WAIT_SECONDS", "1")]);
        let holder = Database::connect(&config).await.unwrap();
        let waiter = Database::connect(&config).await.unwrap();
        create_note_table(holder.write()).await;

        let held = holder.begin_write().await.unwrap();
        insert_note(&held, 1).await.unwrap();
        let release = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            held.commit().await.unwrap();
        });
        let attempts = AtomicUsize::new(0);
        let attempts_ref = &attempts;
        let waiter_ref = &waiter;
        waiter
            .write_with_retry(move || async move {
                attempts_ref.fetch_add(1, Ordering::SeqCst);
                let transaction = waiter_ref.begin_write().await?;
                insert_note(&transaction, 2).await?;
                transaction.commit().await?;
                Ok(())
            })
            .await
            .unwrap();
        release.await.unwrap();
        assert_eq!(count_notes(waiter.read()).await, 2);
        let attempts = attempts.load(Ordering::SeqCst);
        match waiter.backend() {
            DatabaseBackend::Sqlite => assert!(attempts >= 2, "{attempts} attempts"),
            DatabaseBackend::Postgres => assert_eq!(attempts, 1),
        }
    })
    .await
    .unwrap();
}

/// Lỗi không tạm thời trả ngay, không chạy lại: lỗi nghiệp vụ và vi phạm unique.
#[tokio::test]
async fn loi_khac_khong_chay_lai() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        create_note_table(db.write()).await;
        insert_note(db.write(), 1).await.unwrap();
        let attempts = AtomicUsize::new(0);
        let (db_ref, attempts_ref) = (&db, &attempts);

        let error = db
            .write_with_retry(move || async move {
                attempts_ref.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>(Error::conflict("notes.title_taken", "taken"))
            })
            .await
            .unwrap_err();
        assert_eq!(error.code(), "notes.title_taken");

        let error = db
            .write_with_retry(move || async move {
                attempts_ref.fetch_add(1, Ordering::SeqCst);
                let transaction = db_ref.begin_write().await?;
                insert_note(&transaction, 1).await?;
                transaction.commit().await?;
                Ok(())
            })
            .await
            .unwrap_err();
        assert_eq!(error.code(), "internal_error");
        assert!(!is_transient(&error));
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    })
    .await
    .unwrap();
}
