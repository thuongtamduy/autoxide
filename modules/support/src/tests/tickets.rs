//! Phiếu của người mua có tài khoản và của nhân viên: tạo, trả lời, trạng thái, ghi chú nội bộ, ẩn
//! tin, đính kèm, `SupportIntake`, ẩn danh hoá khi xoá tài khoản.

use autoxide_kernel::event::Delivered;
use autoxide_kernel::event::EventHandler;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{AGENT, BUYER, call, deliver, harness};
use crate::api::{NewTicket, Requester, SupportIntake};

fn question() -> Value {
    json!({"category": "order", "subject": "Đơn chưa tới", "body": "Đơn của tôi chưa giao.", "name": "An"})
}

async fn open_as_buyer(app: &autoxide_testkit::TestApp) -> String {
    let (status, opened) = call(app, "POST", "/public/tickets", "buyer", Some(question())).await;
    assert_eq!(status, StatusCode::CREATED, "{opened}");
    assert!(opened["data"]["access_token"].is_null(), "accounts get no ticket token");
    opened["data"]["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn nguoi_mua_khong_thay_ghi_chu_noi_bo() {
    TestDatabase::run(async |test| {
        let harness = harness(test).await;
        let app = &harness.app;
        let id = open_as_buyer(app).await;

        let note = json!({"body": "Khách này hay hỏi, kiểm kho trước.", "internal": true,
                          "mentions": [Uuid::from_u128(0xa2)]});
        let (status, _) =
            call(app, "POST", &format!("/tickets/{id}/messages"), "agent", Some(note)).await;
        assert_eq!(status, StatusCode::CREATED);
        let answer = json!({"body": "Đơn sẽ tới ngày mai <b>nhé</b>."});
        let (status, _) =
            call(app, "POST", &format!("/tickets/{id}/messages"), "agent", Some(answer)).await;
        assert_eq!(status, StatusCode::CREATED);

        let (status, mine) = call(app, "GET", &format!("/me/tickets/{id}"), "buyer", None).await;
        assert_eq!(status, StatusCode::OK, "{mine}");
        let kinds: Vec<&str> = mine["data"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["kind"].as_str().unwrap())
            .collect();
        assert!(!kinds.contains(&"internal"), "{kinds:?}");
        assert_eq!(mine["data"]["status"], "pending_customer");
        let last = mine["data"]["messages"].as_array().unwrap().last().unwrap().clone();
        assert_eq!(last["body_html"], "Đơn sẽ tới ngày mai &lt;b&gt;nhé&lt;/b&gt;.");

        let (_, staff) = call(app, "GET", &format!("/tickets/{id}"), "agent", None).await;
        assert!(
            staff["data"]["messages"].as_array().unwrap().iter().any(|m| m["kind"] == "internal")
        );

        // Thông báo: người được nhắc tên, người mua nhận câu trả lời.
        deliver(app).await;
        let kinds: Vec<String> = harness.notifier.calls().iter().map(|n| n.kind.clone()).collect();
        assert!(kinds.contains(&"support.mentioned".to_owned()), "{kinds:?}");
        assert!(kinds.contains(&"support.ticket_replied".to_owned()), "{kinds:?}");

        // Người mua khác không thấy phiếu.
        let (status, _) = call(app, "GET", &format!("/me/tickets/{id}"), "other", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (_, list) = call(app, "GET", "/me/tickets", "other", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 0);
        let (_, list) = call(app, "GET", "/me/tickets", "buyer", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn may_trang_thai_qua_rest() {
    TestDatabase::run(async |test| {
        let harness = harness(test).await;
        let app = &harness.app;
        let id = open_as_buyer(app).await;
        let path = format!("/tickets/{id}");

        let (status, bad) =
            call(app, "PATCH", &path, "agent", Some(json!({"status": "pending_agent"}))).await;
        assert_eq!(status, StatusCode::OK, "{bad}");
        let (status, solved) = call(
            app,
            "PATCH",
            &path,
            "agent",
            Some(json!({"status": "solved", "priority": "high", "assignee_user_id": Uuid::from_u128(AGENT)})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{solved}");
        assert_eq!(solved["data"]["resolution"], "answered");
        assert_eq!(solved["data"]["priority"], "high");

        // `solved` → `pending_customer` không phải một bước của quy trình.
        let (status, problem) =
            call(app, "PATCH", &path, "agent", Some(json!({"status": "pending_customer"}))).await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::CONFLICT, Some("support.invalid_transition")));

        // Người mua viết vào phiếu `solved`: mở lại, chờ nhân viên.
        let (status, _) = call(
            app,
            "POST",
            &format!("/me/tickets/{id}/messages"),
            "buyer",
            Some(json!({"body": "Vẫn chưa nhận được."})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let (_, got) = call(app, "GET", &path, "agent", None).await;
        assert_eq!(got["data"]["status"], "pending_agent");

        // Giao cho người không làm ở nút: từ chối.
        let (status, problem) = call(
            app,
            "PATCH",
            &path,
            "agent",
            Some(json!({"assignee_user_id": Uuid::from_u128(0xdead)})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem["code"], "support.assignee_out_of_scope");

        let (status, _) = call(app, "PATCH", &path, "agent", Some(json!({"status": "closed"}))).await;
        assert_eq!(status, StatusCode::OK);
        let (status, problem) = call(
            app,
            "POST",
            &format!("/me/tickets/{id}/messages"),
            "buyer",
            Some(json!({"body": "Còn nữa."})),
        )
        .await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::CONFLICT, Some("support.ticket_closed")));

        // Phiếu mới nối tiếp phiếu đã đóng.
        let mut follow = question();
        follow["follow_up_of"] = json!(id);
        let (status, _) = call(app, "POST", "/public/tickets", "buyer", Some(follow.clone())).await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, _) = call(app, "POST", "/public/tickets", "other", Some(follow)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "follow up of someone else's ticket");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tin_nhan_khong_sua_duoc_chi_an_co_ly_do() {
    TestDatabase::run(async |test| {
        let harness = harness(test).await;
        let app = &harness.app;
        let id = open_as_buyer(app).await;
        let (_, sent) = call(
            app,
            "POST",
            &format!("/tickets/{id}/messages"),
            "agent",
            Some(json!({"body": "Gửi nhầm số điện thoại 0901234567"})),
        )
        .await;
        let message = sent["data"]["id"].as_str().unwrap().to_owned();

        // Không có endpoint sửa hay xoá tin.
        for method in ["PATCH", "PUT", "DELETE"] {
            let (status, _) =
                call(app, method, &format!("/tickets/{id}/messages/{message}"), "agent", None)
                    .await;
            assert!(
                matches!(status, StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED),
                "{method}: {status}"
            );
        }
        let hide = format!("/tickets/{id}/messages/{message}/hide");
        let (status, problem) =
            call(app, "POST", &hide, "agent", Some(json!({"reason": " "}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        let (status, hidden) =
            call(app, "POST", &hide, "agent", Some(json!({"reason": "Sai số điện thoại"}))).await;
        assert_eq!(status, StatusCode::OK, "{hidden}");
        assert_eq!(hidden["data"]["hidden_reason"], "Sai số điện thoại");

        let (_, mine) = call(app, "GET", &format!("/me/tickets/{id}"), "buyer", None).await;
        let shown = mine["data"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == message.as_str())
            .unwrap()
            .clone();
        assert_eq!((shown["hidden"].as_bool(), shown["body"].as_str()), (Some(true), Some("")));
        assert!(shown["hidden_reason"].is_null());
        let (_, staff) = call(app, "GET", &format!("/tickets/{id}"), "agent", None).await;
        assert!(staff.to_string().contains("Gửi nhầm số điện thoại"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dinh_kem_chi_tep_cua_chinh_minh() {
    TestDatabase::run(async |test| {
        let harness = harness(test).await;
        let app = &harness.app;
        let mine = harness.media.upload(Uuid::from_u128(BUYER));
        let theirs = harness.media.upload(Uuid::from_u128(0xb2));
        let id = open_as_buyer(app).await;
        let path = format!("/me/tickets/{id}/messages");
        let (status, problem) =
            call(app, "POST", &path, "buyer", Some(json!({"body": "Ảnh", "media_ids": [theirs]})))
                .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem["code"], "support.attachment_not_owned");
        let (status, sent) =
            call(app, "POST", &path, "buyer", Some(json!({"body": "Ảnh", "media_ids": [mine]})))
                .await;
        assert_eq!(status, StatusCode::CREATED, "{sent}");
        assert_eq!(sent["data"]["media_ids"][0], mine.to_string());
        let too_many: Vec<Uuid> = (0..6).map(|_| mine).collect();
        let (status, _) =
            call(app, "POST", &path, "buyer", Some(json!({"body": "Ảnh", "media_ids": too_many})))
                .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nhan_vien_ghi_ho_va_module_khac_mo_phieu() {
    TestDatabase::run(async |test| {
        let harness = harness(test).await;
        let app = &harness.app;
        let body = json!({"requester": {"phone": "0901234567", "name": "Bình"}, "category": "payment",
                          "subject": "Gọi điện hỏi thanh toán", "body": "Khách hỏi đã nhận tiền chưa."});
        let (status, opened) = call(app, "POST", "/tickets", "agent", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{opened}");
        assert_eq!(opened["data"]["channel"], "staff");
        assert_eq!(opened["data"]["requester"]["phone"], "+84901234567");
        assert_eq!(opened["data"]["requester"]["verified"], true);
        let (status, problem) =
            call(app, "POST", "/tickets", "buyer", Some(json!({"requester": {"email": "a@b.co"},
                "category": "order", "subject": "x", "body": "y"}))).await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::FORBIDDEN, Some("permission_denied")));

        // `SupportIntake`: gọi lại cùng `reference` không tạo phiếu thứ hai.
        let intake = app.api::<dyn SupportIntake>().unwrap();
        let tenant = app.default_tenant();
        let ticket = || NewTicket {
            org_id: root_of(DEFAULT_TENANT).as_uuid(),
            requester: Requester::VerifiedPhone("+84907654321".to_owned()),
            requester_name: "Chi".to_owned(),
            category: "order".to_owned(),
            subject: "Hỏi về đơn".to_owned(),
            body: "Đơn giao chưa?".to_owned(),
            links: vec!["orders.order:01".to_owned()],
            reference: "orders.order:01:key-1".to_owned(),
        };
        let first = intake.open(&tenant, ticket()).await.unwrap();
        assert!(first.access_token.is_some());
        let again = intake.open(&tenant, ticket()).await.unwrap();
        assert_eq!((again.id, again.access_token), (first.id, None));
        let (_, linked) = call(app, "GET", "/tickets?link=orders.order:01", "agent", None).await;
        assert_eq!(linked["data"].as_array().unwrap().len(), 1, "{linked}");
        assert_eq!(linked["data"][0]["channel"], "order_lookup");
        let (_, found) = call(app, "GET", "/tickets?q=hoi%20ve", "agent", None).await;
        assert_eq!(found["data"].as_array().unwrap().len(), 1, "{found}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn xoa_tai_khoan_an_danh_hoa_giu_thong_ke() {
    TestDatabase::run(async |test| {
        let harness = harness(test).await;
        let app = &harness.app;
        let id = open_as_buyer(app).await;
        call(
            app,
            "POST",
            &format!("/tickets/{id}/messages"),
            "agent",
            Some(json!({"body": "Chào An"})),
        )
        .await;

        let event = Delivered::new(
            app.default_tenant(),
            "auth",
            autoxide_kernel::privacy::USER_DELETED,
            json!({"before": {"id": Uuid::from_u128(BUYER)}}),
        );
        let handler = super::service_of(app);
        handler.handle(&event).await.unwrap();
        handler.handle(&event).await.unwrap();

        let (_, got) = call(app, "GET", &format!("/tickets/{id}"), "agent", None).await;
        let data = &got["data"];
        assert!(data["requester"]["user_id"].is_null(), "{data}");
        assert_eq!(data["requester"]["name"], "");
        assert_eq!(data["number"], "000001", "the ticket stays for statistics");
        let messages = data["messages"].as_array().unwrap();
        assert_eq!(messages[0]["body"], "", "the customer's text is gone");
        assert_eq!(messages[1]["body"], "Chào An", "staff answers stay");
        let (_, list) = call(app, "GET", "/me/tickets", "buyer", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dong_phieu_va_gui_tin_cung_luc_phieu_van_dong() {
    TestDatabase::run(async |test| {
        let harness = harness(test).await;
        let app = &harness.app;
        for _ in 0..5 {
            let id = open_as_buyer(app).await;
            let path = format!("/tickets/{id}");
            let answer = json!({"body": "Chúng tôi đang kiểm tra."});
            let (status, _) =
                call(app, "POST", &format!("{path}/messages"), "agent", Some(answer)).await;
            assert_eq!(status, StatusCode::CREATED);
            // Nhân viên đóng phiếu đúng lúc người mua gửi tin. Thứ tự nào cũng được, nhưng phiếu
            // phải còn đóng: Postgres từng cho tin của người mua ghi đè `closed` thành
            // `pending_agent` (đọc trạng thái trước khi lệnh đóng commit).
            let mine = format!("/me/tickets/{id}/messages");
            let (closed, sent) = tokio::join!(
                call(app, "PATCH", &path, "agent", Some(json!({"status": "closed"}))),
                call(app, "POST", &mine, "buyer", Some(json!({"body": "Còn nữa."}))),
            );
            assert_eq!(closed.0, StatusCode::OK, "{}", closed.1);
            assert!(matches!(sent.0, StatusCode::CREATED | StatusCode::CONFLICT), "{}", sent.1);
            let (_, got) = call(app, "GET", &path, "agent", None).await;
            assert_eq!(got["data"]["status"], "closed");
        }
    })
    .await
    .unwrap();
}
