//! Xác nhận các thư viện nền đã ghim ở `Cargo.toml` gốc hợp nhau, trên database
//! đang chọn (gate chạy file này trên cả SQLite và Postgres).
//!
//! Đây không phải code mẫu cho module — nó cố ý nhét mọi thứ vào một file để
//! kiểm đúng một điều: bộ phiên bản đang ghim biên dịch được cùng nhau và cho
//! cùng một kết quả trên hai database. Khi nâng phiên bản thư viện, file này là
//! chỗ hỏng đầu tiên và nói rõ thư viện nào lệch.
//!
//! Những điều phần sau của core dựa vào và được kiểm ở đây:
//! - migration viết một lần chạy được trên cả hai database, có `up` và `down`;
//! - UUIDv7 làm khoá chính, sắp theo ID = sắp theo thứ tự tạo (pagination);
//! - thời gian UTC và số nguyên 64-bit (tiền) đi một vòng không sai lệch;
//! - vi phạm unique được báo bằng CÙNG một kiểu lỗi trên hai database;
//! - route và tài liệu OpenAPI sinh ra từ cùng một khai báo.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
// Clippy chỉ tự miễn cho hàm `#[test]`, không miễn cho hàm phụ, nên khai ở đây.
// Trong `src/` thì KHÔNG được khai dòng này (`cargo xtask check` chặn).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::time::Timestamp;
use autoxide_testkit::TestDatabase;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode, header};
use axum::{Json, Router};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, QueryOrder, Set, SqlErr};
use serde::{Deserialize, Serialize};
use tower::ServiceExt;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use utoipa_scalar::{Scalar, Servable};
use uuid::Uuid;
use validator::Validate;

/// Migration tạo bảng `stack_probe`.
mod migration {
    use autoxide_kernel::migration::prelude::*;
    use autoxide_kernel::migration::schema::{
        big_integer, pk_uuid, string, timestamp_with_time_zone,
    };

    pub struct CreateProbe;

    impl MigrationName for CreateProbe {
        fn name(&self) -> &str {
            "m20261111_000001_create_stack_probe"
        }
    }

    // `async fn` trong trait dùng qua `dyn` (ở đây `Box<dyn MigrationTrait>`) chưa
    // được Rust hỗ trợ trực tiếp; macro `async_trait` của thư viện lo việc đó.
    #[async_trait::async_trait]
    impl MigrationTrait for CreateProbe {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .create_table(
                    Table::create()
                        .table("stack_probe")
                        .col(pk_uuid("id"))
                        .col(string("name"))
                        .col(big_integer("amount_minor"))
                        .col(timestamp_with_time_zone("created_at"))
                        .to_owned(),
                )
                .await?;
            manager
                .create_index(
                    Index::create()
                        .name("stack_probe_name_unique")
                        .table("stack_probe")
                        .col("name")
                        .unique()
                        .to_owned(),
                )
                .await
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager.drop_table(Table::drop().table("stack_probe").to_owned()).await
        }
    }

    /// Chạy `up` / `down` của migration thử thẳng trên kết nối (không qua bảng lịch sử: file này chỉ kiểm thư
    /// viện nền; bộ chạy migration có test riêng ở `migration.rs`).
    pub struct Migrator;

    impl Migrator {
        pub async fn up(
            db: &sea_orm::DatabaseConnection,
            _steps: Option<u32>,
        ) -> Result<(), DbErr> {
            CreateProbe.up(&SchemaManager::new(db)).await
        }

        pub async fn down(
            db: &sea_orm::DatabaseConnection,
            _steps: Option<u32>,
        ) -> Result<(), DbErr> {
            CreateProbe.down(&SchemaManager::new(db)).await
        }
    }
}

