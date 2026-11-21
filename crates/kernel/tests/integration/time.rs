//! Kiểu thời gian mới (`autoxide_kernel::time`, D-20261121-time-library) trên database đang chọn: đi một vòng
//! không lệch, đọc được dòng `chrono` đã ghi (và ngược lại), sắp xếp và lọc đúng khi dòng cũ và mới trộn nhau.

use autoxide_kernel::migration::prelude::*;
use autoxide_kernel::time::{Date, Timestamp};
use autoxide_testkit::TestDatabase;
use chrono::{DateTime, NaiveDate, Utc};
use jiff::SignedDuration;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseBackend, DatabaseConnection,
    EntityTrait, QueryFilter, QueryOrder, Set,
};
use uuid::Uuid;

/// Bảng thử, đọc bằng kiểu mới.
mod probe {
    use autoxide_kernel::time::{Date, Timestamp};
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "time_probe")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub at: Timestamp,
        pub maybe_at: Option<Timestamp>,
        pub day: Date,
        pub maybe_day: Option<Date>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// Cùng bảng, đọc / ghi bằng `chrono` như code cũ.
mod legacy {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "time_probe")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub at: DateTimeUtc,
        pub maybe_at: Option<DateTimeUtc>,
        pub day: Date,
        pub maybe_day: Option<Date>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

async fn create_table(db: &DatabaseConnection) {
    SchemaManager::new(db)
        .create_table(
            Table::create()
                .table("time_probe")
                .col(schema::pk_uuid("id"))
                .col(schema::timestamp_with_time_zone("at"))
                .col(schema::timestamp_with_time_zone_null("maybe_at"))
                .col(schema::date("day"))
                .col(schema::date_null("maybe_day"))
                .to_owned(),
        )
        .await
        .unwrap();
}

