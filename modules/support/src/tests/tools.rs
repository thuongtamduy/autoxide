//! Tool MCP cùng permission như REST, gợi ý AI chỉ là bản nháp, email trả lời vào lại phiếu.

use std::sync::Arc;

use autoxide_kernel::access::Scope;
use autoxide_kernel::ai::TextModel;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_testkit::ai::FakeTextModel;
use autoxide_testkit::{TestApp, TestDatabase, tool};
use axum::http::StatusCode;
use serde_json::json;

use super::{call, harness_with};
use crate::api::{InboundEmail, InboundMail, InboundOutcome};

async fn open(app: &TestApp) -> String {
    let body = json!({"category": "order", "subject": "Đơn chưa tới", "body": "Gọi tôi 0901234567 nhé", "name": "An"});
    let (status, opened) = call(app, "POST", "/public/tickets", "buyer", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{opened}");
    opened["data"]["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn tool_mcp_cung_permission_nhu_rest() {
    TestDatabase::run(async |test| {
        let root = root_of(DEFAULT_TENANT);
        let harness = harness_with(test, move |builder| {
            builder.member("reader", 0xe1, root, &[("support.tickets.read", Scope::Tenant)])
        })
        .await;
        let app = &harness.app;
        let id = open(app).await;

        let agent = app.mcp("agent").await.unwrap();
        let listed = agent.call_tool(tool("support.list_tickets", json!({}))).await.unwrap();
        assert_eq!(listed.structured_content.unwrap()["data"][0]["id"], id.as_str());
        let replied = agent
            .call_tool(tool("support.reply_ticket", json!({"id": id, "body": "Đã nhận"})))
            .await
            .unwrap();
        assert_eq!(replied.is_error, Some(false), "{replied:?}");
        let got = agent.call_tool(tool("support.get_ticket", json!({"id": id}))).await.unwrap();
        assert_eq!(got.structured_content.unwrap()["data"]["status"], "pending_customer");

        // Người chỉ đọc: REST và MCP cùng từ chối việc ghi.
        let reader = app.mcp("reader").await.unwrap();
        let denied =
            reader.call_tool(tool("support.reply_ticket", json!({"id": id, "body": "x"}))).await;
        let denied_mcp = match denied {
            Ok(result) => result.is_error == Some(true),
            Err(_) => true,
        };
        assert!(denied_mcp, "MCP lets a reader write");
        let (status, _) = call(
            app,
            "POST",
            &format!("/tickets/{id}/messages"),
            "reader",
            Some(json!({"body": "x"})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let read = reader.call_tool(tool("support.get_ticket", json!({"id": id}))).await.unwrap();
        assert_eq!(read.is_error, Some(false));
        agent.cancel().await.unwrap();
        reader.cancel().await.unwrap();
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
            Ok(json!({"reply": "Chào An, đơn của bạn sẽ tới trong hôm nay."}))
        }));
        let provided = model.clone();
        let harness =
            harness_with(test, move |builder| builder.provide::<dyn TextModel>(provided)).await;
        let app = &harness.app;
        let id = open(app).await;

        let (status, draft) =
            call(app, "POST", &format!("/tickets/{id}/reply-suggestions"), "agent", None).await;
        assert_eq!(status, StatusCode::OK, "{draft}");
        assert_eq!(draft["data"]["ai_assisted"], true);
        assert_eq!(model.calls(), 1);
        // Chưa gửi gì: phiếu vẫn chỉ có tin của người mua, trạng thái không đổi.
        let (_, got) = call(app, "GET", &format!("/tickets/{id}"), "agent", None).await;
        assert_eq!(got["data"]["messages"].as_array().unwrap().len(), 1);
        assert_eq!(got["data"]["status"], "open");
        let (_, drafts) =
            call(app, "GET", &format!("/tickets/{id}/reply-drafts"), "agent", None).await;
        assert_eq!(drafts["data"].as_array().unwrap().len(), 1);
        // Nhân viên gửi bản đã sửa, gắn nhãn AI.
        let body = json!({"body": "Chào An, đơn sẽ tới hôm nay.", "ai_assisted": true});
        let (_, sent) =
            call(app, "POST", &format!("/tickets/{id}/messages"), "agent", Some(body)).await;
        assert_eq!(sent["data"]["ai_assisted"], true);
        // Người mua không gọi được.
        let (status, _) =
            call(app, "POST", &format!("/tickets/{id}/reply-suggestions"), "buyer", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn email_tra_loi_vao_lai_phieu_khi_bat() {
    TestDatabase::run(async |test| {
        let harness = harness_with(test, |builder| builder).await;
        let app = &harness.app;
        let body = json!({"requester": {"email": "khach@example.com", "name": "Khách"}, "category": "order",
                          "subject": "Gọi điện", "body": "Hỏi đơn"});
        let (_, opened) = call(app, "POST", "/tickets", "agent", Some(body)).await;
        let id: uuid::Uuid = opened["data"]["id"].as_str().unwrap().parse().unwrap();
        let inbound = app.api::<dyn InboundMail>().unwrap();
        let tenant = app.default_tenant();
        let key = inbound.reply_address_key(&tenant, id).await.unwrap();
        let mail = |recipient: String, sender: &str| InboundEmail {
            recipient,
            sender: sender.to_owned(),
            text: "Tôi đã nhận hàng.\n\nOn Mon, Shop wrote:\n> Đơn đã giao".to_owned(),
        };
        let address = format!("support+{key}@shop.example");
        // Tắt mặc định.
        let outcome = inbound.receive(&tenant, mail(address.clone(), "khach@example.com")).await.unwrap();
        assert_eq!(outcome, InboundOutcome::Dropped);
        let root = tenant.id().as_uuid();
        let (_, policy) = call(app, "GET", &format!("/policies/{root}"), "agent", None).await;
        let mut policy = policy["data"].clone();
        for field in ["org_id", "defined_at"] {
            policy.as_object_mut().unwrap().remove(field);
        }
        policy["inbound_email"] = json!(true);
        let (status, body) = call(app, "PUT", &format!("/policies/{root}"), "agent", Some(policy)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        // Người gửi lạ, khoá sai: bỏ.
        let stranger = inbound.receive(&tenant, mail(address.clone(), "la@example.com")).await.unwrap();
        assert_eq!(stranger, InboundOutcome::Dropped);
        let forged = format!("support+{id}.000000000000000000000000@shop.example");
        assert_eq!(inbound.receive(&tenant, mail(forged, "khach@example.com")).await.unwrap(), InboundOutcome::Dropped);
        // Đúng người, đúng khoá: thành tin của phiếu, phần trích dẫn bị cắt.
        let added = inbound.receive(&tenant, mail(address, "Khach@Example.com")).await.unwrap();
        assert_eq!(added, InboundOutcome::Added { ticket_id: id });
        let (_, got) = call(app, "GET", &format!("/tickets/{id}"), "agent", None).await;
        let last = got["data"]["messages"].as_array().unwrap().last().unwrap().clone();
        assert_eq!(last["body"], "Tôi đã nhận hàng.");
    })
    .await
    .unwrap();
}
