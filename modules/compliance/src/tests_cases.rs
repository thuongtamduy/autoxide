//! Test vụ việc (bước 37, D-363) và bảo vệ người tiêu dùng (D-364) qua REST và trait công bố, trên cả
//! SQLite và Postgres: kênh báo công khai có chống spam và mã tra cứu, ẩn / hiện đích qua event, phản
//! hồi sau hạn bị từ chối, quyết định cần phiên người, nhắc quá hạn không trùng, cung cấp thông tin
//! cho cơ quan chỉ lưu tên trường, lý do được ẩn đánh giá.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::job::{EnqueueOptions, Job, JobQueue};
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::resource::{Resource, ResourceResolver};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, ScopeExt, TenantContext};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Statement};
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::ModerationPolicy;
use crate::entity::case;
use crate::tests::{call, run_jobs};

const HANDLER: &[&str] = &[
    "compliance.cases.read",
    "compliance.cases.handle",
    "compliance.disclosures.manage",
    "compliance.policies.manage",
    "compliance.legal.manage",
];
/// Người bán giả (sổ đối tượng): `shop.item:1` do bob (user 3) quản lý.
const BOB: Uuid = Uuid::from_u128(3);

struct Items;

#[async_trait::async_trait]
impl ResourceResolver for Items {
    async fn resolve(&self, _: &TenantContext, id: &str) -> Result<Option<Resource>, Error> {
        Ok(matches!(id, "1" | "2")
            .then(|| Resource::new("Item").public(true).owner_user_ids(vec![BOB])))
    }
}

struct FakeShop;

impl Module for FakeShop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.resource_type("shop.item", Arc::new(Items))
    }
}

async fn app(test: &TestDatabase) -> TestApp {
    TestApp::builder(test)
        .setting("AUTOXIDE_ANTISPAM_MIN_FILL_SECONDS", "0")
        .user("admin", 1, HANDLER)
        .user("alice", 2, &[])
        .user("bob", 3, &[])
        .principal(
            "robot",
            Principal::new(Uuid::from_u128(4), HANDLER.iter().map(|name| (*name).to_owned()))
                .via_api_key(Uuid::from_u128(5)),
        )
        .module(FakeShop)
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

async fn events(database: &Database, name: &str) -> Vec<Value> {
    let backend = database.read().get_database_backend();
    let sql = format!("SELECT payload FROM compliance_events WHERE name = '{name}' ORDER BY id");
    database
        .read()
        .query_all_raw(Statement::from_string(backend, sql))
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            let text: String = row.try_get("", "payload").unwrap();
            serde_json::from_str(&text).unwrap()
        })
        .collect()
}

/// Dời một mốc của vụ `id` (thời gian giả).
async fn set_time(app: &TestApp, id: &str, column: case::Column, at: &str) {
    let at: Timestamp = at.parse().unwrap();
    // tenant-scope: việc nền / kiểm tra quét mọi khách của database, chủ ý không lọc khách (rà soát bảo mật 2026-11)
    case::Entity::update_many()
        .col_expr(column, Expr::value(at))
        .filter(case::Column::Id.eq(id.parse::<Uuid>().unwrap()))
        .exec(app.database().write())
        .await
        .unwrap();
}

async fn token(app: &TestApp, scope: &str) -> String {
    let issued = app.get(&format!("/api/v1/antispam/token?scope={scope}")).send().await.unwrap();
    assert_eq!(issued.status(), StatusCode::OK, "{}", issued.text());
    issued.json()["data"]["token"].as_str().unwrap().to_owned()
}

