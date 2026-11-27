//! Gọi điện trong chat (D-232) với `FakeVoiceProvider`: một yêu cầu gọi lại một mục hộp việc, hai
//! nhân viên bấm cùng lúc một cuộc gọi, webhook chống trùng / sai chữ ký / sai thứ tự, ghi âm tắt mặc
//! định và phải xác nhận, số người mua không lộ, bản ghi lưu về rồi xoá ở nhà cung cấp, hết hạn, xoá
//! tài khoản. Bộ giả không có HTTP client: cả luồng chạy không mở kết nối mạng.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_compliance::api::{AccountVerification, VerificationLevel};
use autoxide_kernel::access::{OrgTarget, Principal, Scope};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::sms::PhoneNumber;
use autoxide_kernel::storage::Storage;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, ScopeExt, TenantId, root_of};
use autoxide_kernel::voice::{CallStatus, VoiceProvider};
use autoxide_kernel::work::WorkSource;
use autoxide_testkit::{FAKE_RECORDING, FakeVoiceProvider, MemoryStorage, TestApp, TestDatabase};
use axum::http::StatusCode;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{BUYER, Harness, call, harness_with, messages, run_jobs, start};
use crate::entity::{call as call_entity, callback};

const PHONE: &str = "+84901234567";

/// Số đã xác thực: chỉ người mua `BUYER` có.
struct Accounts;

#[async_trait::async_trait]
impl AccountVerification for Accounts {
    async fn level(&self, _: TenantId, user: Uuid) -> Result<VerificationLevel, Error> {
        Ok(if user == Uuid::from_u128(BUYER) {
            VerificationLevel::Phone
        } else {
            VerificationLevel::None
        })
    }

    async fn verified_phone(&self, _: TenantId, user: Uuid) -> Result<Option<PhoneNumber>, Error> {
        Ok((user == Uuid::from_u128(BUYER)).then(|| PhoneNumber::parse(PHONE, "84").unwrap()))
    }
}

struct Voice {
    h: Harness,
    fake: Arc<FakeVoiceProvider>,
    storage: Arc<MemoryStorage>,
    /// Đường dẫn webhook (dưới `/api/v1/conversations`).
    webhook: String,
}

async fn voice(test: &TestDatabase, notice: bool, recording: bool) -> Voice {
    let fake = Arc::new(FakeVoiceProvider::default().with_recording_notice(notice));
    let storage = Arc::new(MemoryStorage::new());
    let (provider, store) = (fake.clone(), storage.clone());
    let h = harness_with(test, false, move |builder| {
        builder
            .contribute::<dyn VoiceProvider>(provider)
            .provide::<dyn AccountVerification>(Arc::new(Accounts))
            .provide::<dyn Storage>(store)
    })
    .await;
    let root = root_of(DEFAULT_TENANT).as_uuid();
    let line = json!({
        "provider": "fake",
        "hotline": "028 3822 1234",
        "recording_enabled": recording,
        "callback_hours": (1..=7).map(|day| json!({"day": day, "from": "00:00", "to": "23:59"})).collect::<Vec<_>>(),
        "utc_offset_minutes": 0
    });
    let (status, set) =
        call(&h.app, "PUT", &format!("/voice/lines/{root}"), "agent", Some(line)).await;
    assert!(status == StatusCode::OK || !notice || recording, "{set}");
    let webhook = if status == StatusCode::OK {
        set["data"]["webhook_path"].as_str().unwrap().to_owned()
    } else {
        String::new()
    };
    Voice { h, fake, storage, webhook }
}

/// Khung gọi lại ngày mai 10:00–11:00 UTC.
fn window() -> (String, String) {
    let tomorrow = Timestamp::now().date_utc().succ().unwrap().start_of_day_utc()
        + SignedDuration::from_hours(10);
    (tomorrow.to_rfc3339(), (tomorrow + SignedDuration::from_hours(1)).to_rfc3339())
}