fn ts(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn day(text: &str) -> Date {
    text.parse().unwrap()
}

async fn insert(
    db: &DatabaseConnection,
    at: Timestamp,
    maybe_at: Option<Timestamp>,
    day: Date,
) -> Uuid {
    let id = Uuid::now_v7();
    probe::ActiveModel {
        id: Set(id),
        at: Set(at),
        maybe_at: Set(maybe_at),
        day: Set(day),
        maybe_day: Set(None),
    }
    .insert(db)
    .await
    .unwrap();
    id
}

async fn insert_legacy(db: &DatabaseConnection, at: DateTime<Utc>, day: NaiveDate) -> Uuid {
    let id = Uuid::now_v7();
    legacy::ActiveModel {
        id: Set(id),
        at: Set(at),
        maybe_at: Set(Some(at)),
        day: Set(day),
        maybe_day: Set(Some(day)),
    }
    .insert(db)
    .await
    .unwrap();
    id
}

/// Chuỗi đang nằm trên đĩa (SQLite) hay dạng chữ chuẩn của Postgres — để so hai cách ghi cho cùng kết quả.
async fn stored_text(db: &DatabaseConnection, id: Uuid) -> String {
    let backend = db.get_database_backend();
    let sql = match backend {
        DatabaseBackend::Postgres => "SELECT CAST(at AS TEXT) AS at FROM time_probe WHERE id = $1",
        _ => "SELECT at FROM time_probe WHERE id = ?",
    };
    let row = db
        .query_one_raw(autoxide_kernel::database::statement(backend, sql, [id.into()]))
        .await
        .unwrap()
        .unwrap();
    row.try_get("", "at").unwrap()
}

#[tokio::test]
async fn di_mot_vong_khong_lech_va_null() {
    TestDatabase::run(async |database| {
        let db = database.connect().await.unwrap();
        create_table(&db).await;

        let now = Timestamp::now();
        let id = insert(&db, now, None, day("2024-02-29")).await;
        let row = probe::Entity::find_by_id(id).one(&db).await.unwrap().unwrap();
        assert_eq!(row.at, now);
        assert_eq!(row.maybe_at, None);
        assert_eq!(row.day, day("2024-02-29"));
        assert_eq!(row.maybe_day, None);

        // Sửa qua ActiveModel: giá trị đi qua `ValueType::try_from` của SeaORM.
        let mut active: probe::ActiveModel = row.into();
        active.maybe_at = Set(Some(now + SignedDuration::from_hours(1)));
        active.maybe_day = Set(Some(day("2026-11-21")));
        let updated = active.update(&db).await.unwrap();
        let again = probe::Entity::find_by_id(id).one(&db).await.unwrap().unwrap();
        assert_eq!(again, updated);
        assert_eq!(again.maybe_at, Some(now + SignedDuration::from_hours(1)));

        // Lọc theo NULL và theo giá trị.
        let nulls =
            probe::Entity::find().filter(probe::Column::MaybeAt.is_null()).all(&db).await.unwrap();
        assert!(nulls.is_empty());
        let matched =
            probe::Entity::find().filter(probe::Column::At.eq(now)).all(&db).await.unwrap();
        assert_eq!(matched.len(), 1);
        let by_day = probe::Entity::find()
            .filter(probe::Column::Day.eq(day("2024-02-29")))
            .all(&db)
            .await
            .unwrap();
        assert_eq!(by_day.len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dong_chrono_va_dong_moi_doc_cheo_va_luu_giong_nhau() {
    TestDatabase::run(async |database| {
        let db = database.connect().await.unwrap();
        create_table(&db).await;

        // Mỗi giá trị có độ chính xác micro giây (điều Postgres giữ được) với số chữ số lẻ khác nhau.
        for text in
            ["2026-11-21T10:00:00Z", "2026-11-21T10:00:00.120Z", "2026-11-21T10:00:00.123456Z"]
        {
            let ours = ts(text);
            let theirs: DateTime<Utc> = ours.into();
            let chrono_day = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();

            let new_id = insert(&db, ours, Some(ours), Date::from(chrono_day)).await;
            let old_id = insert_legacy(&db, theirs, chrono_day).await;

            // Hai cách ghi để lại cùng một chuỗi (SQLite) / cùng một giá trị (Postgres).
            assert_eq!(stored_text(&db, new_id).await, stored_text(&db, old_id).await, "{text}");

            // Kiểu mới đọc dòng `chrono` đã ghi; `chrono` đọc dòng kiểu mới đã ghi.
            let old_row = probe::Entity::find_by_id(old_id).one(&db).await.unwrap().unwrap();
            assert_eq!(
                (old_row.at, old_row.maybe_at, old_row.day),
                (ours, Some(ours), Date::from(chrono_day))
            );
            let new_row = legacy::Entity::find_by_id(new_id).one(&db).await.unwrap().unwrap();
            assert_eq!(
                (new_row.at, new_row.maybe_at, new_row.day),
                (theirs, Some(theirs), chrono_day)
            );
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn sap_xep_va_loc_dung_khi_tron_dong_cu_dong_moi() {
    TestDatabase::run(async |database| {
        let db = database.connect().await.unwrap();
        create_table(&db).await;

        // Cố ý chọn những giá trị mà chuỗi cắt số 0 (`.12Z`, `.125Z`) sẽ sắp sai.
        let texts = [
            "2026-11-21T10:00:00.125Z",
            "2026-11-21T10:00:00Z",
            "2026-11-21T10:00:00.120Z",
            "2026-11-21T10:00:00.120001Z",
            "2026-11-21T09:59:59.999999Z",
            "2026-11-21T10:00:01Z",
        ];
        for (index, text) in texts.iter().enumerate() {
            let value = ts(text);
            // Xen kẽ: dòng chẵn ghi bằng kiểu mới, dòng lẻ bằng `chrono` (dữ liệu cũ).
            if index % 2 == 0 {
                insert(&db, value, None, day("2026-11-21")).await;
            } else {
                insert_legacy(&db, value.into(), NaiveDate::from_ymd_opt(2026, 11, 21).unwrap())
                    .await;
            }
        }
        let mut expected: Vec<Timestamp> = texts.iter().map(|text| ts(text)).collect();
        expected.sort();

        let ordered: Vec<Timestamp> = probe::Entity::find()
            .order_by_asc(probe::Column::At)
            .all(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.at)
            .collect();
        assert_eq!(ordered, expected);

        let pivot = ts("2026-11-21T10:00:00.120Z");
        let after: Vec<Timestamp> = probe::Entity::find()
            .filter(probe::Column::At.gt(pivot))
            .order_by_asc(probe::Column::At)
            .all(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.at)
            .collect();
        assert_eq!(
            after,
            expected.iter().copied().filter(|value| *value > pivot).collect::<Vec<_>>()
        );

        let between = probe::Entity::find()
            .filter(
                probe::Column::At
                    .between(ts("2026-11-21T10:00:00Z"), ts("2026-11-21T10:00:00.125Z")),
            )
            .all(&db)
            .await
            .unwrap();
        assert_eq!(between.len(), 4);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doc_gia_tri_database_tu_ghi() {
    TestDatabase::run(async |database| {
        let db = database.connect().await.unwrap();
        create_table(&db).await;
        let id = insert(&db, ts("2000-01-01T00:00:00Z"), None, day("2000-01-01")).await;

        // `CURRENT_TIMESTAMP` / `CURRENT_DATE`: SQLite ghi `YYYY-MM-DD HH:MM:SS` (không độ lệch), Postgres ghi
        // `timestamptz` — cả hai phải đọc ra đúng giờ UTC hiện tại.
        let backend = db.get_database_backend();
        let sql = match backend {
            DatabaseBackend::Postgres => {
                "UPDATE time_probe SET at = CURRENT_TIMESTAMP, day = CURRENT_DATE WHERE id = $1"
            }
            _ => "UPDATE time_probe SET at = CURRENT_TIMESTAMP, day = CURRENT_DATE WHERE id = ?",
        };
        db.execute_raw(autoxide_kernel::database::statement(backend, sql, [id.into()]))
            .await
            .unwrap();

        let row = probe::Entity::find_by_id(id).one(&db).await.unwrap().unwrap();
        let drift = (Timestamp::now() - row.at).abs();
        assert!(drift < SignedDuration::from_secs(60), "{row:?}");
        assert!(
            row.day
                >= Timestamp::now()
                    .checked_sub(SignedDuration::from_hours(24))
                    .unwrap()
                    .date_in(&jiff::tz::TimeZone::UTC)
        );
    })
    .await
    .unwrap();
}

/// Câu SQL viết tay qua `autoxide_kernel::database::statement` ghi / lọc được cột thời gian trên cả hai database,
/// và lưu đúng chuỗi như builder (Postgres từ chối nếu tham số thiếu tên kiểu).
#[tokio::test]
async fn cau_viet_tay_ghi_va_loc_duoc_cot_thoi_gian() {
    use autoxide_kernel::database::statement;
    TestDatabase::run(async |database| {
        let db = database.connect().await.unwrap();
        create_table(&db).await;
        let backend = db.get_database_backend();
        let id = Uuid::now_v7();
        let at = ts("2026-11-21T10:00:00.123456Z");
        // `$n` chạy được trên cả hai: SQLite hiểu `$1` là tham số theo vị trí.
        db.execute_raw(statement(
            backend,
            "INSERT INTO time_probe (id, at, maybe_at, day, maybe_day) VALUES ($1, $2, $3, $4, $5)",
            [
                id.into(),
                at.into(),
                None::<Timestamp>.into(),
                day("2026-11-21").into(),
                None::<Date>.into(),
            ],
        ))
        .await
        .unwrap();
        let reference = insert(&db, at, None, day("2026-11-21")).await;
        assert_eq!(stored_text(&db, id).await, stored_text(&db, reference).await);
        db.execute_raw(statement(
            backend,
            "UPDATE time_probe SET maybe_at = $1, maybe_day = $2 WHERE id = $3",
            [at.into(), day("2026-11-22").into(), id.into()],
        ))
        .await
        .unwrap();
        let row = probe::Entity::find_by_id(id).one(&db).await.unwrap().unwrap();
        assert_eq!(
            (row.at, row.maybe_at, row.day, row.maybe_day),
            (at, Some(at), day("2026-11-21"), Some(day("2026-11-22")))
        );
        let found = db
            .query_one_raw(statement(
                backend,
                "SELECT COUNT(*) AS n FROM time_probe WHERE at >= $1 AND day = $2",
                [at.into(), day("2026-11-21").into()],
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.try_get::<i64>("", "n").unwrap(), 2);
    })
    .await
    .unwrap();
}
