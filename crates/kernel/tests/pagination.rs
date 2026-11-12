//! Pagination theo UUIDv7 phải cho cùng kết quả trên SQLite và Postgres. Gate
//! chạy file này trên cả hai.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
// Clippy chỉ tự miễn cho hàm `#[test]`, không miễn cho hàm phụ, nên khai ở đây.
// Trong `src/` thì KHÔNG được khai dòng này (`cargo xtask check` chặn).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::pagination::{Order, Page, PageRequest, fetch_page};
use autoxide_testkit::TestDatabase;
use sea_orm::sea_query::{ColumnDef, Table};
use sea_orm::{ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

mod item {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "item")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub kind: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

async fn connect(database: &TestDatabase) -> Database {
    let source = ConfigSource::from_pairs([("DATABASE_URL", database.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    let db = Database::connect(&reader.finish(config).unwrap().0).await.unwrap();
    let create = Table::create()
        .table("item")
        .col(ColumnDef::new("id").uuid().not_null().primary_key())
        .col(ColumnDef::new("kind").string().not_null())
        .to_owned();
    db.write().execute(&create).await.unwrap();
    db
}

/// Thêm `count` dòng, xen kẽ kind `a` / `b`; trả về ID theo thứ tự tạo.
async fn insert_items(db: &Database, count: usize) -> Vec<Uuid> {
    let mut ids = Vec::new();
    for index in 0..count {
        let id = Uuid::now_v7();
        let kind = if index % 2 == 0 { "a" } else { "b" };
        item::ActiveModel { id: Set(id), kind: Set(kind.to_owned()) }
            .insert(db.write())
            .await
            .unwrap();
        ids.push(id);
    }
    ids
}

async fn page(db: &Database, request: &PageRequest) -> Page<item::Model> {
    fetch_page(item::Entity::find(), item::Column::Id, |row| row.id, request, db.read())
        .await
        .unwrap()
}

/// Lật hết các trang theo `next_cursor`; trả về (ID theo thứ tự nhận, số trang).
async fn walk(db: &Database, limit: u32, order: Order) -> (Vec<Uuid>, usize) {
    let mut request = PageRequest::first(limit, order);
    let (mut ids, mut pages) = (Vec::new(), 0);
    loop {
        let page = page(db, &request).await;
        pages += 1;
        assert!(page.data.len() <= limit as usize);
        assert_eq!(page.pagination.limit, limit);
        ids.extend(page.data.iter().map(|row| row.id));
        match page.pagination.next_cursor {
            Some(cursor) => request = request.after(Uuid::parse_str(&cursor).unwrap()),
            None => return (ids, pages),
        }
    }
}

#[tokio::test]
async fn lat_het_cac_trang_theo_ca_hai_chieu() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let created = insert_items(&db, 25).await;

        let (ascending, pages) = walk(&db, 10, Order::Asc).await;
        assert_eq!(ascending, created);
        assert_eq!(pages, 3);

        let (descending, pages) = walk(&db, 10, Order::Desc).await;
        let mut newest_first = created.clone();
        newest_first.reverse();
        assert_eq!(descending, newest_first);
        assert_eq!(pages, 3);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn so_dong_chia_het_cho_limit_thi_khong_co_trang_rong_thua() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        insert_items(&db, 20).await;

        let (ids, pages) = walk(&db, 10, Order::Asc).await;

        // Trang thứ hai đầy và là trang cuối: `next_cursor` phải là null ngay, chứ
        // không dẫn client tới một trang thứ ba rỗng.
        assert_eq!((ids.len(), pages), (20, 2));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bang_rong_thi_mot_trang_rong_khong_con_tro() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;

        let page = page(&db, &PageRequest::default()).await;

        assert!(page.data.is_empty());
        assert_eq!(page.pagination.next_cursor, None);
        assert_eq!(page.pagination.limit, 50);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dieu_kien_loc_duoc_giu_qua_cac_trang() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let created = insert_items(&db, 25).await;
        // Kind `a` là các dòng ở vị trí chẵn: 13 dòng.
        let expected: Vec<Uuid> = created.iter().copied().step_by(2).collect();

        let mut request = PageRequest::first(5, Order::Asc);
        let mut ids = Vec::new();
        loop {
            let select = item::Entity::find().filter(item::Column::Kind.eq("a"));
            let page = fetch_page(select, item::Column::Id, |row| row.id, &request, db.read())
                .await
                .unwrap();
            assert!(page.data.iter().all(|row| row.kind == "a"));
            ids.extend(page.data.iter().map(|row| row.id));
            match page.pagination.next_cursor {
                Some(cursor) => request = request.after(Uuid::parse_str(&cursor).unwrap()),
                None => break,
            }
        }

        assert_eq!(ids, expected);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn du_lieu_them_giua_chung_khong_lam_trung_hay_sot() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let mut created = insert_items(&db, 15).await;

        // Client lấy trang đầu…
        let first = page(&db, &PageRequest::first(10, Order::Asc)).await;
        // …rồi có dữ liệu mới được thêm trước khi client lấy trang kế.
        created.extend(insert_items(&db, 3).await);
        let cursor = Uuid::parse_str(&first.pagination.next_cursor.unwrap()).unwrap();
        let second = page(&db, &PageRequest::first(10, Order::Asc).after(cursor)).await;

        let seen: Vec<Uuid> = first.data.iter().chain(&second.data).map(|row| row.id).collect();
        // Mọi dòng xuất hiện đúng một lần, theo thứ tự tạo, gồm cả dòng mới.
        assert_eq!(seen, created);
        assert_eq!(second.pagination.next_cursor, None);

        // Chiều mới-nhất-trước: dòng mới chen vào ĐẦU danh sách không đẩy dòng cũ
        // sang trang sau thành trùng (lỗi kinh điển của pagination kiểu OFFSET).
        let first = page(&db, &PageRequest::first(10, Order::Desc)).await;
        insert_items(&db, 4).await;
        let cursor = Uuid::parse_str(&first.pagination.next_cursor.unwrap()).unwrap();
        let second = page(&db, &PageRequest::first(10, Order::Desc).after(cursor)).await;
        let mut expected = created.clone();
        expected.reverse();
        let seen: Vec<Uuid> = first.data.iter().chain(&second.data).map(|row| row.id).collect();
        assert_eq!(seen, expected);
    })
    .await
    .unwrap();
}

/// Sắp theo cột khác ID (nhiều dòng cùng giá trị): lật hết các trang theo `next_cursor`
/// thì được mọi dòng đúng một lần, theo (cột, ID) — trên cả hai database.
#[tokio::test]
async fn sap_theo_cot_khac_lat_trang_khong_trung_khong_sot() {
    use autoxide_kernel::pagination::{PageQuery, SortValue, fetch_page_by};

    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let ids = insert_items(&db, 7).await;

        for order in [Order::Asc, Order::Desc] {
            let mut cursor: Option<String> = None;
            let mut seen = Vec::new();
            loop {
                let query = PageQuery {
                    limit: Some("2".to_owned()),
                    cursor: cursor.clone(),
                    order: Some(if order == Order::Asc { "asc" } else { "desc" }.to_owned()),
                };
                let request = PageRequest::try_from(query).unwrap();
                let page = fetch_page_by(
                    item::Entity::find(),
                    item::Column::Kind,
                    |row| SortValue::Text(row.kind.clone()),
                    item::Column::Id,
                    |row| row.id,
                    &request,
                    db.read(),
                )
                .await
                .unwrap();
                seen.extend(page.data.iter().map(|row| (row.kind.clone(), row.id)));
                match page.pagination.next_cursor {
                    Some(next) => cursor = Some(next),
                    None => break,
                }
            }
            let mut expected: Vec<(String, Uuid)> = ids
                .iter()
                .enumerate()
                .map(|(index, id)| ((if index % 2 == 0 { "a" } else { "b" }).to_owned(), *id))
                .collect();
            expected.sort();
            if order == Order::Desc {
                expected.reverse();
            }
            assert_eq!(seen, expected, "{order:?}");
        }

        // Con trỏ của danh sách sắp theo ID không dùng được cho danh sách sắp theo cột.
        let request = PageRequest::first(2, Order::Asc).after(ids[0]);
        let mixed = fetch_page_by(
            item::Entity::find(),
            item::Column::Kind,
            |row| SortValue::Text(row.kind.clone()),
            item::Column::Id,
            |row| row.id,
            &request,
            db.read(),
        )
        .await
        .unwrap_err();
        assert_eq!(mixed.fields()[0].field, "cursor");

        // Con trỏ bị sửa cho sai kiểu (số cho cột chữ): cả hai database trả lỗi con trỏ,
        // không phải lỗi máy chủ trên Postgres (D-149).
        use base64::Engine as _;
        let forged = serde_json::json!({"c": "kind", "v": {"i": 5}, "id": ids[0]});
        let forged = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&forged).unwrap());
        let query = PageQuery { limit: None, cursor: Some(forged), order: None };
        let request = PageRequest::try_from(query).unwrap();
        let wrong_type = fetch_page_by(
            item::Entity::find(),
            item::Column::Kind,
            |row| SortValue::Text(row.kind.clone()),
            item::Column::Id,
            |row| row.id,
            &request,
            db.read(),
        )
        .await
        .unwrap_err();
        assert_eq!(wrong_type.fields()[0].field, "cursor");
        db.close().await.unwrap();
    })
    .await
    .unwrap();
}