async fn public(app: &TestApp, path: &str, body: Value) -> (StatusCode, Value) {
    let answer = app.post(&format!("/api/v1/compliance{path}")).json(body).send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

fn staff_case(kind: &str, target: &str) -> Value {
    json!({
        "kind": kind, "target": target, "reporter_kind": "rights_holder", "name": "Brand Co",
        "email": "legal@brand.example", "claim": "This item copies our registered trademark.",
        "rights": [{"kind": "trademark", "number": "4-0123456"}],
    })
}

#[tokio::test]
async fn bao_cong_khai_co_chong_spam() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let report = json!({
            "kind": "counterfeit", "target": "shop.item:1", "name": "Lan", "email": "lan@example.com",
            "claim": "The bag sold here is a fake of a known brand.",
        });
        // Không token form: lỗi để người thật tải lại form.
        let (status, problem) = public(&app, "/public/reports", report.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
        assert_eq!(problem["code"], "antispam.token_missing");
        // Bẫy bị điền: trả lời như đã nhận, không lưu gì.
        let mut trapped = report.clone();
        trapped["form_token"] = json!(token(&app, "compliance:reports").await);
        trapped["trap"] = json!("http://spam.example");
        let (status, fake) = public(&app, "/public/reports", trapped).await;
        assert_eq!(status, StatusCode::CREATED, "{fake}");
        assert!(fake["data"]["tracking_token"].is_string());
        let (_, listed) = call(&app, "GET", "/cases", "admin", None).await;
        assert_eq!(listed["data"].as_array().unwrap().len(), 0, "{listed}");

        // Đích không có: báo lỗi trường.
        let mut missing = report.clone();
        missing["form_token"] = json!(token(&app, "compliance:reports").await);
        missing["target"] = json!("shop.item:9");
        let (status, problem) = public(&app, "/public/reports", missing).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");

        let mut good = report.clone();
        let form = token(&app, "compliance:reports").await;
        good["form_token"] = json!(form);
        let (status, receipt) = public(&app, "/public/reports", good.clone()).await;
        assert_eq!(status, StatusCode::CREATED, "{receipt}");
        let id = receipt["data"]["id"].as_str().unwrap().to_owned();
        let tracking = receipt["data"]["tracking_token"].as_str().unwrap().to_owned();
        // Gửi lại cùng token form: không tạo vụ thứ hai.
        let (status, _) = public(&app, "/public/reports", good).await;
        assert_eq!(status, StatusCode::CREATED);
        let (_, listed) = call(&app, "GET", "/cases", "admin", None).await;
        assert_eq!(listed["data"].as_array().unwrap().len(), 1, "{listed}");
        let case = &listed["data"][0];
        assert_eq!((case["status"].as_str(), case["reporter_kind"].as_str()), (Some("awaiting_response"), Some("guest")));

        // Tra cứu bằng mã: đúng mã thì thấy, sai mã như không có.
        let found = app
            .get(&format!("/api/v1/compliance/public/cases/{id}?token={tracking}"))
            .send()
            .await
            .unwrap();
        assert_eq!(found.status(), StatusCode::OK, "{}", found.text());
        assert_eq!(found.json()["data"]["kind"], "counterfeit");
        assert!(found.json()["data"].get("reporter_email").is_none());
        let wrong =
            app.get(&format!("/api/v1/compliance/public/cases/{id}?token=nope")).send().await.unwrap();
        assert_eq!(wrong.status(), StatusCode::NOT_FOUND);

        // Khiếu nại của người tiêu dùng: kênh riêng, hạn từ quy tắc.
        let complaint = json!({
            "name": "Minh", "phone": "0912345678", "claim": "The order arrived broken and nobody answers.",
            "form_token": token(&app, "compliance:complaints").await,
        });
        let (status, receipt) = public(&app, "/public/complaints", complaint).await;
        assert_eq!(status, StatusCode::CREATED, "{receipt}");
        let (_, listed) = call(&app, "GET", "/cases?kind=consumer_complaint", "admin", None).await;
        assert_eq!(listed["data"][0]["status"], "under_review", "{listed}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn an_dich_roi_khoi_phuc_khi_bac() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (status, _) = call(&app, "PUT", "/policies/cases.auto_restrict", "admin", Some(json!({"value": true}))).await;
        assert_eq!(status, StatusCode::OK);
        let (status, first) =
            call(&app, "POST", "/cases", "admin", Some(staff_case("counterfeit", "shop.item:1"))).await;
        assert_eq!(status, StatusCode::CREATED, "{first}");
        assert_eq!(first["data"]["restricted"], true);
        assert!(first["data"]["tracking_token"].is_string());
        let restricted = events(app.database(), "compliance.target_restricted").await;
        assert_eq!(restricted.len(), 1);
        assert_eq!(restricted[0]["target"], "shop.item:1");
        // Không có tên người báo trong event (webhooks chuyển event ra ngoài).
        assert!(!restricted[0].to_string().contains("Brand Co"));

        // Vụ thứ hai cùng đích, ẩn tay.
        let (_, second) =
            call(&app, "POST", "/cases", "admin", Some(staff_case("other", "shop.item:1"))).await;
        let second_id = second["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(second["data"]["restricted"], false);
        let (status, again) = call(&app, "POST", &format!("/cases/{second_id}/restrict"), "admin", Some(json!({"reason": "same item"}))).await;
        assert_eq!(status, StatusCode::OK, "{again}");

        // Quyết định cần phiên người: API key bị từ chối.
        let first_id = first["data"]["id"].as_str().unwrap().to_owned();
        let decision = json!({"decision": "rejected", "reason": "The seller showed an invoice from the brand."});
        let (status, problem) = call(&app, "POST", &format!("/cases/{first_id}/decision"), "robot", Some(decision.clone())).await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::FORBIDDEN, Some("compliance.session_required")));
        let (status, decided) = call(&app, "POST", &format!("/cases/{first_id}/decision"), "admin", Some(decision.clone())).await;
        assert_eq!(status, StatusCode::OK, "{decided}");
        assert_eq!((decided["data"]["status"].as_str(), decided["data"]["restricted"].as_bool()), (Some("decided"), Some(false)));
        // Vụ kia vẫn ẩn cùng đích: chưa hiện lại.
        assert_eq!(events(app.database(), "compliance.target_restored").await.len(), 0);
        let (status, _) = call(&app, "POST", &format!("/cases/{first_id}/decision"), "admin", Some(decision)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        let (status, closed) = call(&app, "POST", &format!("/cases/{second_id}/close"), "admin", Some(json!({"withdrawn": true, "reason": "reporter withdrew"}))).await;
        assert_eq!(status, StatusCode::OK, "{closed}");
        assert_eq!(closed["data"]["status"], "withdrawn");
        let restored = events(app.database(), "compliance.target_restored").await;
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0]["case_id"], second_id.as_str());
        // Lưu vết: mọi bước một dòng.
        let (_, detail) = call(&app, "GET", &format!("/cases/{first_id}"), "admin", None).await;
        let kinds: Vec<&str> = detail["data"]["entries"].as_array().unwrap().iter().map(|item| item["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["status", "restriction", "restriction", "decision"], "{detail}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_vu_cung_an_mot_dich_bac_cung_luc_van_hien_lai() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (status, _) = call(&app, "PUT", "/policies/cases.auto_restrict", "admin", Some(json!({"value": true}))).await;
        assert_eq!(status, StatusCode::OK);
        let decision = json!({"decision": "rejected", "reason": "The seller showed an invoice from the brand."});
        for round in 1..=4 {
            let mut ids = Vec::new();
            for _ in 0..2 {
                let (status, opened) =
                    call(&app, "POST", "/cases", "admin", Some(staff_case("counterfeit", "shop.item:2"))).await;
                assert_eq!(status, StatusCode::CREATED, "{opened}");
                assert_eq!(opened["data"]["restricted"], true);
                ids.push(opened["data"]["id"].as_str().unwrap().to_owned());
            }
            // Bác cả hai vụ cùng lúc. Postgres từng cho mỗi bên thấy vụ kia còn ẩn: không bên nào
            // phát `target_restored`, đích bị ẩn vĩnh viễn.
            let (first, second) =
                (format!("/cases/{}/decision", ids[0]), format!("/cases/{}/decision", ids[1]));
            let (one, two) = tokio::join!(
                call(&app, "POST", &first, "admin", Some(decision.clone())),
                call(&app, "POST", &second, "admin", Some(decision.clone())),
            );
            assert_eq!(one.0, StatusCode::OK, "{}", one.1);
            assert_eq!(two.0, StatusCode::OK, "{}", two.1);
            assert_eq!(events(app.database(), "compliance.target_restored").await.len(), round);
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn phan_hoi_sau_han_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (_, opened) = call(
            &app,
            "POST",
            "/cases",
            "admin",
            Some(staff_case("ip_infringement", "shop.item:2")),
        )
        .await;
        let id = opened["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(opened["data"]["respondent_user_ids"], json!([BOB]));
        assert_eq!(opened["data"]["status"], "awaiting_response");

        // Bên bị phản ánh thấy vụ của mình, không thấy email người báo.
        let (status, mine) = call(&app, "GET", "/respondent/cases", "bob", None).await;
        assert_eq!(status, StatusCode::OK, "{mine}");
        assert_eq!(mine["data"][0]["id"], id.as_str());
        assert_eq!(mine["data"][0]["reporter_name"], "Brand Co");
        assert!(!mine.to_string().contains("legal@brand.example"));
        let (_, none) = call(&app, "GET", "/respondent/cases", "alice", None).await;
        assert_eq!(none["data"], json!([]));
        let answer = json!({"body": "We bought these from the brand's distributor."});
        let (status, _) =
            call(&app, "POST", &format!("/cases/{id}/responses"), "alice", Some(answer.clone()))
                .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Hết hạn phản hồi: từ chối.
        set_time(&app, &id, case::Column::RespondBy, "2020-01-01T00:00:00Z").await;
        let (status, problem) =
            call(&app, "POST", &format!("/cases/{id}/responses"), "bob", Some(answer.clone()))
                .await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("compliance.response_closed"))
        );
        set_time(&app, &id, case::Column::RespondBy, "2999-01-01T00:00:00Z").await;
        let (status, entry) =
            call(&app, "POST", &format!("/cases/{id}/responses"), "bob", Some(answer)).await;
        assert_eq!(status, StatusCode::CREATED, "{entry}");
        assert_eq!(entry["data"]["kind"], "response");
        let (_, detail) = call(&app, "GET", &format!("/cases/{id}"), "admin", None).await;
        assert_eq!(detail["data"]["status"], "under_review");
    })
    .await
    .unwrap();
}

