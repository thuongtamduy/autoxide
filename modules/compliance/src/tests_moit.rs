//! Test hồ sơ Bộ Công Thương (bước 37, D-357, D-358) qua REST, MCP và trait công bố, trên cả SQLite và
//! Postgres: danh sách kiểm báo thiếu và đổi theo phiên bản quy tắc, "fact" chưa ai trả lời là
//! `unknown`, đoạn mã logo lạ bị từ chối, logo công khai theo tên miền, `doctor` cảnh báo, bộ hồ sơ
//! điền từ dữ liệu thật, phạm vi theo nút.

use std::sync::Arc;

use autoxide_kernel::access::{Grant, Principal, Scope};
use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::resource::{Resource, ResourceResolver};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, TenantContext, TenantId};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{ComplianceFacts, Fact, LegalProfiles, WebsiteRef};
use crate::tests::call;

/// Trang công khai giả của `content` (sổ đối tượng): ID 100 đã đăng, 200 là nháp.
const PUBLISHED: Uuid = Uuid::from_u128(100);
const DRAFT: Uuid = Uuid::from_u128(200);

struct FakePages;

#[async_trait::async_trait]
impl ResourceResolver for FakePages {
    async fn resolve(&self, _: &TenantContext, id: &str) -> Result<Option<Resource>, Error> {
        Ok(if id == PUBLISHED.to_string() {
            Some(Resource::new("Doi tra").public(true).url(Some("/doi-tra".to_owned())))
        } else if id == DRAFT.to_string() {
            Some(Resource::new("Nhap"))
        } else {
            None
        })
    }
}

/// Module giả tên `content`: chỉ đăng ký loại đối tượng `content.entry`.
struct FakeContent;

impl Module for FakeContent {
    fn name(&self) -> &'static str {
        "content"
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.resource_type("content.entry", Arc::new(FakePages))
    }
}

const LEGAL: &[&str] = &[
    "compliance.legal.read",
    "compliance.legal.manage",
    "compliance.policies.manage",
    "compliance.documents.manage",
    "compliance.documents.read",
];

async fn app(test: &TestDatabase) -> TestApp {
    TestApp::builder(test)
        .user("admin", 1, LEGAL)
        .user("nobody", 2, &[])
        .user("staff", 5, &["compliance.rules.read", "compliance.rules.manage"])
        .provide::<dyn autoxide_address::api::AdministrativeUnits>(Arc::new(
            autoxide_address::api::testing::FakeAdministrativeUnits::sample(),
        ))
        .module(FakeContent)
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

fn statuses(checklist: &Value) -> std::collections::HashMap<String, String> {
    checklist["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (item["key"].as_str().unwrap().to_owned(), item["status"].as_str().unwrap().to_owned())
        })
        .collect()
}

/// Tạo hồ sơ cấp khách và một website; trả ID website.
async fn website(app: &TestApp, kind: &str, domain: &str) -> String {
    let (status, created) =
        call(app, "POST", "/websites", "admin", Some(json!({"domain": domain, "kind": kind})))
            .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created["data"]["id"].as_str().unwrap().to_owned()
}

fn full_profile() -> Value {
    json!({
        "entity_type": "organization",
        "legal_name": "Cong ty TNHH Vi Du",
        "tax_code": "0101234567",
        "registration_number": "0101234567",
        "registration_issued_on": "2020-01-02",
        "registration_issued_by": "Phong Dang ky kinh doanh Ha Noi",
        "address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "12 Trang Tien"},
        "representative_name": "Nguyen Van A",
        "representative_title": "Giam doc",
        "phone": "024 1234 5678",
        "email": "lienhe@shop.example"
    })
}