/// Entity của bảng `stack_probe`.
mod probe {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "stack_probe")]
    pub struct Model {
        // ID sinh trong ứng dụng (UUIDv7), không để database tự tăng.
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        #[sea_orm(unique)]
        pub name: String,
        pub amount_minor: i64,
        pub created_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

async fn insert_probe(
    db: &DatabaseConnection,
    name: &str,
    amount_minor: i64,
    created_at: Timestamp,
) -> Result<probe::Model, sea_orm::DbErr> {
    probe::ActiveModel {
        id: Set(Uuid::now_v7()),
        name: Set(name.to_owned()),
        amount_minor: Set(amount_minor),
        created_at: Set(created_at),
    }
    .insert(db)
    .await
}

/// Postgres lưu thời gian tới micro giây; cắt bớt để so sánh được sau một vòng.
fn now_in_database_precision() -> Timestamp {
    Timestamp::now()
}

#[tokio::test]
async fn migration_va_entity_cho_cung_ket_qua_tren_database_dang_chon() {
    TestDatabase::run(async |database| {
        let db = database.connect().await.unwrap();
        migration::Migrator::up(&db, None).await.unwrap();

        let created_at = now_in_database_precision();
        let mut inserted_ids = Vec::new();
        for (name, amount_minor) in [("a", 0), ("b", i64::MAX), ("c", -1)] {
            let model = insert_probe(&db, name, amount_minor, created_at).await.unwrap();
            inserted_ids.push(model.id);
        }

        let rows = probe::Entity::find().order_by_asc(probe::Column::Id).all(&db).await.unwrap();

        // Sắp theo ID trả đúng thứ tự tạo: nền của pagination theo UUIDv7.
        assert_eq!(rows.iter().map(|row| row.id).collect::<Vec<_>>(), inserted_ids);
        assert_eq!(rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
        // Số nguyên 64-bit (tiền theo đơn vị nhỏ nhất) không bị làm tròn.
        assert_eq!(rows.iter().map(|row| row.amount_minor).collect::<Vec<_>>(), [0, i64::MAX, -1]);
        // Thời gian UTC đi một vòng không lệch.
        assert!(rows.iter().all(|row| row.created_at == created_at));

        // Vi phạm unique: cả hai database phải báo cùng một kiểu lỗi, vì các phần
        // sau (idempotency, đăng ký trùng email) rẽ nhánh theo nó.
        let duplicate = insert_probe(&db, "a", 1, created_at).await.unwrap_err();
        assert!(
            matches!(duplicate.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))),
            "{duplicate:?}"
        );

        // `down` gỡ được những gì `up` đã tạo.
        migration::Migrator::down(&db, None).await.unwrap();
        assert!(probe::Entity::find().all(&db).await.is_err());
    })
    .await
    .unwrap();
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
struct CreateProbe {
    #[validate(length(min = 1, max = 50))]
    name: String,
    #[validate(range(min = 0))]
    amount_minor: i64,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
struct ProbeBody {
    id: Uuid,
    name: String,
    amount_minor: i64,
    created_at: Timestamp,
}

/// Create a probe row.
#[utoipa::path(
    post,
    path = "/probes",
    request_body = CreateProbe,
    responses(
        (status = 201, description = "The probe was created", body = ProbeBody),
        (status = 422, description = "The input failed validation"),
    )
)]
async fn create_probe(
    State(db): State<DatabaseConnection>,
    Json(input): Json<CreateProbe>,
) -> Result<(StatusCode, Json<ProbeBody>), StatusCode> {
    input.validate().map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    let model = insert_probe(&db, &input.name, input.amount_minor, now_in_database_precision())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let body = ProbeBody {
        id: model.id,
        name: model.name,
        amount_minor: model.amount_minor,
        created_at: model.created_at,
    };
    Ok((StatusCode::CREATED, Json(body)))
}

/// Router + tài liệu OpenAPI sinh từ cùng một lời gọi `routes!`.
fn app(db: DatabaseConnection) -> (Router, utoipa::openapi::OpenApi) {
    let (router, openapi) =
        OpenApiRouter::new().routes(routes!(create_probe)).with_state(db).split_for_parts();
    let router = router
        .merge(Scalar::with_url("/docs", openapi.clone()))
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(RequestBodyLimitLayer::new(16 * 1024))
        .layer(CorsLayer::new());
    (router, openapi)
}

fn post_json(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn body_bytes(response: axum::response::Response) -> Vec<u8> {
    axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap().to_vec()
}

#[tokio::test]
async fn http_validate_va_openapi_chay_cung_nhau() {
    TestDatabase::run(async |database| {
        let db = database.connect().await.unwrap();
        migration::Migrator::up(&db, None).await.unwrap();
        let (router, openapi) = app(db);

        // Request hợp lệ: 201, body JSON đọc lại được, có request id.
        // `clone()` vì `oneshot` tiêu thụ router; Router clone rẻ (chỉ tăng bộ đếm).
        let response = router
            .clone()
            .oneshot(post_json("/probes", serde_json::json!({"name": "a", "amount_minor": 1500})))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        assert!(response.headers().contains_key("x-request-id"));
        let created: ProbeBody = serde_json::from_slice(&body_bytes(response).await).unwrap();
        assert_eq!((created.name.as_str(), created.amount_minor), ("a", 1500));
        assert_eq!(created.id.get_version_num(), 7);

        // Request sai: validator chặn trước khi chạm database.
        let response = router
            .clone()
            .oneshot(post_json("/probes", serde_json::json!({"name": "", "amount_minor": -1})))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

        // Trang xem tài liệu API.
        let response = router
            .oneshot(Request::builder().uri("/docs").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let html = String::from_utf8(body_bytes(response).await).unwrap();
        assert!(html.contains("/probes"), "docs page does not embed the OpenAPI document");

        // Tài liệu OpenAPI mô tả đúng route vừa gọi và kiểu của từng trường.
        let document = serde_json::to_value(&openapi).unwrap();
        let operation = &document["paths"]["/probes"]["post"];
        assert_eq!(operation["summary"], "Create a probe row.");
        assert!(operation["responses"]["201"].is_object(), "{operation}");
        let properties = &document["components"]["schemas"]["ProbeBody"]["properties"];
        assert_eq!(properties["id"]["format"], "uuid");
        assert_eq!(properties["created_at"]["format"], "date-time");
        assert_eq!(properties["amount_minor"]["format"], "int64");
        let input = &document["components"]["schemas"]["CreateProbe"]["properties"];
        assert_eq!(input["name"]["type"], "string");
    })
    .await
    .unwrap();
}

#[test]
fn uuid_v7_tang_dan_theo_thu_tu_sinh() {
    // Sinh liên tiếp trong cùng một mili giây vẫn phải tăng dần: pagination và
    // thứ tự event dựa vào điều này.
    let ids: Vec<Uuid> = (0..1000).map(|_| Uuid::now_v7()).collect();
    assert!(ids.iter().all(|id| id.get_version_num() == 7));
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
}
