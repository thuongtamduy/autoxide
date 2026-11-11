//! `TestDatabase` phải cho mỗi test một database riêng và dọn sạch sau khi dùng,
//! trên cả SQLite lẫn Postgres (gate chạy file này hai lần, mỗi database một lần).

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
// Clippy chỉ tự miễn cho hàm `#[test]`, không miễn cho hàm phụ, nên khai ở đây.
// Trong `src/` thì KHÔNG được khai dòng này (`cargo xtask check` chặn).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_testkit::TestDatabase;
use sea_orm::sea_query::{ColumnDef, Query, Table};
use sea_orm::{ConnectionTrait, Database, DatabaseConnection};

async fn create_marker_table(connection: &DatabaseConnection) {
    let create = Table::create()
        .table("marker")
        .col(ColumnDef::new("id").integer().not_null().primary_key())
        .to_owned();
    connection.execute(&create).await.unwrap();
    let insert =
        Query::insert().into_table("marker").columns(["id"]).values_panic([1.into()]).to_owned();
    connection.execute(&insert).await.unwrap();
}

async fn marker_rows(connection: &DatabaseConnection) -> Result<usize, sea_orm::DbErr> {
    let select = Query::select().column("id").from("marker").to_owned();
    Ok(connection.query_all(&select).await?.len())
}

#[tokio::test]
async fn moi_database_tach_biet_voi_database_khac() {
    TestDatabase::run(async |first| {
        TestDatabase::run(async |second| {
            assert_ne!(first.url(), second.url());
            assert_eq!(first.backend(), second.backend());

            let first_connection = first.connect().await.unwrap();
            let second_connection = second.connect().await.unwrap();
            assert_eq!(first_connection.get_database_backend(), first.backend());

            create_marker_table(&first_connection).await;

            assert_eq!(marker_rows(&first_connection).await.unwrap(), 1);
            // Bảng tạo ở database thứ nhất không tồn tại ở database thứ hai.
            assert!(marker_rows(&second_connection).await.is_err());
        })
        .await
        .unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn run_tra_ve_ket_qua_cua_bai_test() {
    let answer = TestDatabase::run(async |database| {
        let connection = database.connect().await.unwrap();
        create_marker_table(&connection).await;
        marker_rows(&connection).await.unwrap() + 41
    })
    .await
    .unwrap();
    assert_eq!(answer, 42);
}

#[tokio::test]
async fn database_bi_xoa_sau_khi_chay_xong() {
    let url = TestDatabase::run(async |database| {
        let connection = database.connect().await.unwrap();
        create_marker_table(&connection).await;
        connection.close().await.unwrap();
        database.url().to_owned()
    })
    .await
    .unwrap();

    // SQLite: thư mục chứa file đã mất. Postgres: database đã bị DROP.
    assert!(Database::connect(url).await.is_err());
}

#[tokio::test]
async fn database_bi_xoa_ca_khi_ket_noi_con_mo() {
    // Bài test quên đóng kết nối là chuyện thường; dọn dẹp vẫn phải chạy được.
    // Ở đây kết nối còn sống cả SAU khi `run` trả về.
    let (url, connection) = TestDatabase::run(async |database| {
        let connection = database.connect().await.unwrap();
        create_marker_table(&connection).await;
        (database.url().to_owned(), connection)
    })
    .await
    .unwrap();

    assert!(Database::connect(url).await.is_err());
    drop(connection);
}

#[tokio::test(flavor = "multi_thread")]
async fn nhieu_database_dong_thoi() {
    // Test chạy song song nên nhiều database được tạo và xoá cùng lúc là trạng
    // thái bình thường, không phải ngoại lệ.
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..8 {
        tasks.spawn(async {
            TestDatabase::run(async |database| {
                let connection = database.connect().await.unwrap();
                create_marker_table(&connection).await;
                assert_eq!(marker_rows(&connection).await.unwrap(), 1);
                database.url().to_owned()
            })
            .await
            .unwrap()
        });
    }
    let mut urls = tasks.join_all().await;
    urls.sort();
    urls.dedup();
    assert_eq!(urls.len(), 8);
}
