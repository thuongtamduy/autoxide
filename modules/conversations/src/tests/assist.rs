//! Trả lời tự động qua `AutoResponder` của `support` (D-230), "gặp nhân viên", bản nháp AI.

use std::sync::Arc;

use autoxide_kernel::ai::TextModel;
use autoxide_support::api::{AutoSource, ReplyDecision, Requester};
use autoxide_testkit::TestDatabase;
use autoxide_testkit::ai::FakeTextModel;
use axum::http::StatusCode;
use serde_json::json;
use uuid::Uuid;

use super::{BUYER, call, harness_with, messages, reply, run_jobs, start};

fn send(text: &str) -> ReplyDecision {
    ReplyDecision::Send { text: text.to_owned(), source: AutoSource::Macro(Uuid::now_v7()) }
}

#[tokio::test]
async fn tu_dong_tra_loi_qua_autoresponder_cua_support() {
    TestDatabase::run(async |test| {
        let h = harness_with(test, true, |builder| builder).await;
        *h.responder.decision.lock().unwrap() = Some(send("Shop mở cửa 8:00–21:00."));
        let id = start(&h.app, "buyer", "Mấy giờ mở cửa?").await;
        run_jobs(&h.app).await;
        run_jobs(&h.app).await;
        let all = messages(&h.app, &id).await;
        assert_eq!(all.len(), 2, "{all:?}");
        assert_eq!(all[1]["author_kind"], "system");
        assert_eq!(all[1]["auto_reply"], true);
        assert_eq!(all[1]["body"], "Shop mở cửa 8:00–21:00.");
        // `support` nhận đúng câu hỏi, người hỏi, luồng.
        let questions = h.responder.questions.lock().unwrap().clone();
        assert_eq!(questions.len(), 1, "the job ran once per message");
        assert_eq!(questions[0].text, "Mấy giờ mở cửa?");
        assert_eq!(questions[0].requester, Requester::User(Uuid::from_u128(BUYER)));
        assert_eq!(questions[0].thread, format!("conversations.thread:{id}"));
        assert_eq!(questions[0].auto_replies_in_row, 0);
        // Câu thứ hai: bộ đếm liên tiếp tăng; nhân viên trả lời thì đếm lại.
        let input = json!({"body": "Chủ nhật có mở không?", "client_id": "q-2"});
        call(&h.app, "POST", &format!("/me/threads/{id}/messages"), "buyer", Some(input)).await;
        run_jobs(&h.app).await;
        assert_eq!(h.responder.questions.lock().unwrap()[1].auto_replies_in_row, 1);
        reply(&h.app, "agent", &id, "Có bạn nhé").await;
        let input = json!({"body": "Cảm ơn", "client_id": "q-3"});
        call(&h.app, "POST", &format!("/me/threads/{id}/messages"), "buyer", Some(input)).await;
        run_jobs(&h.app).await;
        assert_eq!(h.responder.questions.lock().unwrap()[2].auto_replies_in_row, 0);
        // Chuyển người: cuộc đánh dấu chờ nhân viên; bản nháp: lưu cho nhân viên, không gửi.
        *h.responder.decision.lock().unwrap() =
            Some(ReplyDecision::Draft { text: "Bản nháp".to_owned() });
        let input = json!({"body": "Đơn tôi sao rồi?", "client_id": "q-4"});
        call(&h.app, "POST", &format!("/me/threads/{id}/messages"), "buyer", Some(input)).await;
        run_jobs(&h.app).await;
        let (_, staff) = call(&h.app, "GET", &format!("/threads/{id}"), "agent", None).await;
        assert_eq!(staff["data"]["draft_reply"], "Bản nháp");
        let (_, own) = call(&h.app, "GET", &format!("/me/threads/{id}"), "buyer", None).await;
        assert!(own["data"].get("draft_reply").is_none(), "{own}");
        *h.responder.decision.lock().unwrap() = Some(ReplyDecision::Handoff);
        let input = json!({"body": "Tôi muốn khiếu nại", "client_id": "q-5"});
        call(&h.app, "POST", &format!("/me/threads/{id}/messages"), "buyer", Some(input)).await;
        run_jobs(&h.app).await;
        let (_, waiting) = call(&h.app, "GET", "/threads?needs_agent=true", "agent", None).await;
        assert_eq!(waiting["data"].as_array().unwrap().len(), 1);
        let automatic =
            messages(&h.app, &id).await.iter().filter(|m| m["auto_reply"] == true).count();
        assert_eq!(automatic, 3);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gap_nhan_vien_dung_tu_dong_trong_cuoc() {
    TestDatabase::run(async |test| {
        let h = harness_with(test, true, |builder| builder).await;
        *h.responder.decision.lock().unwrap() = Some(send("Trả lời tự động"));
        let id = start(&h.app, "buyer", "Hỏi").await;
        run_jobs(&h.app).await;
        let (status, paused) =
            call(&h.app, "POST", &format!("/me/threads/{id}/handoff"), "buyer", None).await;
        assert_eq!(status, StatusCode::OK, "{paused}");
        assert_eq!(paused["data"]["auto_reply_paused"], true);
        assert_eq!(paused["data"]["needs_agent"], true);
        let input = json!({"body": "Cho gặp người thật", "client_id": "h-2"});
        call(&h.app, "POST", &format!("/me/threads/{id}/messages"), "buyer", Some(input)).await;
        run_jobs(&h.app).await;
        // Chỉ câu đầu được hỏi `support`; sau "gặp nhân viên" không còn tin tự động.
        assert_eq!(h.responder.questions.lock().unwrap().len(), 1);
        let automatic =
            messages(&h.app, &id).await.iter().filter(|m| m["auto_reply"] == true).count();
        assert_eq!(automatic, 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn goi_y_ai_chi_la_ban_nhap() {
    TestDatabase::run(async |test| {
        let model = Arc::new(FakeTextModel::new(|prompt| {
            // Mô hình không thấy số điện thoại của người mua.
            let seen = format!("{:?}", prompt.input);
            assert!(!seen.contains("0901234567"), "{seen}");
            Ok(json!({"reply": "Chào bạn, shop kiểm tra ngay."}))
        }));
        let h =
            harness_with(test, false, move |builder| builder.provide::<dyn TextModel>(model)).await;
        let id = start(&h.app, "buyer", "Gọi tôi số 0901234567 nhé").await;
        let (status, drafted) =
            call(&h.app, "POST", &format!("/threads/{id}/suggest-reply"), "agent", None).await;
        assert_eq!(status, StatusCode::OK, "{drafted}");
        assert_eq!(drafted["data"]["draft_reply"], "Chào bạn, shop kiểm tra ngay.");
        // Không gửi gì cho người mua.
        assert_eq!(messages(&h.app, &id).await.len(), 1);
    })
    .await
    .unwrap();
}
