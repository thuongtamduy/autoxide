//! Trả lời tự động (D-230), với `FakeTextModel` trả phân loại định sẵn: chỉ gửi chữ đã duyệt hay mẫu
//! câu của nguồn dữ liệu, luật quyết trong Rust, không bao giờ tự động cho việc rủi ro.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::{Arc, Mutex};

use autoxide_kernel::ai::TextModel;
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::ai::FakeTextModel;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{BUYER, call, harness_with};
use crate::api::{FactIntent, FactSource, Requester, SupportFacts};

const ANSWER: &str = "Chào {customer_name}, cửa hàng mở cửa 8:00–21:00 mỗi ngày.";

/// Đơn `DH1` của người mua `BUYER`: chỉ trả lời cho đúng chủ.
struct Orders;

#[async_trait::async_trait]
impl FactSource for Orders {
    fn intents(&self) -> Vec<FactIntent> {
        vec![FactIntent {
            intent: "orders.status",
            description: "Where is my order?",
            params: json!({"type": "object", "properties": {"number": {"type": "string"}}}),
        }]
    }

    async fn answer(
        &self,
        _: &TenantContext,
        requester: &Requester,
        _: &str,
        params: &Value,
        _: &str,
    ) -> Result<Option<String>, Error> {
        let mine = *requester == Requester::User(Uuid::from_u128(BUYER));
        Ok((mine && params["number"] == "DH1").then(|| "Đơn DH1 đang được giao.".to_owned()))
    }
}

struct Setup {
    app: TestApp,
    /// Phân loại mô hình giả sẽ trả.
    answer: Arc<Mutex<Value>>,
    macro_id: String,
}

