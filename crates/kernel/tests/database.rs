//! `Database` (kết nối tách đọc / ghi) phải cho cùng hành vi trên SQLite và
//! Postgres. Gate chạy file này trên cả hai.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
// Clippy chỉ tự miễn cho hàm `#[test]`, không miễn cho hàm phụ, nên khai ở đây.
// Trong `src/` thì KHÔNG được khai dòng này (`cargo xtask check` chặn).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseBackend, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_testkit::TestDatabase;
use sea_orm::sea_query::{ColumnDef, ForeignKey, Query, Table};
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