#[derive(Serialize)]
struct CaseDeadlines;

impl Job for CaseDeadlines {
    const KIND: &'static str = "compliance.case_deadlines";
}

#[tokio::test]
async fn nhac_qua_han_dung_moc_khong_trung() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (_, opened) =
            call(&app, "POST", "/cases", "admin", Some(staff_case("other", "shop.item:1"))).await;
        let id = opened["data"]["id"].as_str().unwrap().to_owned();
        set_time(&app, &id, case::Column::AckDueAt, "2020-01-01T00:00:00Z").await;
        for _ in 0..2 {
            JobQueue::new("compliance")
                .enqueue_database(app.database().write(), &CaseDeadlines, EnqueueOptions::default())
                .await
                .unwrap();
            run_jobs(&app).await;
        }
        let overdue = events(app.database(), "compliance.case_overdue").await;
        assert_eq!(overdue.len(), 1, "{overdue:?}");
        assert_eq!(overdue[0]["step"], "ack");
        // Qua hạn quyết định: nhắc thêm đúng một lần.
        set_time(&app, &id, case::Column::DecideBy, "2020-01-02T00:00:00Z").await;
        for _ in 0..2 {
            JobQueue::new("compliance")
                .enqueue_database(app.database().write(), &CaseDeadlines, EnqueueOptions::default())
                .await
                .unwrap();
            run_jobs(&app).await;
        }
        let overdue = events(app.database(), "compliance.case_overdue").await;
        assert_eq!(overdue.len(), 2, "{overdue:?}");
        assert_eq!(overdue[1]["step"], "decide");
        let (_, listed) = call(&app, "GET", "/cases?overdue=true", "admin", None).await;
        assert_eq!(listed["data"][0]["overdue"], true);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn disclosure_ghi_audit_khong_ghi_gia_tri() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (status, profile) = call(&app, "POST", "/legal-profiles", "admin", Some(json!({
            "entity_type": "organization", "legal_name": "Cong ty Vi Du", "tax_code": "0101234567",
        }))).await;
        assert_eq!(status, StatusCode::CREATED, "{profile}");
        let org = autoxide_kernel::tenancy::root_of(DEFAULT_TENANT).as_uuid();
        let request = json!({
            "authority": "Market surveillance office", "request_reference": "123/QLTT",
            "legal_basis": "Decree 85/2021/ND-CP", "subject": format!("org:{org}"),
            "fields": ["legal_name", "tax_code"],
        });
        let (status, problem) =
            call(&app, "POST", "/disclosures", "robot", Some(request.clone())).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("compliance.session_required"))
        );
        let (status, given) = call(&app, "POST", "/disclosures", "admin", Some(request)).await;
        assert_eq!(status, StatusCode::CREATED, "{given}");
        assert_eq!(given["data"]["values"]["tax_code"], "0101234567");
        let (_, listed) = call(&app, "GET", "/disclosures", "admin", None).await;
        assert_eq!(listed["data"][0]["fields"], json!(["legal_name", "tax_code"]));
        assert!(listed["data"][0].get("values").is_none());
        assert!(!listed.to_string().contains("0101234567"));
        let recorded = events(app.database(), "compliance.disclosure_recorded").await;
        assert_eq!(recorded.len(), 1);
        assert!(!recorded[0].to_string().contains("0101234567"));
        let (status, _) = call(
            &app,
            "POST",
            "/disclosures",
            "admin",
            Some(json!({
                "authority": "x", "request_reference": "1", "legal_basis": "y",
                "subject": format!("org:{org}"), "fields": ["password"],
            })),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

/// Vùng pháp lý không có quy tắc `moderation.hide_reasons`: luật không giới hạn lý do ẩn (kiểm độc lập #575).
#[tokio::test]
async fn vung_khong_co_quy_tac_an_thi_khong_gioi_han() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (status, set) =
            call(&app, "PUT", "/policies/jurisdiction", "admin", Some(json!({"value": "xx"})))
                .await;
        assert_eq!(status, StatusCode::OK, "{set}");
        let policy = app.api::<dyn ModerationPolicy>().unwrap();
        policy.check_hide_reason(DEFAULT_TENANT, "reviews", "negative").await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ly_do_an_danh_gia_va_kenh_khieu_nai() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let policy = app.api::<dyn ModerationPolicy>().unwrap();
        policy.check_hide_reason(DEFAULT_TENANT, "reviews", "spam").await.unwrap();
        let error =
            policy.check_hide_reason(DEFAULT_TENANT, "reviews", "negative").await.unwrap_err();
        assert_eq!(error.code(), "compliance.hide_reason_not_allowed");
        let reasons = app.get("/api/v1/compliance/public/moderation-reasons").send().await.unwrap();
        assert_eq!(reasons.status(), StatusCode::OK);
        assert!(reasons.json()["data"]["reasons"].as_array().unwrap().contains(&json!("spam")));

        let (status, _) = call(
            &app,
            "PUT",
            "/policies/complaints.contact",
            "admin",
            Some(json!({"value": {"email": "care@shop.example"}})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let channel = app.get("/api/v1/compliance/public/complaints").send().await.unwrap();
        assert_eq!(channel.status(), StatusCode::OK, "{}", channel.text());
        let data = &channel.json()["data"];
        assert_eq!(data["contact"]["email"], "care@shop.example");
        assert_eq!(data["decide_within_working_days"], 7);
        assert_eq!(data["respond_within_working_days"], 3);
    })
    .await
    .unwrap();
}