async fn setup(test: &TestDatabase, auto_reply: bool, blocked: &[&str]) -> Setup {
    let answer =
        Arc::new(Mutex::new(json!({"kind": "other", "confidence_bp": 0, "risk_flags": []})));
    let shared = answer.clone();
    let model = Arc::new(FakeTextModel::new(move |prompt| {
        if prompt.purpose == "support.reply_suggestion" {
            return Ok(json!({"reply": "Bản nháp"}));
        }
        Ok(shared.lock().unwrap().clone())
    }));
    let harness = harness_with(test, move |builder| builder.provide::<dyn TextModel>(model)).await;
    let app = harness.app;
    app.api::<dyn SupportFacts>().unwrap().attach(Arc::new(Orders)).unwrap();
    let root = app.default_tenant().id().as_uuid();
    let (_, policy) = call(&app, "GET", &format!("/policies/{root}"), "agent", None).await;
    let mut policy = policy["data"].clone();
    for field in ["org_id", "defined_at"] {
        policy.as_object_mut().unwrap().remove(field);
    }
    policy["auto_reply"] = json!(auto_reply);
    policy["auto_reply_blocked_terms"] = json!(blocked);
    let (status, body) =
        call(&app, "PUT", &format!("/policies/{root}"), "agent", Some(policy)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let body = json!({"org_id": root, "name": "Giờ mở cửa", "bodies": {"vi": ANSWER, "en": "We open 8:00–21:00."}});
    let (_, saved) = call(&app, "POST", "/macros", "agent", Some(body)).await;
    let macro_id = saved["data"]["id"].as_str().unwrap().to_owned();
    let due = (Timestamp::now() + SignedDuration::from_hours(24 * 30)).date_utc().to_string();
    let approve =
        json!({"enabled": true, "questions": {"vi": ["Mấy giờ mở cửa?"]}, "review_due_on": due});
    let (status, body) =
        call(&app, "PUT", &format!("/macros/{macro_id}/auto-reply"), "agent", Some(approve)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    Setup { app, answer, macro_id }
}

impl Setup {
    fn classify(&self, value: Value) {
        *self.answer.lock().unwrap() = value;
    }

    fn macro_answer(&self, confidence: i64) -> Value {
        json!({"kind": "answer", "macro_id": self.macro_id, "confidence_bp": confidence, "risk_flags": []})
    }

    async fn ask(&self, text: &str) -> String {
        let body = json!({"category": "other", "subject": "Hỏi", "body": text, "name": "An", "locale": "vi"});
        let (status, opened) =
            call(&self.app, "POST", "/public/tickets", "buyer", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{opened}");
        let id = opened["data"]["id"].as_str().unwrap().to_owned();
        self.run_jobs().await;
        id
    }

    async fn write(&self, id: &str, text: &str) {
        let body = json!({"body": text});
        let (status, _) =
            call(&self.app, "POST", &format!("/me/tickets/{id}/messages"), "buyer", Some(body))
                .await;
        assert_eq!(status, StatusCode::CREATED);
        self.run_jobs().await;
    }

    async fn run_jobs(&self) {
        let worker = self.app.modules().worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
        for _ in 0..10 {
            if worker.run_once().await.unwrap() == 0 {
                break;
            }
        }
    }

    /// Tin tự động của phiếu.
    async fn automatic(&self, id: &str) -> Vec<Value> {
        let (_, got) = call(&self.app, "GET", &format!("/me/tickets/{id}"), "buyer", None).await;
        got["data"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["auto_reply"] == true)
            .cloned()
            .collect()
    }
}

#[tokio::test]
async fn tu_dong_tat_mac_dinh() {
    TestDatabase::run(async |test| {
        let setup = setup(test, false, &[]).await;
        setup.classify(setup.macro_answer(9900));
        let id = setup.ask("Mấy giờ mở cửa?").await;
        assert!(setup.automatic(&id).await.is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chi_gui_nguyen_van_cau_tra_loi_da_duyet() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true, &[]).await;
        // Mô hình chỉ trả phân loại (schema không có chỗ cho chữ trả lời): chữ gửi đi là câu trả lời
        // mẫu nguyên văn, đã điền biến.
        setup.classify(setup.macro_answer(9900));
        let id = setup.ask("Mấy giờ mở cửa?").await;
        let sent = setup.automatic(&id).await;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["body"], "Chào An, cửa hàng mở cửa 8:00–21:00 mỗi ngày.");
        assert_eq!(sent[0]["author_kind"], "system");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn do_tin_cay_thap_thanh_ban_nhap() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true, &[]).await;
        setup.classify(setup.macro_answer(5000));
        let id = setup.ask("Mấy giờ mở cửa?").await;
        assert!(setup.automatic(&id).await.is_empty());
        let (_, drafts) =
            call(&setup.app, "GET", &format!("/tickets/{id}/reply-drafts"), "agent", None).await;
        assert_eq!(drafts["data"].as_array().unwrap().len(), 1, "{drafts}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khieu_nai_hoan_tien_khong_bao_gio_tu_dong() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true, &[]).await;
        // Mô hình tự tin, không gắn cờ — luật Rust vẫn chặn theo từ khoá.
        setup.classify(setup.macro_answer(10_000));
        let id = setup.ask("Tôi muốn hoàn tiền đơn hôm qua").await;
        assert!(setup.automatic(&id).await.is_empty());
        // Mô hình gắn cờ rủi ro dù không có từ khoá.
        let mut flagged = setup.macro_answer(10_000);
        flagged["risk_flags"] = json!(["angry"]);
        setup.classify(flagged);
        let id = setup.ask("Mấy giờ mở cửa??? Chán quá").await;
        assert!(setup.automatic(&id).await.is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tu_khoa_chan_cua_nut_chan_tu_dong() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true, &["bảo hành"]).await;
        setup.classify(setup.macro_answer(9900));
        let id = setup.ask("Mấy giờ mở cửa để bảo hành?").await;
        assert!(setup.automatic(&id).await.is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn fact_don_cua_nguoi_khac_tra_none() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true, &[]).await;
        setup.classify(
            json!({"kind": "fact", "intent": "orders.status", "params": {"number": "DH1"},
                              "confidence_bp": 9900, "risk_flags": []}),
        );
        let id = setup.ask("Đơn DH1 tới đâu rồi?").await;
        assert_eq!(setup.automatic(&id).await[0]["body"], "Đơn DH1 đang được giao.");
        // Người khác hỏi cùng đơn: nguồn trả `None`, không gửi gì.
        let body = json!({"category": "other", "subject": "Hỏi", "body": "Đơn DH1 tới đâu rồi?"});
        let (_, opened) = call(&setup.app, "POST", "/public/tickets", "other", Some(body)).await;
        setup.run_jobs().await;
        let other = opened["data"]["id"].as_str().unwrap();
        let (_, got) =
            call(&setup.app, "GET", &format!("/me/tickets/{other}"), "other", None).await;
        assert!(
            got["data"]["messages"].as_array().unwrap().iter().all(|m| m["auto_reply"] == false)
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn qua_so_lan_lien_tiep_chuyen_nguoi() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true, &[]).await;
        setup.classify(setup.macro_answer(9900));
        let id = setup.ask("Mấy giờ mở cửa?").await;
        setup.write(&id, "Mấy giờ mở cửa vậy?").await;
        setup.write(&id, "Mấy giờ mở cửa ạ?").await;
        // Mặc định tối đa 2 câu tự động liên tiếp (tới khi nhân viên trả lời): câu thứ ba tới người.
        assert_eq!(setup.automatic(&id).await.len(), 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bam_gap_nhan_vien_dung_tu_dong() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true, &[]).await;
        setup.classify(setup.macro_answer(9900));
        let id = setup.ask("Mấy giờ mở cửa?").await;
        let (status, ticket) = call(
            &setup.app,
            "POST",
            &format!("/me/tickets/{id}/handoff"),
            "buyer",
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{ticket}");
        setup.write(&id, "Mấy giờ mở cửa?").await;
        assert_eq!(setup.automatic(&id).await.len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bao_sai_du_nguong_thoi_dung_cau_tra_loi() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true, &[]).await;
        setup.classify(setup.macro_answer(9900));
        for _ in 0..3 {
            let id = setup.ask("Mấy giờ mở cửa?").await;
            let sent = setup.automatic(&id).await;
            let reply = sent.first().map(|m| m["id"].clone());
            // `id` của tin; nhật ký có `reply_message_id` trỏ tới nó.
            let (_, log) = call(&setup.app, "GET", "/auto-replies", "agent", None).await;
            let entry = log["data"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| Some(e["reply_message_id"].clone()) == reply)
                .unwrap()
                .clone();
            let path =
                format!("/me/tickets/{id}/auto-replies/{}/feedback", entry["id"].as_str().unwrap());
            let (status, _) =
                call(&setup.app, "POST", &path, "buyer", Some(json!({"helpful": false}))).await;
            assert_eq!(status, StatusCode::OK);
        }
        let id = setup.ask("Mấy giờ mở cửa?").await;
        assert!(
            setup.automatic(&id).await.is_empty(),
            "the saved reply stopped after 3 wrong reports"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nguoi_mua_bao_phan_hoi_bang_id_tin_ho_thay() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true, &[]).await;
        setup.classify(setup.macro_answer(9900));
        let id = setup.ask("Mấy giờ mở cửa?").await;
        let sent = setup.automatic(&id).await;
        // Người mua chỉ thấy tin (cờ `auto_reply`), không đọc được nhật ký của nhân viên.
        let message = sent[0]["id"].as_str().unwrap().to_owned();
        let path = format!("/me/tickets/{id}/auto-replies/{message}/feedback");
        let (status, body) =
            call(&setup.app, "POST", &path, "buyer", Some(json!({"helpful": true}))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["data"]["feedback"], "helpful");
        assert_eq!(body["data"]["reply_message_id"], message.as_str());
        // Id lạ vẫn là không tìm thấy.
        let path = format!("/me/tickets/{id}/auto-replies/{}/feedback", Uuid::now_v7());
        let (status, body) =
            call(&setup.app, "POST", &path, "buyer", Some(json!({"helpful": true}))).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert_eq!(body["code"], "support.auto_reply_not_found");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn het_han_ra_lai_thoi_dung() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true, &[]).await;
        setup.classify(setup.macro_answer(9900));
        // Ngày phải rà lại đã qua.
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
        crate::entity::saved_reply::Entity::update_many()
            .col_expr(
                crate::entity::saved_reply::Column::ReviewDueOn,
                sea_orm::sea_query::Expr::value(
                    (Timestamp::now() - SignedDuration::from_hours(24)).date_utc().to_string(),
                ),
            )
            .filter(
                crate::entity::saved_reply::Column::Id.eq(setup.macro_id.parse::<Uuid>().unwrap()),
            )
            .exec(setup.app.database().write())
            .await
            .unwrap();
        let id = setup.ask("Mấy giờ mở cửa?").await;
        assert!(setup.automatic(&id).await.is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn xem_thu_khong_gui() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true, &[]).await;
        setup.classify(setup.macro_answer(9900));
        let root = setup.app.default_tenant().id().as_uuid();
        let (status, decision) = call(
            &setup.app,
            "POST",
            "/auto-reply/preview",
            "agent",
            Some(json!({"org_id": root, "text": "Mấy giờ mở cửa?"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{decision}");
        assert_eq!(decision["data"]["outcome"], "send");
        assert_eq!(decision["data"]["macro_id"], setup.macro_id.as_str());
        let (_, log) = call(&setup.app, "GET", "/auto-replies", "agent", None).await;
        assert_eq!(log["data"].as_array().unwrap().len(), 0, "nothing was sent");
        let (status, _) = call(
            &setup.app,
            "POST",
            "/auto-reply/preview",
            "buyer",
            Some(json!({"org_id": root, "text": "x"})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gui_lai_job_khong_tra_loi_hai_lan() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true, &[]).await;
        setup.classify(setup.macro_answer(9900));
        let id = setup.ask("Mấy giờ mở cửa?").await;
        let service = super::service_of(&setup.app);
        let (_, got) = call(&setup.app, "GET", &format!("/tickets/{id}"), "agent", None).await;
        let first = got["data"]["messages"][0]["id"].as_str().unwrap().parse().unwrap();
        // Job chạy lại cho cùng tin (sau lỗi giữa chừng, hay hai worker).
        let tenant = setup.app.default_tenant();
        service.auto_reply(&tenant, id.parse().unwrap(), first).await.unwrap();
        service.auto_reply(&tenant, id.parse().unwrap(), first).await.unwrap();
        assert_eq!(setup.automatic(&id).await.len(), 1);
    })
    .await
    .unwrap();
}