async fn request(
    app: &TestApp,
    who: &str,
    id: &str,
    client: &str,
    ack: bool,
) -> (StatusCode, Value) {
    let (from, to) = window();
    let body = json!({"client_id": client, "window_start": from, "window_end": to, "phone_source": "account", "recording_ack": ack});
    call(app, "POST", &format!("/me/threads/{id}/callbacks"), who, Some(body)).await
}

/// Gửi webhook (đã ký) tới đường dẫn của đường dây.
async fn post_webhook(v: &Voice, signed: (Vec<(String, String)>, Vec<u8>)) -> StatusCode {
    let (headers, body) = signed;
    let mut request = v.h.app.post(&format!("/api/v1/conversations{}", v.webhook));
    for (name, value) in headers {
        request = request.header(&name, &value);
    }
    request.header("content-type", "application/json").bytes(body).send().await.unwrap().status()
}

async fn call_back(app: &TestApp, who: &str, callback_id: &str, key: &str) -> (StatusCode, Value) {
    let response = app
        .post(&format!("/api/v1/conversations/callbacks/{callback_id}/call"))
        .token(who)
        .header("idempotency-key", key)
        .send()
        .await
        .unwrap();
    let status = response.status();
    (status, response.json())
}

async fn calls_of(app: &TestApp, id: &str) -> Vec<Value> {
    let (status, got) = call(app, "GET", &format!("/threads/{id}/calls"), "agent", None).await;
    assert_eq!(status, StatusCode::OK, "{got}");
    got["data"].as_array().unwrap().clone()
}