/// Phiếu hỗ trợ chuyển thành khiếu nại (`ComplaintIntake`, bước 53): gọi lại cùng nguồn không tạo
/// vụ việc thứ hai; vụ việc có hạn luật định và bên bị khiếu nại.
#[tokio::test]
async fn khieu_nai_tu_phieu_goi_lai_khong_tao_hai_vu_viec() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let intake = app.api::<dyn crate::api::ComplaintIntake>().unwrap();
        let tenant = app.default_tenant();
        let complaint = || crate::api::ComplaintFromTicket {
            source_ref: "shop.item:1".to_owned(),
            org_id: tenant.id().as_uuid(),
            reporter_user_id: None,
            name: Some("An".to_owned()),
            email: Some("an@example.com".to_owned()),
            phone: None,
            claim: "Sản phẩm bị lỗi, cửa hàng không đổi.".to_owned(),
        };
        let first = intake.open(&tenant, complaint()).await.unwrap();
        assert!(first.created);
        let again = intake.open(&tenant, complaint()).await.unwrap();
        assert_eq!((again.id, again.created), (first.id, false));
        let row = case::Entity::find_by_id(first.id)
            .scoped(&tenant)
            .one(app.database().read())
            .await
            .unwrap();
        let row = row.unwrap();
        assert_eq!((row.kind.as_str(), row.channel.as_str()), ("consumer_complaint", "support"));
        let short = crate::api::ComplaintFromTicket {
            claim: "ngắn".to_owned(),
            source_ref: "shop.item:2".to_owned(),
            ..complaint()
        };
        assert_eq!(intake.open(&tenant, short).await.unwrap_err().code(), "validation_failed");
    })
    .await
    .unwrap();
}
