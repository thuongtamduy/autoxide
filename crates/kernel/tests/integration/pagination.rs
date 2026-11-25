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

// Bảng có cột sắp được phép trống: `fetch_page_by` phải từ chối (dòng NULL rơi khỏi mọi trang sau trang đầu).
mod nullable {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "nullable_item")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub note: Option<String>,
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
                let direction = if order == Order::Asc { "asc" } else { "desc" };
                let query = PageQuery::new(
                    Some("2".to_owned()),
                    cursor.clone(),
                    Some(direction.to_owned()),
                );
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
        let query = PageQuery::new(None, Some(forged), None);
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

#[tokio::test]
async fn trang_theo_tang_xep_tang_truoc_lat_khong_trung_khong_sot() {
    use autoxide_kernel::pagination::{PageQuery, fetch_page_tiers};
    use sea_orm::Condition;

    TestDatabase::run(async |database| {
        let db = connect(database).await;
        // 9 dòng: a, b, c, a, b, c… — tầng 1 là `b`, tầng 2 là `a` (không khớp tầng 1); `c` bị bỏ.
        let mut ids = Vec::new();
        for index in 0..9 {
            let id = Uuid::now_v7();
            let kind = ["a", "b", "c"][index % 3];
            item::ActiveModel { id: Set(id), kind: Set(kind.to_owned()) }
                .insert(db.write())
                .await
                .unwrap();
            ids.push((kind, id));
        }
        let tiers = [
            Condition::all().add(item::Column::Kind.eq("b")),
            Condition::all().add(item::Column::Kind.is_in(["a", "b"])),
        ];
        for (direction, order) in [("asc", Order::Asc), ("desc", Order::Desc)] {
            let mut cursor: Option<String> = None;
            let (mut seen, mut pages) = (Vec::new(), 0);
            loop {
                let query = PageQuery::new(
                    Some("2".to_owned()),
                    cursor.clone(),
                    Some(direction.to_owned()),
                );
                let request = PageRequest::try_from(query).unwrap();
                let page = fetch_page_tiers(
                    item::Entity::find(),
                    &tiers,
                    item::Column::Id,
                    |row| row.id,
                    &request,
                    db.read(),
                )
                .await
                .unwrap();
                pages += 1;
                seen.extend(page.data.iter().map(|row| row.id));
                match page.pagination.next_cursor {
                    Some(next) => cursor = Some(next),
                    None => break,
                }
            }
            let tier = |kind: &str| {
                let mut found: Vec<Uuid> =
                    ids.iter().filter(|(k, _)| *k == kind).map(|(_, id)| *id).collect();
                if order == Order::Desc {
                    found.reverse();
                }
                found
            };
            let expected: Vec<Uuid> = tier("b").into_iter().chain(tier("a")).collect();
            assert_eq!(seen, expected, "{direction}");
            assert_eq!(pages, 3, "6 rows, 2 per page, no empty last page");
        }

        // Con trỏ theo ID không dùng được cho danh sách theo tầng.
        let request = PageRequest::first(2, Order::Asc).after(ids[0].1);
        let error = fetch_page_tiers(
            item::Entity::find(),
            &tiers,
            item::Column::Id,
            |row| row.id,
            &request,
            db.read(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.fields()[0].field, "cursor");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn count_tra_tong_so_khop_bo_loc_o_trang_dau() {
    use autoxide_kernel::pagination::{PageQuery, SortValue, fetch_page_by, fetch_page_tiers};
    use sea_orm::Condition;

    TestDatabase::run(async |database| {
        let db = connect(database).await;
        insert_items(&db, 7).await;
        let only_a = || item::Entity::find().filter(item::Column::Kind.eq("a"));
        let request = PageRequest::try_from(
            PageQuery::new(Some("2".to_owned()), None, None).with_count(true),
        )
        .unwrap();

        let first = fetch_page(only_a(), item::Column::Id, |row| row.id, &request, db.read())
            .await
            .unwrap();
        assert_eq!(first.data.len(), 2);
        assert_eq!(first.pagination.total, Some(4));
        // Trang sau không đếm lại.
        let cursor = first.pagination.next_cursor.unwrap();
        let next = PageQuery::new(Some("2".to_owned()), Some(cursor), None).with_count(true);
        let next = PageRequest::try_from(next).unwrap();
        let second =
            fetch_page(only_a(), item::Column::Id, |row| row.id, &next, db.read()).await.unwrap();
        assert_eq!(second.pagination.total, None);
        // Không hỏi thì không đếm.
        let plain = PageRequest::first(2, Order::Desc);
        let page =
            fetch_page(only_a(), item::Column::Id, |row| row.id, &plain, db.read()).await.unwrap();
        assert_eq!(page.pagination.total, None);

        let sorted = fetch_page_by(
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
        assert_eq!(sorted.pagination.total, Some(7));

        let tiers = [Condition::all().add(item::Column::Kind.eq("b"))];
        let tiered = fetch_page_tiers(
            item::Entity::find(),
            &tiers,
            item::Column::Id,
            |row| row.id,
            &request,
            db.read(),
        )
        .await
        .unwrap();
        assert_eq!(tiered.pagination.total, Some(3));
    })
    .await
    .unwrap();
}

/// `search::starts_with` / `search::contains` thoát `%` và `_`: chuỗi người dùng đặt không thành ký
/// tự đại diện của `LIKE` (slug `a_b` không khớp `axb-1`), trên cả SQLite và Postgres.
#[tokio::test]
async fn like_thoat_ky_tu_dai_dien() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        for kind in ["a_b-1", "axb-1", "a%b-1", "azzb-1", r"a\b-1"] {
            item::ActiveModel { id: Set(Uuid::now_v7()), kind: Set(kind.to_owned()) }
                .insert(db.write())
                .await
                .unwrap();
        }
        let kinds = async |condition| {
            let mut found: Vec<String> = item::Entity::find()
                .filter(condition)
                .all(db.read())
                .await
                .unwrap()
                .into_iter()
                .map(|row| row.kind)
                .collect();
            found.sort();
            found
        };
        use autoxide_kernel::search;
        assert_eq!(kinds(search::starts_with(item::Column::Kind, "a_b-")).await, ["a_b-1"]);
        assert_eq!(kinds(search::starts_with(item::Column::Kind, "a%b")).await, ["a%b-1"]);
        assert_eq!(kinds(search::starts_with(item::Column::Kind, r"a\b")).await, [r"a\b-1"]);
        assert_eq!(kinds(search::contains(item::Column::Kind, "_b-")).await, ["a_b-1"]);
        // Không thoát (cách cũ) thì `_` khớp mọi ký tự: cả `axb-1` lẫn `a%b-1`.
        assert_eq!(kinds(item::Column::Kind.starts_with("a_b-")).await.len(), 4);
    })
    .await
    .unwrap();
}

/// Cột sắp được phép trống (`Option<T>` trong entity) là lỗi lập trình: từ chối trước khi truy vấn, kể cả
/// trang đầu, thay vì làm mất trong im lặng các dòng NULL ở trang sau (D-20261125-list-pages).
#[tokio::test]
async fn cot_sap_duoc_phep_trong_bi_tu_choi() {
    use autoxide_kernel::error::ErrorKind;
    use autoxide_kernel::pagination::{SortValue, fetch_page_by};

    TestDatabase::run(async |database| {
        let db = connect(database).await;
        // Không tạo bảng `nullable_item`: lời từ chối phải đến trước mọi câu SQL.
        let error = fetch_page_by(
            nullable::Entity::find(),
            nullable::Column::Note,
            |row| SortValue::Text(row.note.clone().unwrap_or_default()),
            nullable::Column::Id,
            |row| row.id,
            &PageRequest::first(10, Order::Asc),
            db.read(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Internal);
        assert!(error.to_string().contains("`note`"), "{error}");
        db.close().await.unwrap();
    })
    .await
    .unwrap();
}