#[tokio::test]
async fn goi_lai_tao_dung_mot_muc_hop_viec() {
    TestDatabase::run(async |test| {
        let v = voice(test, true, false).await;
        let id = start(&v.h.app, "buyer", "Gọi lại giúp mình").await;
        let (status, first) = request(&v.h.app, "buyer", &id, "cb-1", false).await;
        assert_eq!(status, StatusCode::CREATED, "{first}");
        let masked = first["data"]["phone_masked"].as_str().unwrap();
        assert!(masked.ends_with("567") && !masked.contains("901234"), "{masked}");
        assert!(!first.to_string().contains(PHONE));
        // Gửi lại cùng `client_id`, và 10 lần song song: vẫn một yêu cầu.
        let sends: Vec<_> =
            (0..10).map(|_| request(&v.h.app, "buyer", &id, "cb-1", false)).collect();
        for (status, body) in futures_util::future::join_all(sends).await {
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(body["data"]["id"], first["data"]["id"]);
        }
        // Yêu cầu khác khi còn yêu cầu mở: 409.
        let (status, other) = request(&v.h.app, "buyer", &id, "cb-2", false).await;
        assert_eq!(status, StatusCode::CONFLICT, "{other}");
        assert_eq!(other["code"], "conversations.callback_pending");
        let agent = Principal::with_grants(
            Uuid::from_u128(super::AGENT),
            [(
                "conversations.call".to_owned(),
                autoxide_kernel::access::Grant::new(root_of(DEFAULT_TENANT), Scope::Tenant),
            )],
        );
        let tenant = v.h.app.default_tenant();
        let target = OrgTarget::new(root_of(DEFAULT_TENANT), None);
        let source = crate::voice::work::Callbacks;
        assert_eq!(source.count(&tenant, &agent, target).await.unwrap(), 1);
        assert_eq!(source.items(&tenant, &agent, target, 10).await.unwrap().len(), 1);
        let (_, listed) = call(&v.h.app, "GET", "/callbacks?status=open", "agent", None).await;
        assert_eq!(listed["data"].as_array().unwrap().len(), 1);
        assert!(!listed.to_string().contains(PHONE));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nguoi_mua_huy_nhan_vien_dong_va_lay_phien_nghe_may() {
    TestDatabase::run(async |test| {
        let v = voice(test, true, false).await;
        let id = start(&v.h.app, "buyer", "Gọi lại giúp mình").await;
        // Người mua đổi ý: huỷ yêu cầu của mình.
        let (_, made) = request(&v.h.app, "buyer", &id, "cb-1", false).await;
        let cb = made["data"]["id"].as_str().unwrap().to_owned();
        let (status, cancelled) =
            call(&v.h.app, "DELETE", &format!("/me/threads/{id}/callbacks/{cb}"), "buyer", None)
                .await;
        assert_eq!(status, StatusCode::OK, "{cancelled}");
        assert_eq!(cancelled["data"]["status"], "cancelled");
        // Yêu cầu mới; nhân viên đã liên lạc được bằng cách khác nên đóng.
        let (status, made) = request(&v.h.app, "buyer", &id, "cb-2", false).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let cb = made["data"]["id"].as_str().unwrap().to_owned();
        let (status, closed) =
            call(&v.h.app, "POST", &format!("/callbacks/{cb}/close"), "agent", None).await;
        assert_eq!(status, StatusCode::OK, "{closed}");
        assert_eq!(closed["data"]["status"], "done");
        // Nhân viên lấy phiên WebRTC để nghe máy ở đường dây của cửa hàng.
        let (status, session) = call(&v.h.app, "GET", "/voice/agent-session", "agent", None).await;
        assert_eq!(status, StatusCode::OK, "{session}");
        assert!(
            session["data"]["token"].as_str().is_some_and(|token| !token.is_empty()),
            "{session}"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_nhan_vien_bam_goi_cung_luc_chi_mot_cuoc_goi() {
    TestDatabase::run(async |test| {
        let v = voice(test, true, false).await;
        let id = start(&v.h.app, "buyer", "Gọi mình nhé").await;
        let (_, made) = request(&v.h.app, "buyer", &id, "cb-1", false).await;
        let cb = made["data"]["id"].as_str().unwrap().to_owned();
        let (a, b) = tokio::join!(
            call_back(&v.h.app, "agent", &cb, "k-a"),
            call_back(&v.h.app, "agent2", &cb, "k-b")
        );
        let statuses = [a.0, b.0];
        assert!(statuses.contains(&StatusCode::CREATED), "{a:?} {b:?}");
        assert!(statuses.contains(&StatusCode::CONFLICT), "{a:?} {b:?}");
        let loser = if a.0 == StatusCode::CONFLICT { &a.1 } else { &b.1 };
        assert_eq!(loser["code"], "conversations.callback_taken");
        assert_eq!(v.fake.calls().len(), 1);
        assert_eq!(calls_of(&v.h.app, &id).await.len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn so_hien_thi_la_hotline_cua_cua_hang_va_dong_thoi_gian_mot_lan() {
    TestDatabase::run(async |test| {
        let v = voice(test, true, false).await;
        let id = start(&v.h.app, "buyer", "Gọi mình").await;
        let (_, made) = request(&v.h.app, "buyer", &id, "cb-1", false).await;
        let cb = made["data"]["id"].as_str().unwrap().to_owned();
        let (status, placed) = call_back(&v.h.app, "agent", &cb, "k-1").await;
        assert_eq!(status, StatusCode::CREATED, "{placed}");
        let calls = v.fake.calls();
        let root = root_of(DEFAULT_TENANT).as_uuid();
        let (_, line) = call(&v.h.app, "GET", &format!("/voice/lines/{root}"), "agent", None).await;
        // Số người mua thấy là hotline của cửa hàng, không phải số của nhân viên.
        assert_eq!(calls[0].from.as_str(), line["data"]["hotline"].as_str().unwrap());
        assert_eq!(calls[0].agent_identity, Uuid::from_u128(super::AGENT).to_string());
        assert_eq!(calls[0].to.as_str(), PHONE);
        assert!(!calls[0].record);
        let call_id: Uuid = placed["data"]["id"].as_str().unwrap().parse().unwrap();
        // Đổ chuông, nghe máy, kết thúc — rồi giao lại event cuối hai lần.
        assert_eq!(
            post_webhook(&v, v.fake.emit(call_id, CallStatus::Ringing, None, false)).await,
            StatusCode::OK
        );
        assert_eq!(
            post_webhook(&v, v.fake.emit(call_id, CallStatus::InProgress, None, false)).await,
            StatusCode::OK
        );
        let ended = v.fake.emit(call_id, CallStatus::Completed, Some(95), false);
        let (a, b) = tokio::join!(post_webhook(&v, ended.clone()), post_webhook(&v, ended));
        assert_eq!((a, b), (StatusCode::OK, StatusCode::OK));
        run_jobs(&v.h.app).await;
        run_jobs(&v.h.app).await;
        let calls = calls_of(&v.h.app, &id).await;
        assert_eq!(calls[0]["status"], "completed");
        assert_eq!(calls[0]["duration_seconds"], 95);
        let summaries: Vec<_> = messages(&v.h.app, &id)
            .await
            .into_iter()
            .filter(|m| m["call_id"] == calls[0]["id"])
            .collect();
        assert_eq!(summaries.len(), 1, "one system line per call");
        assert_eq!(summaries[0]["author_kind"], "system");
        let (_, list) = call(&v.h.app, "GET", "/callbacks?status=done", "agent", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn webhook_sai_chu_ky_401_khong_luu_va_sai_thu_tu_khong_lui() {
    TestDatabase::run(async |test| {
        let v = voice(test, true, false).await;
        let id = start(&v.h.app, "buyer", "Gọi mình").await;
        let (_, made) = request(&v.h.app, "buyer", &id, "cb-1", false).await;
        let cb = made["data"]["id"].as_str().unwrap().to_owned();
        let (_, placed) = call_back(&v.h.app, "agent", &cb, "k-1").await;
        let call_id: Uuid = placed["data"]["id"].as_str().unwrap().parse().unwrap();
        // Chữ ký sai: 401, không lưu gì.
        let (headers, mut body) = v.fake.emit(call_id, CallStatus::Completed, Some(10), false);
        body.push(b' ');
        assert_eq!(post_webhook(&v, (headers, body)).await, StatusCode::UNAUTHORIZED);
        let stored = crate::entity::voice_event::Entity::find()
            .all(v.h.app.database().read())
            .await
            .unwrap();
        assert!(stored.is_empty());
        // Token lạ: 404.
        let (headers, body) = v.fake.emit(call_id, CallStatus::Ringing, None, false);
        let mut wrong = v.h.app.post("/api/v1/conversations/voice/webhooks/fake/nope");
        for (name, value) in headers {
            wrong = wrong.header(&name, &value);
        }
        assert_eq!(wrong.bytes(body).send().await.unwrap().status(), StatusCode::NOT_FOUND);
        // Tới sai thứ tự: đang nghe máy, rồi "đổ chuông" tới muộn — không lùi.
        post_webhook(&v, v.fake.emit(call_id, CallStatus::InProgress, None, false)).await;
        run_jobs(&v.h.app).await;
        post_webhook(&v, v.fake.emit(call_id, CallStatus::Ringing, None, false)).await;
        run_jobs(&v.h.app).await;
        assert_eq!(calls_of(&v.h.app, &id).await[0]["status"], "in_progress");
        // Trạng thái cuối không đổi nữa.
        post_webhook(&v, v.fake.emit(call_id, CallStatus::NoAnswer, None, false)).await;
        run_jobs(&v.h.app).await;
        post_webhook(&v, v.fake.emit(call_id, CallStatus::Completed, Some(3), false)).await;
        run_jobs(&v.h.app).await;
        assert_eq!(calls_of(&v.h.app, &id).await[0]["status"], "no_answer");
        // Không nghe máy: yêu cầu mở lại, tăng số lần thử.
        let (_, open) = call(&v.h.app, "GET", "/callbacks?status=open", "agent", None).await;
        assert_eq!(open["data"][0]["attempts"], 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ghi_am_tat_mac_dinh_va_phai_xac_nhan() {
    TestDatabase::run(async |test| {
        // Tắt mặc định: đặt đường dây không nêu ghi âm.
        let v = voice(test, true, false).await;
        let id = start(&v.h.app, "buyer", "Hỏi").await;
        let (_, info) = call(&v.h.app, "GET", &format!("/me/threads/{id}/voice"), "buyer", None).await;
        assert_eq!(info["data"]["available"], true);
        assert_eq!(info["data"]["recording"], false);
        assert!(info["data"]["recording_notice"].is_null());
        let root = root_of(DEFAULT_TENANT).as_uuid();
        let (_, line) = call(&v.h.app, "GET", &format!("/voice/lines/{root}"), "agent", None).await;
        assert_eq!(line["data"]["recording_enabled"], false);
        // Bật ghi âm: gọi / xin gọi lại phải kèm xác nhận.
        let on = json!({"provider": "fake", "hotline": "02838221234", "recording_enabled": true, "utc_offset_minutes": 0,
            "callback_hours": (1..=7).map(|day| json!({"day": day, "from": "00:00", "to": "23:59"})).collect::<Vec<_>>()});
        let (status, _) = call(&v.h.app, "PUT", &format!("/voice/lines/{root}"), "agent", Some(on)).await;
        assert_eq!(status, StatusCode::OK);
        let (_, info) = call(&v.h.app, "GET", &format!("/me/threads/{id}/voice"), "buyer", None).await;
        assert_eq!(info["data"]["recording"], true);
        assert!(info["data"]["recording_notice"].as_str().is_some_and(|text| !text.is_empty()));
        let (status, body) = call(&v.h.app, "POST", &format!("/me/threads/{id}/calls"), "buyer", Some(json!({}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["code"], "conversations.recording_notice_required");
        let (status, body) = request(&v.h.app, "buyer", &id, "cb-1", false).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        let (status, started) =
            call(&v.h.app, "POST", &format!("/me/threads/{id}/calls"), "buyer", Some(json!({"recording_ack": true}))).await;
        assert_eq!(status, StatusCode::CREATED, "{started}");
        assert!(started["data"]["token"].as_str().is_some());
        let sessions = v.fake.sessions();
        assert!(sessions[0].recording_notice.is_some());
        assert_eq!(sessions[0].identity, Uuid::from_u128(BUYER).to_string());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nha_cung_cap_khong_phat_thong_bao_thi_khong_bat_ghi_am() {
    TestDatabase::run(async |test| {
        let v = voice(test, false, false).await;
        let root = root_of(DEFAULT_TENANT).as_uuid();
        let on = json!({"provider": "fake", "hotline": "02838221234", "recording_enabled": true});
        let (status, body) =
            call(&v.h.app, "PUT", &format!("/voice/lines/{root}"), "agent", Some(on)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["code"], "conversations.recording_unsupported");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nhan_vien_nhanh_khac_khong_thay_so_nguoi_mua() {
    TestDatabase::run(async |test| {
        let v = voice(test, true, false).await;
        let id = start(&v.h.app, "buyer", "Gọi mình").await;
        let (_, made) = request(&v.h.app, "buyer", &id, "cb-1", false).await;
        let cb = made["data"]["id"].as_str().unwrap().to_owned();
        // REST: nhánh khác không thấy yêu cầu, cuộc gọi, không gọi được.
        let (_, listed) = call(&v.h.app, "GET", "/callbacks", "branch", None).await;
        assert!(listed["data"].as_array().unwrap().is_empty());
        let (status, _) =
            call(&v.h.app, "GET", &format!("/threads/{id}/calls"), "branch", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, body) = call_back(&v.h.app, "branch", &cb, "k-1").await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        // MCP: cùng kết quả.
        let client = v.h.app.mcp("branch").await.unwrap();
        let got = client
            .call_tool(autoxide_testkit::tool("conversations.list_callbacks", json!({})))
            .await
            .unwrap();
        assert!(!format!("{got:?}").contains(&cb), "{got:?}");
        client.cancel().await.unwrap();
        // Event (cũng là nội dung SSE): không bao giờ có số đầy đủ.
        let db = v.h.app.database().read();
        let rows = db
            .query_all_raw(sea_orm::Statement::from_string(
                db.get_database_backend(),
                "SELECT payload FROM conversations_events".to_owned(),
            ))
            .await
            .unwrap();
        for row in rows {
            let payload: String = row.try_get("", "payload").unwrap();
            assert!(!payload.contains("901234567"), "{payload}");
        }
        // Cột lưu số đã mã hoá.
        let stored = callback::Entity::find().all(db).await.unwrap();
        assert!(
            stored[0].phone_sealed.as_deref().is_some_and(|sealed| !sealed.contains("901234567"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn so_chua_xac_thuc_khong_yeu_cau_goi_lai() {
    TestDatabase::run(async |test| {
        let v = voice(test, true, false).await;
        let id = start(&v.h.app, "other", "Gọi mình").await;
        let (status, body) = request(&v.h.app, "other", &id, "cb-1", false).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["code"], "conversations.phone_unverified");
        // Ngoài giờ nhận gọi lại hay quá số ngày: 422.
        let id = start(&v.h.app, "buyer", "Gọi mình").await;
        let late = (Timestamp::now() + SignedDuration::from_hours(24 * 30)).to_rfc3339();
        let body = json!({"client_id": "x", "window_start": late, "window_end": late, "phone_source": "account"});
        let (status, body) = call(&v.h.app, "POST", &format!("/me/threads/{id}/callbacks"), "buyer", Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["code"], "conversations.callback_window_invalid");
    })
    .await
    .unwrap();
}

/// Gọi ra có ghi âm, kết thúc với bản ghi; trả ID cuộc trò chuyện và cuộc gọi.
async fn recorded_call(v: &Voice) -> (String, Uuid) {
    let id = start(&v.h.app, "buyer", "Gọi mình").await;
    let (status, made) = request(&v.h.app, "buyer", &id, "cb-1", true).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let cb = made["data"]["id"].as_str().unwrap().to_owned();
    let (_, placed) = call_back(&v.h.app, "agent", &cb, "k-1").await;
    let call_id: Uuid = placed["data"]["id"].as_str().unwrap().parse().unwrap();
    assert!(v.fake.calls()[0].record);
    assert!(v.fake.calls()[0].recording_notice.is_some());
    post_webhook(v, v.fake.emit(call_id, CallStatus::Completed, Some(40), true)).await;
    run_jobs(&v.h.app).await;
    run_jobs(&v.h.app).await;
    (id, call_id)
}

#[tokio::test]
async fn ban_ghi_luu_vao_storage_va_xoa_o_nha_cung_cap() {
    TestDatabase::run(async |test| {
        let v = voice(test, true, true).await;
        let (id, call_id) = recorded_call(&v).await;
        assert!(!v.fake.has_recording(&format!("rec-{call_id}")), "deleted at the provider");
        let calls = calls_of(&v.h.app, &id).await;
        assert_eq!(calls[0]["recorded"], true);
        let response =
            v.h.app
                .get(&format!("/api/v1/conversations/calls/{call_id}/recording"))
                .token("agent")
                .send()
                .await
                .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.body(), FAKE_RECORDING);
        // Nhánh khác không nghe được.
        let (status, _) =
            call(&v.h.app, "GET", &format!("/calls/{call_id}/recording"), "branch", None).await;
        assert!(status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND);
        // Xoá theo yêu cầu.
        let (status, deleted) =
            call(&v.h.app, "DELETE", &format!("/calls/{call_id}/recording"), "agent", None).await;
        assert_eq!(status, StatusCode::OK, "{deleted}");
        assert_eq!(deleted["data"]["recorded"], false);
        assert!(recordings(&v).await.is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ban_ghi_het_han_thi_xoa() {
    TestDatabase::run(async |test| {
        let v = voice(test, true, true).await;
        let (_, call_id) = recorded_call(&v).await;
        assert_eq!(recordings(&v).await.len(), 1);
        let tenant = v.h.app.default_tenant();
        // Chưa hết hạn: không xoá.
        let storage_service = with_storage(&v);
        assert_eq!(storage_service.purge_recordings(&tenant, Timestamp::now()).await.unwrap(), 0);
        call_entity::Entity::update_many()
            .col_expr(
                call_entity::Column::RecordingExpiresAt,
                Expr::value(Some(Timestamp::now() - SignedDuration::from_hours(24))),
            )
            .filter(call_entity::Column::Id.eq(call_id))
            .scoped(&tenant)
            .exec(tenant.database().write())
            .await
            .unwrap();
        assert_eq!(storage_service.purge_recordings(&tenant, Timestamp::now()).await.unwrap(), 1);
        assert!(recordings(&v).await.is_empty());
    })
    .await
    .unwrap();
}

/// Bản ghi còn ở nhà cung cấp mà bộ nối của nhà cung cấp đó tắt: không xoá dấu vết (`provider_recording_id`), báo
/// lỗi để job thử lại; bật lại bộ nối thì lần sau xoá được (bước 70 P9).
#[tokio::test]
async fn ban_ghi_o_nha_cung_cap_tat_thi_bao_loi_va_giu_dau_vet() {
    TestDatabase::run(async |test| {
        let v = voice(test, true, true).await;
        let (_, call_id) = recorded_call(&v).await;
        let tenant = v.h.app.default_tenant();
        let transaction = tenant.db().begin_write().await.unwrap();
        call_entity::Entity::update_many()
            .col_expr(
                call_entity::Column::RecordingExpiresAt,
                Expr::value(Some(Timestamp::now() - SignedDuration::from_hours(24))),
            )
            .col_expr(call_entity::Column::ProviderRecordingId, Expr::value(Some("rec-left")))
            .filter(call_entity::Column::Id.eq(call_id))
            .scoped(&tenant)
            .exec(&transaction)
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        let provider_of = async || {
            call_entity::Entity::find_by_id(call_id)
                .one(&tenant.db().read())
                .await
                .unwrap()
                .unwrap()
                .provider_recording_id
        };
        // Bộ nối tắt: lỗi, bản ghi trong nơi lưu cũng chưa bị đụng tới.
        let mut off = Arc::try_unwrap(with_storage(&v)).ok().unwrap();
        off.parts.voice = Vec::new();
        let error = off.purge_recordings(&tenant, Timestamp::now()).await.unwrap_err();
        assert_eq!(error.code(), "conversations.voice_provider_unavailable");
        assert_eq!(provider_of().await.as_deref(), Some("rec-left"));
        assert_eq!(recordings(&v).await.len(), 1);
        // Bật lại: xoá ở nơi lưu và ở nhà cung cấp, xoá dấu vết.
        assert_eq!(with_storage(&v).purge_recordings(&tenant, Timestamp::now()).await.unwrap(), 1);
        assert_eq!(provider_of().await, None);
        assert!(recordings(&v).await.is_empty());
        assert!(!v.fake.has_recording("rec-left"));
    })
    .await
    .unwrap();
}

/// Khoá bản ghi âm trong nơi lưu.
async fn recordings(v: &Voice) -> Vec<String> {
    let keys = v.storage.keys().await.unwrap();
    keys.into_iter().filter(|key| key.contains("conversations/recordings/")).collect()
}

/// Service dùng chung nơi lưu và nhà cung cấp giả của `v`.
fn with_storage(v: &Voice) -> Arc<crate::service::ConversationsService> {
    let mut service = Arc::try_unwrap(super::service_of()).ok().unwrap();
    service.parts.storage = Some(v.storage.clone());
    service.parts.voice = vec![v.fake.clone()];
    Arc::new(service)
}

#[tokio::test]
async fn xoa_tai_khoan_xoa_ban_ghi_va_so() {
    TestDatabase::run(async |test| {
        let v = voice(test, true, true).await;
        let (id, _) = recorded_call(&v).await;
        assert_eq!(recordings(&v).await.len(), 1);
        let service = with_storage(&v);
        let event = Delivered::new(
            v.h.app.default_tenant(),
            "auth",
            autoxide_kernel::privacy::USER_DELETED,
            json!({"before": {"id": Uuid::from_u128(BUYER)}}),
        );
        service.handle(&event).await.unwrap();
        service.handle(&event).await.unwrap();
        assert!(recordings(&v).await.is_empty());
        let rows = callback::Entity::find().all(v.h.app.database().read()).await.unwrap();
        assert!(rows.iter().all(|row| row.phone_sealed.is_none() && row.phone_masked.is_empty()));
        assert_eq!(calls_of(&v.h.app, &id).await[0]["recorded"], false);
    })
    .await
    .unwrap();
}

/// Bước 70 P9 (D-20261125-list-pages): cuộc gọi của một cuộc trò chuyện là một trang, mới nhất trước; theo
/// `next_cursor` tới hết không trùng không sót, `order=asc` ngược lại; tool `conversations.list_calls` cùng tham
/// số trả đúng trang REST trả.
#[tokio::test]
async fn cuoc_goi_phan_trang_va_tool_nhu_rest() {
    TestDatabase::run(async |test| {
        let v = voice(test, true, false).await;
        let id = start(&v.h.app, "buyer", "Gọi mình").await;
        let (_, made) = request(&v.h.app, "buyer", &id, "cb-1", false).await;
        let cb = made["data"]["id"].as_str().unwrap().to_owned();
        let (status, placed) = call_back(&v.h.app, "agent", &cb, "k-1").await;
        assert_eq!(status, StatusCode::CREATED, "{placed}");
        // Thêm bốn cuộc gọi cùng cuộc trò chuyện (chép dòng có sẵn, id mới).
        let tenant = v.h.app.default_tenant();
        let first: Uuid = placed["data"]["id"].as_str().unwrap().parse().unwrap();
        let row = call_entity::Entity::find_by_id(first)
            .scoped(&tenant)
            .one(tenant.database().read())
            .await
            .unwrap()
            .unwrap();
        let mut ids = vec![first.to_string()];
        for index in 0..4 {
            let mut copy = row.clone();
            copy.id = Uuid::now_v7();
            copy.callback_id = None;
            copy.provider_call_id = Some(format!("copy-{index}"));
            ids.push(copy.id.to_string());
            sea_orm::ActiveModelTrait::insert(
                call_entity::ActiveModel::from(copy),
                tenant.database().write(),
            )
            .await
            .unwrap();
        }
        let walk = async |order: &str| {
            let mut seen = Vec::new();
            let first = format!("/threads/{id}/calls?limit=2&order={order}");
            let mut uri = first.clone();
            loop {
                let (status, page) = call(&v.h.app, "GET", &uri, "agent", None).await;
                assert_eq!(status, StatusCode::OK, "{page}");
                let rows = page["data"].as_array().unwrap();
                assert!(rows.len() <= 2);
                seen.extend(rows.iter().map(|row| row["id"].as_str().unwrap().to_owned()));
                match page["pagination"]["next_cursor"].as_str() {
                    Some(cursor) => uri = format!("{first}&cursor={cursor}"),
                    None => return seen,
                }
            }
        };
        assert_eq!(walk("asc").await, ids);
        ids.reverse();
        assert_eq!(walk("desc").await, ids);

        let (_, rest) =
            call(&v.h.app, "GET", &format!("/threads/{id}/calls?limit=2"), "agent", None).await;
        let client = v.h.app.mcp("agent").await.unwrap();
        let tool = |arguments: Value| autoxide_testkit::tool("conversations.list_calls", arguments);
        let page = client.call_tool(tool(json!({"id": id, "limit": 2}))).await.unwrap();
        let page = page.structured_content.unwrap();
        assert_eq!(page["data"], rest["data"]);
        assert_eq!(page["pagination"], rest["pagination"]);
        let cursor = page["pagination"]["next_cursor"].clone();
        let next =
            client.call_tool(tool(json!({"id": id, "limit": 2, "cursor": cursor}))).await.unwrap();
        let next = next.structured_content.unwrap();
        let shown: Vec<&str> = next["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect();
        assert_eq!(shown, [ids[2].as_str(), ids[3].as_str()]);
        client.cancel().await.unwrap();
    })
    .await
    .unwrap();
}
