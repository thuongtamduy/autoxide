//! Liên kết bản ghi, câu trả lời mẫu, chuyển khiếu nại, mức hài lòng (bước 53 mục 4).

use std::sync::{Arc, Mutex};

use autoxide_compliance::api::{CaseRef, ComplaintFromTicket, ComplaintIntake};
use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::resource::{Resource, ResourceResolver};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::json;
use uuid::Uuid;

use super::{Harness, call, harness_with};

/// Khiếu nại giả: một vụ việc cho mỗi `source_ref`, như `compliance` thật.
#[derive(Default)]
pub(crate) struct FakeComplaints(pub Mutex<Vec<(String, Uuid)>>);

#[async_trait::async_trait]
impl ComplaintIntake for FakeComplaints {
    async fn open(
        &self,
        _: &TenantContext,
        complaint: ComplaintFromTicket,
    ) -> Result<CaseRef, Error> {
        let mut cases = self.0.lock().unwrap();
        if let Some((_, id)) = cases.iter().find(|(source, _)| *source == complaint.source_ref) {
            return Ok(CaseRef { id: *id, created: false });
        }
        let id = Uuid::now_v7();
        cases.push((complaint.source_ref, id));
        Ok(CaseRef { id, created: true })
    }
}

/// Module giả `shop` có đơn hàng `shop.order:1`.
struct Shop;

struct Orders;

#[async_trait::async_trait]
impl ResourceResolver for Orders {
    async fn resolve(&self, _: &TenantContext, id: &str) -> Result<Option<Resource>, Error> {
        Ok((id == "1").then(|| Resource::new("Order 1")))
    }
}

impl Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.resource_type("shop.order", Arc::new(Orders))
    }
}

async fn app(test: &TestDatabase) -> (Harness, Arc<FakeComplaints>) {
    let complaints = Arc::new(FakeComplaints::default());
    let provided = complaints.clone();
    let harness = harness_with(test, move |builder| {
        builder.provide::<dyn ComplaintIntake>(provided).module(Shop)
    })
    .await;
    (harness, complaints)
}

async fn open(app: &TestApp) -> String {
    let body = json!({"category": "order", "subject": "Hàng lỗi", "body": "Áo bị rách, tôi muốn khiếu nại.", "name": "An", "locale": "vi"});
    let (status, opened) = call(app, "POST", "/public/tickets", "buyer", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{opened}");
    opened["data"]["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn chuyen_khieu_nai_goi_lai_khong_tao_hai_vu_viec() {
    TestDatabase::run(async |test| {
        let (harness, complaints) = app(test).await;
        let app = &harness.app;
        let id = open(app).await;
        let path = format!("/tickets/{id}/escalate");
        let body = json!({"to": "consumer_complaint"});
        let (status, first) = call(app, "POST", &path, "agent", Some(body.clone())).await;
        assert_eq!(status, StatusCode::OK, "{first}");
        assert_eq!(
            (first["data"]["status"].as_str(), first["data"]["resolution"].as_str()),
            (Some("closed"), Some("escalated"))
        );
        let case = first["data"]["case_id"].clone();
        assert!(case.is_string());
        // Gọi lại: không vụ việc thứ hai, phiếu giữ nguyên vụ cũ.
        let (status, again) = call(app, "POST", &path, "agent", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{again}");
        assert_eq!(again["data"]["case_id"], case);
        assert_eq!(complaints.0.lock().unwrap().len(), 1);
        // Người mua không chuyển được.
        let (status, _) = call(app, "POST", &path, "buyer", Some(json!({"to": "platform"}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Lên sàn: phiếu vẫn mở, một lần.
        let other = open(app).await;
        let path = format!("/tickets/{other}/escalate");
        let (status, up) = call(app, "POST", &path, "agent", Some(json!({"to": "platform"}))).await;
        assert_eq!(status, StatusCode::OK, "{up}");
        assert_eq!(up["data"]["escalated_to_platform"], true);
        let (status, problem) =
            call(app, "POST", &path, "agent", Some(json!({"to": "platform"}))).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("support.already_escalated"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn lien_ket_mau_cau_va_hai_long() {
    TestDatabase::run(async |test| {
        let (harness, _) = app(test).await;
        let app = &harness.app;
        let id = open(app).await;

        // Liên kết qua sổ đối tượng.
        let links = format!("/tickets/{id}/links");
        let (status, problem) =
            call(app, "POST", &links, "agent", Some(json!({"target": "shop.order:2"}))).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("support.link_not_found"))
        );
        let (status, linked) =
            call(app, "POST", &links, "agent", Some(json!({"target": "shop.order:1"}))).await;
        assert_eq!(status, StatusCode::OK, "{linked}");
        assert_eq!(linked["data"], json!(["shop.order:1"]));
        let (_, found) = call(app, "GET", "/tickets?link=shop.order:1", "agent", None).await;
        assert_eq!(found["data"].as_array().unwrap().len(), 1);
        let (_, detail) = call(app, "GET", &format!("/tickets/{id}"), "agent", None).await;
        assert_eq!(detail["data"]["links"], json!(["shop.order:1"]));
        let (status, left) =
            call(app, "DELETE", &format!("{links}/shop.order:1"), "agent", None).await;
        assert_eq!((status, left["data"].clone()), (StatusCode::OK, json!([])));

        // Câu trả lời mẫu có biến, theo ngôn ngữ của người mua.
        let root = app.default_tenant().id().as_uuid();
        let body = json!({"org_id": root, "name": "Đổi hàng", "bodies": {
            "vi": "Chào {customer_name}, phiếu {ticket_number} đã được ghi nhận.",
            "en": "Hello {customer_name}, ticket {ticket_number} is noted."}});
        let (status, saved) = call(app, "POST", "/macros", "agent", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{saved}");
        let macro_id = saved["data"]["id"].as_str().unwrap();
        let (status, rendered) =
            call(app, "POST", &format!("/tickets/{id}/macros/{macro_id}/render"), "agent", None)
                .await;
        assert_eq!(status, StatusCode::OK, "{rendered}");
        assert_eq!(rendered["data"]["body"], "Chào An, phiếu 000001 đã được ghi nhận.");
        let (_, listed) = call(app, "GET", &format!("/macros?org_id={root}"), "agent", None).await;
        assert_eq!(listed["data"].as_array().unwrap().len(), 1);

        // Hài lòng: chỉ khi đã xử lý.
        let rate = format!("/me/tickets/{id}/satisfaction");
        let (status, problem) = call(app, "POST", &rate, "buyer", Some(json!({"rating": 5}))).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("support.not_solved"))
        );
        call(app, "PATCH", &format!("/tickets/{id}"), "agent", Some(json!({"status": "solved"})))
            .await;
        let (status, _) = call(app, "POST", &rate, "buyer", Some(json!({"rating": 9}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, rated) =
            call(app, "POST", &rate, "buyer", Some(json!({"rating": 4, "note": "Ổn"}))).await;
        assert_eq!(status, StatusCode::OK, "{rated}");
        assert_eq!(rated["data"]["satisfaction"], 4);
        let (status, _) = call(app, "POST", &rate, "other", Some(json!({"rating": 1}))).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}
