//! Khách vãng lai: token phiếu, xác thực OTP trước tin thứ hai, gộp phiếu vào tài khoản.

use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::{Value, json};

use super::{call, form_token, harness};

async fn open_as_guest(app: &TestApp, contact: Value) -> (String, String) {
    let mut body = json!({"category": "product", "subject": "Hỏi size áo", "body": "Áo size M dài bao nhiêu?",
                          "name": "Dung", "form_token": form_token(app).await});
    for (key, value) in contact.as_object().unwrap() {
        body[key] = value.clone();
    }
    let (status, opened) = call(app, "POST", "/public/tickets", "", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{opened}");
    let data = &opened["data"];
    (data["id"].as_str().unwrap().to_owned(), data["access_token"].as_str().unwrap().to_owned())
}

#[tokio::test]
async fn vang_lai_phai_xac_thuc_truoc_tin_thu_hai() {
    TestDatabase::run(async |test| {
        let harness = harness(test).await;
        let app = &harness.app;
        let (id, token) = open_as_guest(app, json!({"email": "Dung@Example.com"})).await;
        let messages = format!("/public/tickets/{id}/messages");

        let (status, problem) =
            call(app, "POST", &messages, "", Some(json!({"token": token, "body": "Thêm nữa"})))
                .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
        assert_eq!(problem["code"], "support.verification_required");

        let verify = format!("/public/tickets/{id}/verify");
        let (status, _) =
            call(app, "POST", &verify, "", Some(json!({"token": token, "code": "000000x"}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let code = harness.notifier.last_code();
        assert_eq!(code.len(), 6);
        let (status, verified) =
            call(app, "POST", &verify, "", Some(json!({"token": token, "code": code}))).await;
        assert_eq!(status, StatusCode::OK, "{verified}");
        assert_eq!(verified["data"]["requester"]["verified"], true);
        assert_eq!(verified["data"]["requester"]["email"], "d•••@example.com");

        let (status, _) =
            call(app, "POST", &messages, "", Some(json!({"token": token, "body": "Thêm nữa"})))
                .await;
        assert_eq!(status, StatusCode::CREATED);
        // Khách vãng lai không gửi tệp.
        let (status, _) = call(
            app,
            "POST",
            "/public/tickets",
            "",
            Some(json!({"category": "order", "subject": "x", "body": "y", "email": "a@b.co",
                        "media_ids": [uuid::Uuid::now_v7()], "form_token": form_token(app).await})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn token_phieu_chi_mo_dung_mot_phieu() {
    TestDatabase::run(async |test| {
        let harness = harness(test).await;
        let app = &harness.app;
        let (first, token) = open_as_guest(app, json!({"phone": "0912345678"})).await;
        let (second, _) = open_as_guest(app, json!({"email": "x@example.com"})).await;

        let (status, got) = call(app, "GET", &format!("/public/tickets/{first}?token={token}"), "", None).await;
        assert_eq!(status, StatusCode::OK, "{got}");
        assert_eq!(got["data"]["requester"]["phone"], "+84•••678");
        let (status, problem) =
            call(app, "GET", &format!("/public/tickets/{second}?token={token}"), "", None).await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::NOT_FOUND, Some("support.ticket_not_found")));
        let (status, _) = call(app, "GET", &format!("/public/tickets/{first}?token=wrong"), "", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Thiếu liên lạc, hay đưa cả hai: lỗi đầu vào.
        let (status, _) = call(
            app,
            "POST",
            "/public/tickets",
            "",
            Some(json!({"category": "order", "subject": "x", "body": "y", "form_token": form_token(app).await})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        // Không có token form: chống spam chặn.
        let (status, _) = call(
            app,
            "POST",
            "/public/tickets",
            "",
            Some(json!({"category": "order", "subject": "x", "body": "y", "email": "a@b.co"})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gop_phieu_vang_lai_vao_tai_khoan() {
    TestDatabase::run(async |test| {
        let harness = harness(test).await;
        let app = &harness.app;
        let (first, token) = open_as_guest(app, json!({"email": "gop@example.com"})).await;
        let (second, _) = open_as_guest(app, json!({"email": "gop@example.com"})).await;

        // Chưa xác thực thì chưa gộp được.
        let claim = json!({"ticket_id": first, "token": token});
        let (status, problem) = call(app, "POST", "/me/claim", "buyer", Some(claim.clone())).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("support.verification_required"))
        );

        // Xác thực phiếu đầu; phiếu thứ hai (chưa xác thực) không được gộp.
        let code = {
            let sent = harness.notifier.sent();
            let mut codes = sent.iter().filter(|n| n.kind == "support.verification_code");
            codes.next().unwrap().params["code"].as_str().unwrap().to_owned()
        };
        let verify = format!("/public/tickets/{first}/verify");
        let (status, _) =
            call(app, "POST", &verify, "", Some(json!({"token": token, "code": code}))).await;
        assert_eq!(status, StatusCode::OK);

        let (status, claimed) = call(app, "POST", "/me/claim", "buyer", Some(claim.clone())).await;
        assert_eq!(status, StatusCode::OK, "{claimed}");
        assert_eq!(claimed["data"]["claimed"], 1);
        let (_, again) = call(app, "POST", "/me/claim", "buyer", Some(claim)).await;
        assert_eq!(again["data"]["claimed"], 0, "calling again changes nothing");

        let (_, mine) = call(app, "GET", "/me/tickets", "buyer", None).await;
        let ids: Vec<&str> =
            mine["data"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
        assert_eq!(ids, [first.as_str()]);
        assert!(!ids.contains(&second.as_str()));
    })
    .await
    .unwrap();
}
