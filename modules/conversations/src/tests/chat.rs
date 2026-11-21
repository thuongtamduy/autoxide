//! Cuộc trò chuyện và tin nhắn: gửi lại không trùng, phạm vi theo nhánh, thu hồi, đã đọc, phân công,
//! đóng, chặn, thông báo gom lô, chuyển phiếu qua `support` thật, tool MCP.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::tenancy::ScopeExt;
use autoxide_support::api::{NewTicket, Requester, SupportIntake, TicketRef};
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::json;
use uuid::Uuid;

use super::{AGENT, BUYER, call, due_now, harness, harness_with, messages, reply, run_jobs, start};
use crate::entity::message;

#[tokio::test]
async fn gui_lai_cung_client_id_khong_trung() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        let first = json!({"body": "Còn size 42 không?", "client_id": "m-1"});
        let (status, started) =
            call(&h.app, "POST", "/me/threads", "buyer", Some(first.clone())).await;
        assert_eq!(status, StatusCode::CREATED, "{started}");
        let id = started["data"]["thread"]["id"].as_str().unwrap().to_owned();
        // Gửi lại y hệt (mất mạng, retry): cùng cuộc, cùng tin, 200.
        let (status, again) = call(&h.app, "POST", "/me/threads", "buyer", Some(first)).await;
        assert_eq!(status, StatusCode::OK, "{again}");
        assert_eq!(again["data"]["thread"]["id"], id.as_str());
        assert_eq!(again["data"]["message"]["id"], started["data"]["message"]["id"]);
        // Mười lần gửi song song cùng `client_id` vào cuộc: một tin.
        let uri = format!("/me/threads/{id}/messages");
        let body = json!({"body": "Alo?", "client_id": "m-2"});
        let sends = (0..10).map(|_| call(&h.app, "POST", &uri, "buyer", Some(body.clone())));
        let results = futures_join(sends).await;
        let created = results.iter().filter(|(status, _)| *status == StatusCode::CREATED).count();
        assert_eq!(created, 1, "{results:?}");
        assert!(results.iter().all(|(status, _)| status.is_success()), "{results:?}");
        assert_eq!(messages(&h.app, &id).await.len(), 2);
        // Tin mới khác `client_id` vào cùng cuộc đang mở, không mở cuộc mới.
        let next = json!({"body": "Mình hỏi thêm", "client_id": "m-3"});
        let (status, more) = call(&h.app, "POST", "/me/threads", "buyer", Some(next)).await;
        assert_eq!(status, StatusCode::OK, "{more}");
        assert_eq!(more["data"]["thread"]["id"], id.as_str());
        let (_, inbox) = call(&h.app, "GET", "/threads", "agent", None).await;
        assert_eq!(inbox["data"].as_array().unwrap().len(), 1);
        assert_eq!(inbox["data"][0]["unread"], 3);
        // Tiền tố dành cho tin hệ thống thì không dùng được.
        let bad = json!({"body": "x", "client_id": "auto:1"});
        let (status, _) = call(&h.app, "POST", &uri, "buyer", Some(bad)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

async fn futures_join<F: std::future::Future>(futures: impl Iterator<Item = F>) -> Vec<F::Output> {
    let handles: Vec<_> = futures.collect();
    let mut out = Vec::new();
    // `join_all` tự viết: chạy đồng thời trên cùng task bằng `tokio::join!` từng cặp là đủ cho test.
    let mut set = Vec::new();
    for future in handles {
        set.push(Box::pin(future));
    }
    let results = join_all(set).await;
    out.extend(results);
    out
}

async fn join_all<F: std::future::Future + Unpin>(mut futures: Vec<F>) -> Vec<F::Output> {
    use std::task::Poll;
    let mut results: Vec<Option<F::Output>> = futures.iter().map(|_| None).collect();
    std::future::poll_fn(|context| {
        let mut pending = false;
        for (index, future) in futures.iter_mut().enumerate() {
            if results[index].is_none() {
                match std::pin::Pin::new(future).poll(context) {
                    Poll::Ready(value) => results[index] = Some(value),
                    Poll::Pending => pending = true,
                }
            }
        }
        if pending { Poll::Pending } else { Poll::Ready(()) }
    })
    .await;
    results.into_iter().map(Option::unwrap).collect()
}

#[tokio::test]
async fn nhan_vien_nhanh_khac_khong_doc_duoc() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        let id = start(&h.app, "buyer", "Đơn của tôi đâu?").await;
        // Nhân viên nhánh khác: không thấy trong hộp thư, đọc / trả lời → 404 như không có.
        let (status, inbox) = call(&h.app, "GET", "/threads", "branch", None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(inbox["data"].as_array().unwrap().is_empty());
        for (method, uri) in
            [("GET", format!("/threads/{id}")), ("GET", format!("/threads/{id}/messages"))]
        {
            let (status, body) = call(&h.app, method, &uri, "branch", None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri} {body}");
            assert_eq!(body["code"], "conversations.thread_not_found");
        }
        let input = json!({"body": "x", "client_id": "b-1"});
        let (status, _) =
            call(&h.app, "POST", &format!("/threads/{id}/messages"), "branch", Some(input)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // Người mua khác cũng không thấy.
        let (status, _) = call(&h.app, "GET", &format!("/me/threads/{id}"), "other", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // Người mua không vào được hộp thư nhân viên.
        let (status, _) = call(&h.app, "GET", "/threads", "buyer", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn thu_hoi_qua_hai_phut_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        let id = start(&h.app, "buyer", "Nhầm").await;
        let first = messages(&h.app, &id).await[0]["id"].as_str().unwrap().to_owned();
        let reply = reply(&h.app, "agent", &id, "Dạ").await;
        // Không thu hồi được tin của người khác.
        let uri = format!("/me/threads/{id}/messages/{}/recall", reply["id"].as_str().unwrap());
        let (status, _) = call(&h.app, "POST", &uri, "buyer", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // Trong 2 phút: thu hồi được, giữ chỗ.
        let uri = format!("/me/threads/{id}/messages/{first}/recall");
        let (status, recalled) = call(&h.app, "POST", &uri, "buyer", None).await;
        assert_eq!(status, StatusCode::OK, "{recalled}");
        assert_eq!(recalled["data"]["recalled"], true);
        assert_eq!(recalled["data"]["body"], "");
        // Tin gửi quá 2 phút trước: 409.
        let input = json!({"body": "Tin cũ", "client_id": "old"});
        let (_, old) =
            call(&h.app, "POST", &format!("/me/threads/{id}/messages"), "buyer", Some(input)).await;
        let old_id: Uuid = old["data"]["id"].as_str().unwrap().parse().unwrap();
        let tenant = h.app.default_tenant();
        message::Entity::update_many()
            .col_expr(
                message::Column::CreatedAt,
                Expr::value(Timestamp::now() - SignedDuration::from_secs(121)),
            )
            .filter(message::Column::Id.eq(old_id))
            .scoped(&tenant)
            .exec(tenant.database().write())
            .await
            .unwrap();
        let uri = format!("/me/threads/{id}/messages/{old_id}/recall");
        let (status, body) = call(&h.app, "POST", &uri, "buyer", None).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["code"], "conversations.recall_expired");
        let all = messages(&h.app, &id).await;
        assert_eq!(all.len(), 3);
        assert_eq!(all[2]["body"], "Tin cũ");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn da_doc_phan_cong_dong_va_chan() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        let id = start(&h.app, "buyer", "Chào shop").await;
        let (_, unread) = call(&h.app, "GET", "/threads?unread=true", "agent", None).await;
        assert_eq!(unread["data"].as_array().unwrap().len(), 1);
        let (status, read) =
            call(&h.app, "POST", &format!("/threads/{id}/read"), "agent", Some(json!({}))).await;
        assert_eq!(status, StatusCode::OK, "{read}");
        assert_eq!(read["data"]["unread"], 0);
        let (_, got) = call(&h.app, "GET", &format!("/me/threads/{id}"), "buyer", None).await;
        assert_eq!(got["data"]["reads"][0]["user_id"], Uuid::from_u128(AGENT).to_string());
        // Nhận về mình, rồi giao cho đồng nghiệp.
        let me = json!({"assignee_user_id": Uuid::from_u128(AGENT).to_string()});
        let (status, assigned) =
            call(&h.app, "POST", &format!("/threads/{id}/assign"), "agent", Some(me)).await;
        assert_eq!(status, StatusCode::OK, "{assigned}");
        let (_, mine) = call(&h.app, "GET", "/threads?assignee=me", "agent", None).await;
        assert_eq!(mine["data"].as_array().unwrap().len(), 1);
        reply(&h.app, "agent", &id, "Shop chào bạn").await;
        let (_, got) = call(&h.app, "GET", &format!("/me/threads/{id}"), "buyer", None).await;
        assert_eq!(got["data"]["unread"], 1);
        // Chặn: người mua không viết được nữa; gỡ chặn thì viết lại được.
        let (status, _) = call(
            &h.app,
            "PUT",
            &format!("/threads/{id}/block"),
            "agent",
            Some(json!({"reason": "spam"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let input = json!({"body": "Alo", "client_id": "x-1"});
        let uri = format!("/me/threads/{id}/messages");
        let (status, body) = call(&h.app, "POST", &uri, "buyer", Some(input.clone())).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["code"], "conversations.blocked");
        let (status, _) =
            call(&h.app, "DELETE", &format!("/threads/{id}/block"), "agent", None).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = call(&h.app, "POST", &uri, "buyer", Some(input)).await;
        assert_eq!(status, StatusCode::CREATED);
        // Báo vi phạm và xử lý.
        let (status, report) = call(
            &h.app,
            "POST",
            &format!("/me/threads/{id}/reports"),
            "buyer",
            Some(json!({"reason": "Lời lẽ khiếm nhã"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{report}");
        let (_, reports) = call(&h.app, "GET", "/reports?status=open", "agent", None).await;
        assert_eq!(reports["data"].as_array().unwrap().len(), 1);
        let report_id = report["data"]["id"].as_str().unwrap();
        let (status, _) =
            call(&h.app, "POST", &format!("/reports/{report_id}/resolve"), "agent", None).await;
        assert_eq!(status, StatusCode::OK);
        // Đóng: không viết được nữa; viết lại thì thành cuộc mới.
        let (status, _) =
            call(&h.app, "POST", &format!("/threads/{id}/close"), "agent", None).await;
        assert_eq!(status, StatusCode::OK);
        let (status, body) = call(
            &h.app,
            "POST",
            &uri,
            "buyer",
            Some(json!({"body": "Còn đó không", "client_id": "x-2"})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["code"], "conversations.thread_closed");
        let next = start(&h.app, "buyer", "Mình hỏi lại").await;
        assert_ne!(next, id);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tin_chua_doc_gui_thong_bao_mot_lan() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        let id = start(&h.app, "buyer", "Hỏi").await;
        let assign = json!({"assignee_user_id": Uuid::from_u128(AGENT).to_string()});
        call(&h.app, "POST", &format!("/threads/{id}/assign"), "agent", Some(assign)).await;
        // Ba tin của nhân viên trong cùng khung: một thông báo cho người mua.
        for text in ["Một", "Hai", "Ba"] {
            reply(&h.app, "agent", &id, text).await;
        }
        // Chưa tới hạn: chưa báo.
        run_jobs(&h.app).await;
        assert!(h.notifier.sent().iter().all(|sent| sent.kind != "conversations.shop_replied"));
        due_now(&h.app).await;
        run_jobs(&h.app).await;
        run_jobs(&h.app).await;
        let to_buyer: Vec<_> = h
            .notifier
            .sent()
            .into_iter()
            .filter(|sent| sent.kind == "conversations.shop_replied")
            .collect();
        assert_eq!(to_buyer.len(), 1, "{to_buyer:?}");
        assert_eq!(
            to_buyer[0].recipient,
            autoxide_notifications::api::Recipient::User(Uuid::from_u128(BUYER))
        );
        // Người mua đọc rồi thì tin sau đó, khi tới hạn, không báo nữa.
        let input = json!({"body": "Bốn", "client_id": "r-4"});
        call(&h.app, "POST", &format!("/threads/{id}/messages"), "agent", Some(input)).await;
        call(&h.app, "POST", &format!("/me/threads/{id}/read"), "buyer", Some(json!({}))).await;
        due_now(&h.app).await;
        run_jobs(&h.app).await;
        let count = h
            .notifier
            .sent()
            .iter()
            .filter(|sent| sent.kind == "conversations.shop_replied")
            .count();
        assert_eq!(count, 1);
    })
    .await
    .unwrap();
}

/// Mở phiếu giả: đứng chỗ `SupportIntake` của `support` (bài đầu-cuối với `support` thật ở
/// `crates/app/src/e2e/conversations.rs`).
#[derive(Default)]
struct FakeIntake(std::sync::Mutex<Vec<NewTicket>>);

#[async_trait::async_trait]
impl SupportIntake for FakeIntake {
    async fn open(
        &self,
        _tenant: &autoxide_kernel::tenancy::TenantContext,
        ticket: NewTicket,
    ) -> Result<TicketRef, autoxide_kernel::error::Error> {
        let mut opened = self.0.lock().unwrap();
        // Một phiếu cho mỗi `reference`: test chỉ chuyển một cuộc.
        let id = Uuid::from_u128(0x7e);
        if !opened.iter().any(|known| known.reference == ticket.reference) {
            opened.push(ticket);
        }
        Ok(TicketRef { id, number: "000001".to_owned(), access_token: None })
    }
}

#[tokio::test]
async fn chuyen_thanh_phieu_ho_tro() {
    TestDatabase::run(async |test| {
        let intake = Arc::new(FakeIntake::default());
        let shared = intake.clone();
        let h =
            harness_with(test, false, move |builder| builder.provide::<dyn SupportIntake>(shared))
                .await;
        let id = start(&h.app, "buyer", "Áo bị rách").await;
        reply(&h.app, "agent", &id, "Bạn gửi ảnh giúp shop").await;
        let (status, escalated) =
            call(&h.app, "POST", &format!("/threads/{id}/escalate"), "agent", None).await;
        assert_eq!(status, StatusCode::OK, "{escalated}");
        let ticket = escalated["data"]["ticket_id"].as_str().unwrap().to_owned();
        assert_eq!(escalated["data"]["thread"]["ticket_id"], ticket.as_str());
        // Gọi lại: cùng phiếu.
        let (_, again) =
            call(&h.app, "POST", &format!("/threads/{id}/escalate"), "agent", None).await;
        assert_eq!(again["data"]["ticket_id"], ticket.as_str());
        let opened = intake.0.lock().unwrap().clone();
        assert_eq!(opened.len(), 1);
        assert_eq!(opened[0].requester, Requester::User(Uuid::from_u128(BUYER)));
        assert!(opened[0].body.contains("Áo bị rách") && opened[0].body.contains("Bạn gửi ảnh"));
        assert!(opened[0].links.contains(&format!("conversations.thread:{id}")));
        // Nhân viên nhánh khác không chuyển được.
        let (status, _) =
            call(&h.app, "POST", &format!("/threads/{id}/escalate"), "branch", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tool_mcp_dung_chung_service() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        let id = start(&h.app, "buyer", "Có giao tỉnh không?").await;
        let client = h.app.mcp("agent").await.unwrap();
        let listed = client
            .call_tool(autoxide_testkit::tool(
                "conversations.list_threads",
                json!({"unread": true}),
            ))
            .await
            .unwrap();
        assert!(format!("{listed:?}").contains(&id), "{listed:?}");
        let args = json!({"id": id, "body": "Có bạn nhé", "client_id": "t-1"});
        client
            .call_tool(autoxide_testkit::tool("conversations.reply", args.clone()))
            .await
            .unwrap();
        client.call_tool(autoxide_testkit::tool("conversations.reply", args)).await.unwrap();
        // Gọi lại cùng `client_id`: một tin.
        assert_eq!(messages(&h.app, &id).await.len(), 2);
        // Nhân viên nhánh khác không gọi được vào cuộc này.
        let branch = h.app.mcp("branch").await.unwrap();
        let got = branch
            .call_tool(autoxide_testkit::tool("conversations.get_thread", json!({"id": id})))
            .await;
        let refused = match got {
            Ok(result) => result.is_error == Some(true),
            Err(_) => true,
        };
        assert!(refused, "MCP lets another branch read the conversation");
        client.cancel().await.unwrap();
        branch.cancel().await.unwrap();
    })
    .await
    .unwrap();
}

/// Hai thiết bị đánh dấu đã đọc cùng lúc (một tới tin cũ, một tới tin mới nhất): con trỏ không lùi.
/// Trước đây đọc con trỏ rồi ghi tuyệt đối, bên ghi sau thắng.
#[tokio::test]
async fn danh_dau_da_doc_cung_luc_con_tro_khong_lui() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        let id = start(&h.app, "buyer", "Chào shop").await;
        for round in 0..4 {
            let uri = format!("/me/threads/{id}/messages");
            let body = json!({"body": format!("Tin {round}"), "client_id": format!("r-{round}")});
            let (status, _) = call(&h.app, "POST", &uri, "buyer", Some(body)).await;
            assert_eq!(status, StatusCode::CREATED);
            let all = messages(&h.app, &id).await;
            let id_of =
                |row: &serde_json::Value| row["id"].as_str().unwrap().parse::<Uuid>().unwrap();
            let oldest = id_of(all.first().unwrap());
            let newest = id_of(all.last().unwrap());
            let read = format!("/threads/{id}/read");
            let (old, new) = tokio::join!(
                call(&h.app, "POST", &read, "agent", Some(json!({"message_id": oldest}))),
                call(&h.app, "POST", &read, "agent", Some(json!({"message_id": newest}))),
            );
            assert_eq!((old.0, new.0), (StatusCode::OK, StatusCode::OK));
            let cursor = crate::entity::read::Entity::find()
                .filter(crate::entity::read::Column::ThreadId.eq(id.parse::<Uuid>().unwrap()))
                .filter(crate::entity::read::Column::UserId.eq(Uuid::from_u128(AGENT)))
                .one(h.app.database().read())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(cursor.last_read_message_id, newest, "round {round}");
        }
    })
    .await
    .unwrap();
}