#[tokio::test]
async fn danh_sach_kiem_bao_thieu_trang_va_ma_so_thue() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let id = website(&app, "sales", "Shop.Example").await;
        // Chưa có hồ sơ: mục hồ sơ báo thiếu.
        let (status, checklist) =
            call(&app, "GET", &format!("/websites/{id}/checklist"), "admin", None).await;
        assert_eq!(status, StatusCode::OK, "{checklist}");
        assert_eq!(checklist["data"]["domain"], "shop.example");
        assert_eq!(checklist["data"]["procedure"], "notification");
        assert_eq!(checklist["data"]["selling_before_done"], "label");
        let items = statuses(&checklist["data"]);
        assert_eq!(items["owner.legal_name"], "missing");
        assert_eq!(items["moit.account"], "manual");

        // Hồ sơ thiếu mã số thuế; gom mọi lỗi một lần.
        let (status, problem) = call(
            &app,
            "POST",
            "/legal-profiles",
            "admin",
            Some(json!({"entity_type": "x", "legal_name": "", "tax_code": "12", "email": "no"})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["errors"].as_array().unwrap().len(), 4, "{problem}");
        let (status, profile) = call(
            &app,
            "POST",
            "/legal-profiles",
            "admin",
            Some(json!({"entity_type": "organization", "legal_name": "Cong ty TNHH Vi Du",
                "address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "12 Trang Tien"}})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{profile}");
        assert_eq!(profile["data"]["address"]["commune_name"], "Phuong Ba Dinh");
        let (status, again) = call(
            &app,
            "POST",
            "/legal-profiles",
            "admin",
            Some(json!({"entity_type": "organization", "legal_name": "Khac"})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{again}");

        let (_, checklist) =
            call(&app, "GET", &format!("/websites/{id}/checklist"), "admin", None).await;
        let items = statuses(&checklist["data"]);
        assert_eq!(items["owner.legal_name"], "met");
        assert_eq!(items["owner.address"], "met");
        assert_eq!(items["owner.tax_code"], "missing");
        assert_eq!(items["page.return_policy"], "missing");
        assert_eq!(items["policy.complaints_contact"], "missing");

        // Gắn trang: nháp chưa tính, đã đăng thì đạt; trang không có → 422.
        let path = format!("/websites/{id}/pages/return_policy");
        let (status, _) =
            call(&app, "PUT", &path, "admin", Some(json!({"entry_id": Uuid::from_u128(9)}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, _) =
            call(&app, "PUT", &format!("/websites/{id}/pages/nope"), "admin", Some(json!({"entry_id": PUBLISHED})))
                .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, _) = call(&app, "PUT", &path, "admin", Some(json!({"entry_id": DRAFT}))).await;
        assert_eq!(status, StatusCode::OK);
        let (_, checklist) =
            call(&app, "GET", &format!("/websites/{id}/checklist"), "admin", None).await;
        assert_eq!(statuses(&checklist["data"])["page.return_policy"], "missing");
        let (status, _) =
            call(&app, "PUT", &path, "admin", Some(json!({"entry_id": PUBLISHED}))).await;
        assert_eq!(status, StatusCode::OK);

        // Sửa hồ sơ với `If-Match`: bản cũ → 412.
        let profile_id = profile["data"]["id"].as_str().unwrap();
        let read = app
            .get(&format!("/api/v1/compliance/legal-profiles/{profile_id}"))
            .token("admin")
            .send()
            .await
            .unwrap();
        let etag = read.header("etag").to_owned();
        let answer = app
            .patch(&format!("/api/v1/compliance/legal-profiles/{profile_id}"))
            .token("admin")
            .header("if-match", &etag)
            .json(json!({"tax_code": "0101234567"}))
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), StatusCode::OK, "{}", answer.text());
        let stale = app
            .patch(&format!("/api/v1/compliance/legal-profiles/{profile_id}"))
            .token("admin")
            .header("if-match", &etag)
            .json(json!({"trade_name": "Vi Du"}))
            .send()
            .await
            .unwrap();
        assert_eq!(stale.status(), StatusCode::PRECONDITION_FAILED);
        let (status, _) = call(
            &app,
            "PUT",
            "/policies/complaints.contact",
            "admin",
            Some(json!({"value": {"email": "khieunai@shop.example"}})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (_, checklist) =
            call(&app, "GET", &format!("/websites/{id}/checklist"), "admin", None).await;
        let items = statuses(&checklist["data"]);
        assert_eq!(items["owner.tax_code"], "met");
        assert_eq!(items["page.return_policy"], "met");
        assert_eq!(items["policy.complaints_contact"], "met");
        assert!(checklist["data"]["items"][0]["legal_basis"].as_str().unwrap().contains("52/2013"));

        // Không có permission: 403 trước mọi tra cứu, kể cả với ID không có.
        for uri in [format!("/websites/{id}/checklist"), format!("/websites/{}", Uuid::nil())] {
            let (status, _) = call(&app, "GET", &uri, "nobody", None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{uri}");
        }
        let (status, _) = call(&app, "GET", "/websites/01937000-0000-7000-8000-000000000000", "admin", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn danh_sach_kiem_doi_theo_phien_ban_quy_tac() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let id = website(&app, "sales", "shop.example").await;
        let (_, before) =
            call(&app, "GET", &format!("/websites/{id}/checklist"), "admin", None).await;
        assert_eq!(before["data"]["rule_version"], 2);
        assert!(before["data"]["items"].as_array().unwrap().len() > 10);
        let pack = json!({"pack": {"jurisdiction": "vn", "rules": [{
            "key": "moit.checklist.sales", "version": 3,
            "effective_from": "2026-08-01T00:00:00Z", "effective_to": null,
            "body": {"registration": "notification", "status_when_done": "notified",
                "logo": {"color": "blue", "link_hosts": ["online.gov.vn"], "image_hosts": ["online.gov.vn"]},
                "items": [{"key": "owner.legal_name", "title": {"en": "Name", "vi": "Tên"},
                    "check": {"type": "profile_field", "field": "legal_name"},
                    "legal_basis": "Ví dụ — cần đối chiếu văn bản gốc.", "needs_legal_review": true}]},
            "legal_basis": "Ví dụ — cần đối chiếu văn bản gốc."
        }]}});
        let (status, imported) = call(&app, "POST", "/rules", "staff", Some(pack)).await;
        assert_eq!(status, StatusCode::OK, "{imported}");
        let (_, after) = call(&app, "GET", &format!("/websites/{id}/checklist"), "admin", None).await;
        assert_eq!(after["data"]["rule_version"], 3);
        assert_eq!(after["data"]["items"].as_array().unwrap().len(), 1);
    })
    .await
    .unwrap();
}

struct SellerInfo(bool);

#[async_trait::async_trait]
impl Fact for SellerInfo {
    async fn holds(&self, _: TenantId, website: &WebsiteRef) -> Result<bool, Error> {
        assert_eq!(website.kind, "platform");
        Ok(self.0)
    }
}

#[tokio::test]
async fn fact_chua_dang_ky_la_unknown() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let id = website(&app, "platform", "market.example").await;
        let (_, checklist) =
            call(&app, "GET", &format!("/websites/{id}/checklist"), "admin", None).await;
        assert_eq!(checklist["data"]["procedure"], "registration");
        let items = statuses(&checklist["data"]);
        assert_eq!(items["marketplace.seller_info_on_product"], "unknown");
        assert_eq!(items["document.platform_rules"], "missing");
        assert!(checklist["data"]["unknown"].as_u64().unwrap() >= 1);
        let facts = app.api::<dyn ComplianceFacts>().unwrap();
        facts.register("marketplace.seller_info_on_product", Arc::new(SellerInfo(true))).unwrap();
        assert!(
            facts
                .register("marketplace.seller_info_on_product", Arc::new(SellerInfo(false)))
                .is_err()
        );
        let (_, checklist) =
            call(&app, "GET", &format!("/websites/{id}/checklist"), "admin", None).await;
        assert_eq!(statuses(&checklist["data"])["marketplace.seller_info_on_product"], "met");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn snippet_logo_la_bi_tu_choi_va_badge_cong_khai_theo_ten_mien() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let id = website(&app, "sales", "shop.example").await;
        let path = format!("/websites/{id}/status");
        // Chưa có logo: công khai 404.
        let answer =
            app.get("/api/v1/compliance/public/badge").host("shop.example").send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::NOT_FOUND);
        for snippet in [
            "<script>alert(1)</script>",
            "<a href=\"https://online.gov.vn/x\"><img src=\"https://online.gov.vn/a.png\" onerror=\"alert(1)\"></a>",
            "<a href=\"https://evil.example/x\"><img src=\"https://online.gov.vn/a.png\"></a>",
        ] {
            let (status, problem) = call(
                &app,
                "POST",
                &path,
                "admin",
                Some(json!({"status": "notified", "snippet": snippet})),
            )
            .await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{snippet}");
            assert_eq!(problem["code"], "compliance.logo_snippet_invalid", "{problem}");
        }
        // Website bán hàng không "đăng ký"; thiếu logo khi `notified` → 422.
        let (status, _) =
            call(&app, "POST", &path, "admin", Some(json!({"status": "registered"}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, _) =
            call(&app, "POST", &path, "admin", Some(json!({"status": "notified"}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, submitted) = call(
            &app,
            "POST",
            &path,
            "admin",
            Some(json!({"status": "submitted", "reference": "HS-001"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{submitted}");
        // Phiên bản quy tắc đang hiệu lực: v2 từ 01/7/2026 (NĐ 248/2026).
        assert_eq!(submitted["data"]["checklist_version"], 2);
        let real = "<a href='http://online.gov.vn/Home/WebDetails/12345'><img alt='' title='' \
                    src='http://online.gov.vn/Content/EndUser/LogoCCDVSaleNoti/logoSaleNoti.png'/></a>";
        let (status, notified) = call(
            &app,
            "POST",
            &path,
            "admin",
            Some(json!({"status": "notified", "snippet": real})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{notified}");
        assert_eq!(notified["data"]["status"], "notified");
        assert_eq!(notified["data"]["reference"], "HS-001");

        let answer =
            app.get("/api/v1/compliance/public/badge").host("shop.example").send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::OK, "{}", answer.text());
        let badge = answer.json();
        assert_eq!(badge["data"]["color"], "blue");
        assert_eq!(badge["data"]["link_url"], "http://online.gov.vn/Home/WebDetails/12345");
        let snippet = badge["data"]["snippet"].as_str().unwrap();
        assert!(snippet.starts_with("<a href=\"http://online.gov.vn/Home/WebDetails/12345\""));
        assert!(snippet.contains("Đã thông báo Bộ Công Thương"), "{snippet}");
        assert!(!answer.header("etag").is_empty());
        // `?domain=` cho giao diện không gọi trên tên miền của website; tên miền khác → 404.
        let answer = app
            .get("/api/v1/compliance/public/badge?domain=shop.example&locale=en")
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), StatusCode::OK);
        assert!(answer.json()["data"]["alt"].as_str().unwrap().contains("Notified"));
        let answer =
            app.get("/api/v1/compliance/public/badge").host("other.example").send().await.unwrap();
        // Không gắn tên miền: website đã xong thủ tục của cả khách.
        assert_eq!(answer.status(), StatusCode::OK);

        // Thu hồi: logo mất.
        let (status, _) =
            call(&app, "POST", &path, "admin", Some(json!({"status": "revoked"}))).await;
        assert_eq!(status, StatusCode::OK);
        let answer =
            app.get("/api/v1/compliance/public/badge").host("shop.example").send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doctor_canh_bao_website_thieu_muc() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let check = || {
            app.modules()
                .checks()
                .into_iter()
                .find(|(module, check)| *module == "compliance" && check.name == "moit_readiness")
                .unwrap()
                .1
        };
        let finding = check().run().await;
        assert_eq!(finding.status, autoxide_kernel::doctor::Status::Warning, "{finding:?}");
        assert!(finding.message.contains("No website"), "{finding:?}");
        website(&app, "sales", "shop.example").await;
        let finding = check().run().await;
        assert_eq!(finding.status, autoxide_kernel::doctor::Status::Warning, "{finding:?}");
        assert!(finding.message.contains("shop.example"), "{finding:?}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dossier_dien_tu_ho_so_that() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (status, profile) =
            call(&app, "POST", "/legal-profiles", "admin", Some(full_profile())).await;
        assert_eq!(status, StatusCode::CREATED, "{profile}");
        let id = website(&app, "platform", "market.example").await;
        let (status, dossier) =
            call(&app, "GET", &format!("/websites/{id}/dossier"), "admin", None).await;
        assert_eq!(status, StatusCode::OK, "{dossier}");
        let data = &dossier["data"];
        assert_eq!(data["form"]["owner"]["legal_name"], "Cong ty TNHH Vi Du");
        assert_eq!(
            data["form"]["owner"]["address_line"],
            "12 Trang Tien, Phuong Ba Dinh, Thanh pho Ha Noi"
        );
        assert_eq!(data["form"]["website"]["domain"], "market.example");
        assert_eq!(data["form"]["identity_verified"], false);
        let documents = data["documents"].as_array().unwrap();
        let keys: Vec<&str> = documents.iter().map(|doc| doc["key"].as_str().unwrap()).collect();
        assert_eq!(keys, ["declaration", "project_plan", "platform_rules"]);
        for document in documents {
            let body = document["body"].as_str().unwrap();
            assert!(body.contains("Cong ty TNHH Vi Du"), "{body}");
            assert!(body.contains("chuyên môn pháp lý"), "{body}");
        }
        assert!(documents[0]["body"].as_str().unwrap().contains("0101234567"));
        assert!(data["disclaimer"].as_str().unwrap().contains("legal professional"));

        // Biến mẫu qua REST và qua trait `LegalProfiles` (cho `content`).
        let profile_id = profile["data"]["id"].as_str().unwrap();
        let (status, variables) = call(
            &app,
            "GET",
            &format!("/legal-profiles/{profile_id}/template-variables"),
            "admin",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(variables["data"]["contact"]["email"], "lienhe@shop.example");
        let profiles = app.api::<dyn LegalProfiles>().unwrap();
        let found =
            profiles.template_variables(&app.default_tenant(), None).await.unwrap().unwrap();
        assert_eq!(found["owner"]["tax_code"], "0101234567");

        // Thông tin chủ sở hữu công khai cho chân trang.
        let answer = app.get("/api/v1/compliance/public/owner").send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::OK, "{}", answer.text());
        assert_eq!(answer.json()["data"]["legal_name"], "Cong ty TNHH Vi Du");
        // Khối chân trang pháp lý (bước 67) trả cùng dữ liệu; website chưa thông báo: không logo.
        let tenant = app.default_tenant();
        let site = autoxide_kernel::tenancy::Site::whole(&tenant);
        let request = autoxide_kernel::blocks::BlockRequest::new(&tenant, &site, "vi");
        let footer = autoxide_kernel::blocks::BlockRef::new("f", json!({}));
        let data = app.blocks().resolve(&request, &[("compliance.legal_footer", footer)]).await;
        assert_eq!(data["f"]["owner"]["legal_name"], "Cong ty TNHH Vi Du", "{data:?}");
        assert_eq!(data["f"]["badge"], serde_json::Value::Null);

        // MCP: cùng service, cùng permission.
        let mcp = app.mcp("admin").await.unwrap();
        let result = mcp
            .call_tool(autoxide_testkit::tool(
                "compliance.website_checklist",
                json!({"website_id": id}),
            ))
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true), "{result:?}");
        let text = format!("{result:?}");
        assert!(text.contains("marketplace.seller_info_on_product"), "{text}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ho_so_cua_nguoi_ban_chi_trong_pham_vi_nut() {
    TestDatabase::run(async |test| {
        let seller_a = OrgId::from_uuid(Uuid::from_u128(0xA));
        let seller_b = OrgId::from_uuid(Uuid::from_u128(0xB));
        let grants = |org: OrgId| {
            ["compliance.legal.read", "compliance.legal.manage"]
                .map(|name| (name.to_owned(), Grant::new(org, Scope::Unit)))
        };
        let app = TestApp::builder(test)
            .user("admin", 1, LEGAL)
            .principal("seller_a", Principal::with_grants(Uuid::from_u128(10), grants(seller_a)))
            .provide::<dyn autoxide_address::api::AdministrativeUnits>(Arc::new(
                autoxide_address::api::testing::FakeAdministrativeUnits::sample(),
            ))
            .module(FakeContent)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let tenant = app.default_tenant();
        let transaction = tenant.database().begin_write().await.unwrap();
        let root = OrgTree::insert_root(&transaction, DEFAULT_TENANT).await.unwrap();
        for org in [seller_a, seller_b] {
            OrgTree::insert(&transaction, DEFAULT_TENANT, org, root, 8).await.unwrap();
        }
        transaction.commit().await.unwrap();
        let mut ids = Vec::new();
        for org in [seller_a, seller_b] {
            let mut profile = full_profile();
            profile["org_id"] = json!(org.as_uuid());
            let (status, created) =
                call(&app, "POST", "/legal-profiles", "admin", Some(profile)).await;
            assert_eq!(status, StatusCode::CREATED, "{created}");
            ids.push(created["data"]["id"].as_str().unwrap().to_owned());
        }
        let (_, listed) = call(&app, "GET", "/legal-profiles", "admin", None).await;
        assert_eq!(listed["data"].as_array().unwrap().len(), 2);
        // Người bán A chỉ thấy hồ sơ của nút mình; hồ sơ của B là 404 (không lộ tồn tại).
        let (_, listed) = call(&app, "GET", "/legal-profiles", "seller_a", None).await;
        let listed = listed["data"].as_array().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["org_id"], json!(seller_a.as_uuid()));
        let (status, _) =
            call(&app, "GET", &format!("/legal-profiles/{}", ids[1]), "seller_a", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // A tạo website cho nút mình được, cho nút B thì không.
        let site = json!({"org_id": seller_a.as_uuid(), "domain": "a.example", "kind": "sales"});
        let (status, _) = call(&app, "POST", "/websites", "seller_a", Some(site)).await;
        assert_eq!(status, StatusCode::CREATED);
        let site = json!({"org_id": seller_b.as_uuid(), "domain": "b.example", "kind": "sales"});
        let (status, _) = call(&app, "POST", "/websites", "seller_a", Some(site)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        // Nút không có trong cây → 404.
        let mut profile = full_profile();
        profile["org_id"] = json!(Uuid::from_u128(0xC));
        let (status, _) = call(&app, "POST", "/legal-profiles", "admin", Some(profile)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // Trang công khai của website người bán A lấy hồ sơ của A.
        let answer = app
            .get(&format!("/api/v1/compliance/public/owner?site={}", seller_a.as_uuid()))
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), StatusCode::OK, "{}", answer.text());
    })
    .await
    .unwrap();
}
