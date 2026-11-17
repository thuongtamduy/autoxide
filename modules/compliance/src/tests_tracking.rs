//! Test mã theo dõi quảng cáo và đồng ý cookie (D-235, D-361) trên cả SQLite và Postgres: mã ID sai
//! dạng và nhà cung cấp lạ bị từ chối, chưa đồng ý thì không trả mã nào ở nhóm chưa chọn, đổi nhóm /
//! nhà cung cấp tăng phiên bản chính sách, bằng chứng đồng ý không lưu IP thô, người đăng nhập ghi vào
//! sổ đồng ý chung, nhà cung cấp đang bật hiện trong danh sách bên nhận dữ liệu, câu `legal` không đổi
//! được, `csp_hosts` chỉ gồm nhà cung cấp đang bật.

use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use sea_orm::EntityTrait;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{ConsentKey, ConsentLedger, ConsentSubject};
use crate::entity::cookie_consent;
use crate::tests::call;

async fn app(test: &TestDatabase) -> TestApp {
    TestApp::builder(test)
        .user("admin", 1, &["compliance.tracking.manage", "compliance.privacy.read"])
        .user("alice", 2, &[])
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

fn root() -> Uuid {
    root_of(DEFAULT_TENANT).as_uuid()
}

async fn config(app: &TestApp, granted: &str) -> Value {
    let uri = format!("/api/v1/compliance/public/consent-config?granted={granted}&locale=vi");
    let answer = app.get(&uri).send().await.unwrap();
    assert_eq!(answer.status(), StatusCode::OK);
    answer.json()
}

async fn add(app: &TestApp, provider: &str, id: &str) -> (StatusCode, Value) {
    call(
        app,
        "POST",
        "/tracking-tags",
        "admin",
        Some(json!({"provider": provider, "tracking_id": id})),
    )
    .await
}

async fn consent(
    app: &TestApp,
    who: Option<&str>,
    categories: &[&str],
    version: i64,
) -> StatusCode {
    let body = json!({"consent_id": "abcdefghijklmnop1234", "categories": categories, "policy_version": version});
    let mut request = app.post("/api/v1/compliance/public/cookie-consents").json(body);
    if let Some(who) = who {
        request = request.token(who);
    }
    request.send().await.unwrap().status()
}

#[tokio::test]
async fn ma_id_sai_dang_va_nha_cung_cap_la_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (status, _) = add(&app, "ga4", "G-ABC12345").await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, problem) = call(
            &app,
            "POST",
            "/tracking-tags",
            "alice",
            Some(json!({"provider": "ga4", "tracking_id": "G-1"})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
        let (status, problem) = add(&app, "ga4", "UA-1234-1").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem["code"], "compliance.tracking_id_invalid");
        assert!(problem["detail"].as_str().unwrap().contains("G-XXXXXXXXXX"), "{problem}");
        let (status, problem) = add(&app, "hotjar", "123").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem["code"], "compliance.tracking_provider_unknown");
        let (status, problem) = add(&app, "ga4", "g-abc12345").await;
        assert_eq!(status, StatusCode::CONFLICT, "{problem}");
        let (status, providers) = call(&app, "GET", "/tracking-providers", "admin", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(providers["data"].as_array().unwrap().len(), 6);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chua_dong_y_khong_tra_ma_nao_o_nhom_chua_chon() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        // Chưa có gì: không banner, không mã.
        let empty = config(&app, "").await;
        assert_eq!(empty["data"]["enabled"], false);
        assert!(empty["data"]["tags"].as_array().unwrap().is_empty());

        let (status, ga) = add(&app, "ga4", "G-ABC12345").await;
        assert_eq!(status, StatusCode::CREATED, "{ga}");
        let (status, _) = add(&app, "meta", "1234567890123456").await;
        assert_eq!(status, StatusCode::CREATED);

        let none = config(&app, "").await["data"].clone();
        assert_eq!(none["enabled"], true);
        assert!(none["tags"].as_array().unwrap().is_empty(), "{none}");
        let keys: Vec<&str> = none["categories"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["key"].as_str().unwrap())
            .collect();
        assert_eq!(keys, vec!["necessary", "analytics", "marketing"]);
        assert!(none["categories"].as_array().unwrap().iter().all(|c| c["preselected"] == false));
        assert_eq!(none["loading"], "after_consent_deferred");

        let analytics = config(&app, "analytics").await["data"]["tags"].clone();
        let analytics = analytics.as_array().unwrap();
        assert_eq!(analytics.len(), 1);
        assert_eq!(analytics[0]["provider"], "ga4");
        assert_eq!(
            analytics[0]["script"],
            "https://www.googletagmanager.com/gtag/js?id=G-ABC12345"
        );

        // Tạm dừng Meta: không còn mã, host của Meta rời khỏi CSP.
        let csp = config(&app, "").await["data"]["csp_hosts"].clone();
        assert!(csp.as_array().unwrap().iter().any(|host| host == "connect.facebook.net"));
        let (_, tags) = call(&app, "GET", "/tracking-tags", "admin", None).await;
        let meta =
            tags["data"].as_array().unwrap().iter().find(|tag| tag["provider"] == "meta").unwrap()
                ["id"]
                .as_str()
                .unwrap()
                .to_owned();
        let (status, _) = call(
            &app,
            "PATCH",
            &format!("/tracking-tags/{meta}"),
            "admin",
            Some(json!({"status": "paused"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let after = config(&app, "analytics,marketing").await["data"].clone();
        assert_eq!(after["tags"].as_array().unwrap().len(), 1, "{after}");
        assert!(
            !after["csp_hosts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|host| host == "connect.facebook.net")
        );
        // Câu `legal` không đổi được; nút từ chối có ở đó như nút đồng ý.
        assert_eq!(after["texts"]["legal"]["overridable"], false);
        assert_eq!(after["texts"]["reject_all"]["overridable"], true);
        assert!(after["texts"]["reject_all"]["text"].as_str().is_some());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doi_nhom_tang_phien_ban_chinh_sach() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let uri = format!("/cookie-settings/{}", root());
        let (status, set) =
            call(&app, "PUT", &uri, "admin", Some(json!({"categories": ["preferences"]}))).await;
        assert_eq!(status, StatusCode::OK, "{set}");
        assert_eq!(set["data"]["policy_version"], 1);
        let (_, same) = call(&app, "PUT", &uri, "admin", Some(json!({"position": "top"}))).await;
        assert_eq!(same["data"]["policy_version"], 1, "{same}");
        add(&app, "ga4", "G-ABC12345").await;
        let (_, after) = call(&app, "GET", &uri, "admin", None).await;
        assert_eq!(after["data"]["policy_version"], 2, "{after}");
        assert_eq!(after["data"]["effective_categories"], json!(["preferences", "analytics"]));
        let (status, problem) = call(
            &app,
            "PUT",
            &uri,
            "admin",
            Some(json!({"categories": ["necessary"], "position": "left"})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem["errors"].as_array().unwrap().len(), 2, "{problem}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bang_chung_dong_y_khong_luu_ip_tho_va_nguoi_dang_nhap_ghi_so_chung() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        add(&app, "ga4", "G-ABC12345").await;
        add(&app, "meta", "1234567890123456").await;
        let version = config(&app, "").await["data"]["policy_version"].as_i64().unwrap();
        // Phiên bản cũ: hỏi lại.
        assert_eq!(consent(&app, None, &["analytics"], version - 1).await, StatusCode::CONFLICT);
        assert_eq!(consent(&app, None, &["analytics"], version).await, StatusCode::NO_CONTENT);
        assert_eq!(
            consent(&app, Some("alice"), &["marketing"], version).await,
            StatusCode::NO_CONTENT
        );

        let rows = cookie_consent::Entity::find().all(app.database().read()).await.unwrap();
        assert_eq!(rows.len(), 2);
        for row in &rows {
            // Không có IP thô (trong test không có địa chỉ; nếu có thì chỉ là bản băm 22 ký tự).
            assert!(
                row.ip_hash.as_ref().is_none_or(|hash| hash.len() == 22 && !hash.contains('.'))
            );
        }
        assert_eq!(rows.iter().filter(|row| row.user_id == Some(Uuid::from_u128(2))).count(), 1);

        let ledger = app.api::<dyn ConsentLedger>().unwrap();
        let alice = ConsentSubject::User(Uuid::from_u128(2));
        let scope = format!("website:{}", root());
        let marketing = ConsentKey::new("marketing", scope.clone(), "");
        let analytics = ConsentKey::new("analytics", scope, "");
        assert!(ledger.is_granted(DEFAULT_TENANT, &alice, &marketing).await.unwrap());
        assert!(!ledger.is_granted(DEFAULT_TENANT, &alice, &analytics).await.unwrap());

        let (status, stats) =
            call(&app, "GET", &format!("/cookie-consents/stats?org_id={}", root()), "admin", None)
                .await;
        assert_eq!(status, StatusCode::OK, "{stats}");
        assert_eq!(stats["data"]["total"], 2);
        let marketing = stats["data"]["accepted"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["category"] == "marketing")
            .unwrap();
        assert_eq!(marketing["count"], 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nha_cung_cap_bat_hien_trong_ben_nhan_du_lieu() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        add(&app, "tiktok", "C1A2B3C4D5E6F7G8H9I0").await;
        let public = app.get("/api/v1/compliance/public/purposes").send().await.unwrap().json();
        let marketing =
            public["data"].as_array().unwrap().iter().find(|p| p["key"] == "marketing").unwrap();
        let recipients = marketing["recipients"].as_array().unwrap();
        assert!(
            recipients.iter().any(|r| r["name"] == "TikTok Pixel"
                && r["country"] == "SG"
                && r["source"] == "tracking"),
            "{marketing}"
        );
    })
    .await
    .unwrap();
}
