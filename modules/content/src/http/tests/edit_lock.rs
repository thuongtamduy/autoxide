//! Khoá sửa trang (bước 67): giữ 2 phút, người khác thấy ai đang sửa, chiếm lại khoá hết hạn, nhả;
//! hai người cùng chiếm thì chỉ một được. `If-Match` vẫn là chốt cuối. Gate chạy trên cả SQLite và
//! Postgres.

use std::sync::Arc;

use autoxide_kernel::ai::{Embedder, TextModel};
use autoxide_kernel::time::{SignedDuration, Timestamp};
use autoxide_media::api::Media;
use autoxide_testkit::ai::{FakeEmbedder, FakeTextModel};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{StatusCode, header};
use sea_orm::EntityTrait;
use sea_orm::sea_query::Expr;
use serde_json::json;

use super::send;
use crate::entity::edit_lock;
use crate::testing::FakeMedia;

const EDITORS: [&str; 6] = ["ann", "bob", "cid", "dan", "eve", "fay"];

/// Sáu người cùng quyền sửa, một người chỉ đọc.
async fn app(test: &TestDatabase) -> TestApp {
    let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
    let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
    let mut builder = TestApp::builder(test);
    for (index, name) in EDITORS.iter().enumerate() {
        builder = builder.user(name, 10 + index as u128, &["content.read", "content.write"]);
    }
    builder
        .user("reader", 30, &["content.read"])
        .provide::<dyn Media>(media)
        .provide::<dyn TextModel>(text)
        .provide::<dyn Embedder>(embedder)
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

async fn page(app: &TestApp) -> String {
    let created = send(
        app,
        "POST",
        "/entries",
        Some("ann"),
        &[],
        Some(json!({"kind": "page", "title": "Hè"})),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
    created.json()["data"]["id"].as_str().unwrap().to_owned()
}

/// Đẩy mọi khoá về quá khứ, như thể đã qua 2 phút.
async fn expire(app: &TestApp) {
    let past = Timestamp::now() - SignedDuration::from_secs(1);
    // tenant-scope: test đẩy mọi khoá của database test về quá khứ.
    edit_lock::Entity::update_many()
        .col_expr(edit_lock::Column::ExpiresAt, Expr::value(past))
        .exec(app.database().write())
        .await
        .unwrap();
}

#[tokio::test]
async fn khoa_sua_giu_gia_han_nha_va_chiem_lai_khi_het_han() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let id = page(&app).await;
        let uri = format!("/entries/{id}/edit-lock");

        let none = send(&app, "GET", &uri, Some("reader"), &[], None).await;
        assert_eq!((none.status(), none.json()["data"].clone()), (StatusCode::OK, json!(null)));

        let taken = send(&app, "POST", &uri, Some("ann"), &[], None).await;
        assert_eq!(taken.status(), StatusCode::OK, "{}", taken.json());
        let lock = taken.json()["data"].clone();
        assert_eq!(
            (lock["mine"].as_bool(), lock["entry_id"].as_str()),
            (Some(true), Some(id.as_str()))
        );

        // Người khác thấy ai đang sửa, tới khi nào; chiếm hay nhả đều 409.
        let seen = send(&app, "GET", &uri, Some("reader"), &[], None).await.json();
        assert_eq!(
            (seen["data"]["user_id"].clone(), seen["data"]["mine"].as_bool()),
            (lock["user_id"].clone(), Some(false))
        );
        let refused = send(&app, "POST", &uri, Some("bob"), &[], None).await;
        assert_eq!(
            (refused.status(), refused.json()["code"].as_str()),
            (StatusCode::CONFLICT, Some("content.edit_locked"))
        );
        assert!(
            refused.json()["detail"]
                .as_str()
                .unwrap()
                .contains(&lock["user_id"].as_str().unwrap().to_owned()),
            "{}",
            refused.json()
        );
        let not_yours = send(&app, "DELETE", &uri, Some("bob"), &[], None).await;
        assert_eq!(not_yours.status(), StatusCode::CONFLICT);
        assert_eq!(
            send(&app, "POST", &uri, Some("reader"), &[], None).await.status(),
            StatusCode::FORBIDDEN
        );

        // Gia hạn giữ lúc chiếm đầu tiên, dời giờ hết hạn.
        let renewed = send(&app, "POST", &uri, Some("ann"), &[], None).await.json();
        assert_eq!(renewed["data"]["acquired_at"], lock["acquired_at"]);
        assert!(renewed["data"]["expires_at"].as_str() >= lock["expires_at"].as_str());

        // Khoá chỉ để báo: người khác vẫn ghi được nếu `If-Match` khớp.
        let read = send(&app, "GET", &format!("/entries/{id}"), Some("bob"), &[], None).await;
        let etag = read.header(header::ETAG).to_owned();
        let saved = send(
            &app,
            "PATCH",
            &format!("/entries/{id}"),
            Some("bob"),
            &[("if-match", &etag)],
            Some(json!({"excerpt": "x"})),
        )
        .await;
        assert_eq!(saved.status(), StatusCode::OK, "{}", saved.json());

        // Hết hạn: người khác chiếm được, khoá mới tính lại lúc chiếm.
        expire(&app).await;
        assert_eq!(
            send(&app, "GET", &uri, Some("reader"), &[], None).await.json()["data"],
            json!(null)
        );
        let took_over = send(&app, "POST", &uri, Some("bob"), &[], None).await;
        assert_eq!(took_over.status(), StatusCode::OK, "{}", took_over.json());
        assert_ne!(took_over.json()["data"]["acquired_at"], lock["acquired_at"]);
        assert_ne!(took_over.json()["data"]["user_id"], lock["user_id"]);

        // Nhả rồi nhả lại: không lỗi; người khác chiếm được ngay.
        assert_eq!(
            send(&app, "DELETE", &uri, Some("bob"), &[], None).await.status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            send(&app, "DELETE", &uri, Some("bob"), &[], None).await.status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(send(&app, "POST", &uri, Some("ann"), &[], None).await.status(), StatusCode::OK);

        let missing = send(
            &app,
            "POST",
            &format!("/entries/{}/edit-lock", uuid::Uuid::now_v7()),
            Some("ann"),
            &[],
            None,
        )
        .await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_nguoi_cung_giu_khoa_chi_mot_duoc() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        for round in 0..3 {
            let id = page(&app).await;
            let uri = format!("/entries/{id}/edit-lock");
            if round == 2 {
                // Khoá cũ đã hết hạn: các bên cùng chiếm lại, vẫn chỉ một được.
                assert_eq!(
                    send(&app, "POST", &uri, Some("ann"), &[], None).await.status(),
                    StatusCode::OK
                );
                expire(&app).await;
            }
            let attempts = EDITORS.map(|who| send(&app, "POST", &uri, Some(who), &[], None));
            let answers = futures_join(attempts).await;
            let won: Vec<_> = answers.iter().filter(|a| a.status() == StatusCode::OK).collect();
            assert_eq!(
                won.len(),
                1,
                "round {round}: {:?}",
                answers.iter().map(|a| a.status()).collect::<Vec<_>>()
            );
            for answer in &answers {
                assert!(
                    matches!(answer.status(), StatusCode::OK | StatusCode::CONFLICT),
                    "{}",
                    answer.json()
                );
            }
            let holder = send(&app, "GET", &uri, Some("reader"), &[], None).await.json();
            assert_eq!(holder["data"]["user_id"], won[0].json()["data"]["user_id"]);
        }
    })
    .await
    .unwrap();
}

/// Chạy mọi request cùng lúc (xen nhau trên cùng một task).
async fn futures_join<F: std::future::Future>(futures: [F; 6]) -> Vec<F::Output> {
    let [a, b, c, d, e, f] = futures;
    let (a, b, c, d, e, f) = tokio::join!(a, b, c, d, e, f);
    vec![a, b, c, d, e, f]
}
