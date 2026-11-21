//! Mua không cần tài khoản (D-201, D-202, D-204): tra đơn bằng số đơn + mã một lần, token xem đơn,
//! huỷ / đổi địa chỉ / mở hỗ trợ bằng token, gắn đơn vào tài khoản, trả lời tự động về trạng thái đơn.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::{Arc, Mutex};

use autoxide_kernel::error::Error;
use autoxide_kernel::sms::SmsSender;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_support::api::{
    FactIntent, FactSource, NewTicket, Requester, SupportFacts, SupportIntake, TicketRef,
};
use autoxide_testkit::{FakeSmsSender, TestDatabase};
use axum::http::StatusCode;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};

use super::checkout::{body, checkout, ready_cart};
use super::{Call, Shop, shop_with};
use crate::entity::{lookup_code, lookup_token};

const PHONE: &str = "+84901234567";

/// `support` giả: giữ phiếu đã mở, cùng `reference` trả phiếu cũ; giữ nguồn trả lời tự động.
#[derive(Default)]
struct FakeSupport {
    tickets: Mutex<Vec<NewTicket>>,
    facts: Mutex<Vec<Arc<dyn FactSource>>>,
}

#[async_trait::async_trait]
impl SupportIntake for FakeSupport {
    async fn open(&self, _tenant: &TenantContext, ticket: NewTicket) -> Result<TicketRef, Error> {
        let mut tickets = self.tickets.lock().unwrap();
        let index = match tickets.iter().position(|old| old.reference == ticket.reference) {
            Some(index) => index,
            None => {
                tickets.push(ticket);
                tickets.len() - 1
            }
        };
        Ok(TicketRef {
            id: uuid::Uuid::from_u128(index as u128 + 1),
            number: format!("T-{index}"),
            access_token: None,
        })
    }
}

impl SupportFacts for FakeSupport {
    fn attach(&self, source: Arc<dyn FactSource>) -> Result<(), Error> {
        self.facts.lock().unwrap().push(source);
        Ok(())
    }
}

struct Guests {
    shop: Shop,
    sms: Arc<FakeSmsSender>,
    support: Arc<FakeSupport>,
}

async fn guests(test: &TestDatabase, with_support: bool) -> Guests {
    let sms = Arc::new(FakeSmsSender::new());
    let support = Arc::new(FakeSupport::default());
    let (sms_in, support_in) = (sms.clone(), support.clone());
    let shop = shop_with(test, move |builder, _| {
        let builder = builder.provide::<dyn SmsSender>(sms_in);
        if with_support {
            builder
                .provide::<dyn SupportIntake>(support_in.clone())
                .provide::<dyn SupportFacts>(support_in)
        } else {
            builder
        }
    })
    .await;
    Guests { shop, sms, support }
}

/// Một đơn vãng lai COD ở cửa hàng A (đã xác nhận); trả (id, số đơn).
async fn guest_order(shop: &Shop) -> (String, String) {
    let shirt = shop.catalog.add(shop.tree.a, 200_000);
    shop.inventory.stock(shop.tree.a, shirt, 5);
    let guest = Call::as_guest(shop, None);
    let (cart, token, quote) = ready_cart(&guest, shop.tree.a, shirt, 1).await;
    let guest = Call::as_guest(shop, token.as_deref());
    let (status, placed) =
        checkout(&guest, Some(&uuid::Uuid::now_v7().to_string()), body(&cart, &quote, "cod")).await;
    assert_eq!(status, StatusCode::CREATED, "{placed}");
    let order = &placed["data"]["orders"][0];
    (order["id"].as_str().unwrap().to_owned(), order["number"].as_str().unwrap().to_owned())
}

/// Mã 6 số trong tin SMS cuối gửi tới `PHONE`.
fn last_code(sms: &FakeSmsSender) -> String {
    let text = sms.last_text(PHONE).unwrap();
    text.split(|c: char| !c.is_ascii_digit() && c != '-')
        .find(|word| word.len() == 6 && word.bytes().all(|b| b.is_ascii_digit()))
        .unwrap_or_else(|| panic!("no code in {text}"))
        .to_owned()
}

/// Gọi `/api/v1/orders{uri}` không đăng nhập, kèm `Order-Token` và `Idempotency-Key` nếu có.
async fn public(
    shop: &Shop,
    method: &str,
    uri: &str,
    token: Option<&str>,
    key: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = axum::http::Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = shop.request(method, &format!("/api/v1/orders{uri}"));
    if let Some(token) = token {
        request = request.header("Order-Token", token);
    }
    if let Some(key) = key {
        request = request.header("Idempotency-Key", key);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let status = answer.status();
    (status, if answer.body().is_empty() { Value::Null } else { answer.json() })
}

/// Tra đơn rồi xác nhận mã: trả token xem đơn.
async fn access_token(guests: &Guests, number: &str) -> String {
    let (status, sent) = public(
        &guests.shop,
        "POST",
        "/public/orders/lookup",
        None,
        None,
        Some(json!({"number": number, "phone": "0901 234 567"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sent}");
    assert_eq!(sent["data"]["phone"], "090•••567");
    let lookup = sent["data"]["lookup_id"].as_str().unwrap();
    let code = last_code(&guests.sms);
    let (status, access) = public(
        &guests.shop,
        "POST",
        &format!("/public/orders/lookup/{lookup}/confirm"),
        None,
        None,
        Some(json!({"code": code})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{access}");
    access["data"]["access_token"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn tra_don_sai_so_va_khong_co_don_tra_loi_giong_nhau() {
    TestDatabase::run(async |test| {
        let guests = guests(test, false).await;
        let (_, number) = guest_order(&guests.shop).await;
        let lookup = |number: &str, phone: &str| {
            let body = json!({"number": number, "phone": phone});
            public(&guests.shop, "POST", "/public/orders/lookup", None, None, Some(body))
        };
        let (wrong_phone, a) = lookup(&number, "0909999999").await;
        let (no_order, b) = lookup("2026-999999", "0901234567").await;
        assert_eq!(wrong_phone, StatusCode::NOT_FOUND);
        assert_eq!(no_order, StatusCode::NOT_FOUND);
        assert_eq!(a["code"], "orders.lookup_failed");
        assert_eq!(a["code"], b["code"]);
        assert_eq!(a["detail"], b["detail"]);
        assert!(guests.sms.sent.lock().unwrap().is_empty(), "không gửi SMS khi không khớp");

        // Mã sai không mở token; sai quá 5 lần thì mã hỏng hẳn.
        let (status, sent) = lookup(&number, "0901234567").await;
        assert_eq!(status, StatusCode::OK, "{sent}");
        let id = sent["data"]["lookup_id"].as_str().unwrap();
        let confirm = format!("/public/orders/lookup/{id}/confirm");
        let right = last_code(&guests.sms);
        let wrong = if right == "000000" { "111111" } else { "000000" };
        for _ in 0..5 {
            let (status, answer) =
                public(&guests.shop, "POST", &confirm, None, None, Some(json!({"code": wrong})))
                    .await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
            assert_eq!(answer["code"], "orders.lookup_code_invalid");
        }
        let (status, _) =
            public(&guests.shop, "POST", &confirm, None, None, Some(json!({"code": right}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "mã hỏng sau 5 lần sai");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gui_lai_otp_phai_cho_va_tra_qua_nhieu_bi_chan() {
    TestDatabase::run(async |test| {
        let guests = guests(test, false).await;
        let (order, number) = guest_order(&guests.shop).await;
        let body = json!({"number": number, "phone": "0901234567"});
        let (status, _) =
            public(&guests.shop, "POST", "/public/orders/lookup", None, None, Some(body.clone()))
                .await;
        assert_eq!(status, StatusCode::OK);
        let (status, again) =
            public(&guests.shop, "POST", "/public/orders/lookup", None, None, Some(body.clone()))
                .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{again}");
        assert_eq!(again["code"], "orders.lookup_resend_too_soon");
        assert!(again["detail"].as_str().unwrap().contains("seconds"), "{again}");

        // Qua thời gian chờ: gửi lại được.
        lookup_code::Entity::update_many()
            .col_expr(
                lookup_code::Column::CreatedAt,
                Expr::value(Timestamp::now() - SignedDuration::from_mins(2)),
            )
            .filter(lookup_code::Column::OrderId.eq(uuid::Uuid::parse_str(&order).unwrap()))
            .exec(guests.shop.database().write())
            .await
            .unwrap();
        let (status, _) =
            public(&guests.shop, "POST", "/public/orders/lookup", None, None, Some(body.clone()))
                .await;
        assert_eq!(status, StatusCode::OK);

        // Quá `AUTOXIDE_ORDERS_LOOKUP_PER_HOUR` (5) lần tra cùng cặp số đơn + số điện thoại.
        for _ in 0..3 {
            public(&guests.shop, "POST", "/public/orders/lookup", None, None, Some(body.clone()))
                .await;
        }
        let (status, limited) =
            public(&guests.shop, "POST", "/public/orders/lookup", None, None, Some(body)).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{limited}");
        assert_eq!(limited["code"], "orders.lookup_too_many");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn token_xem_don_che_du_lieu_va_het_han() {
    TestDatabase::run(async |test| {
        let guests = guests(test, false).await;
        let (order, number) = guest_order(&guests.shop).await;
        let (other, _) = guest_order(&guests.shop).await;
        let token = access_token(&guests, &number).await;
        let uri = format!("/public/orders/{order}");
        let (status, seen) = public(&guests.shop, "GET", &uri, Some(&token), None, None).await;
        assert_eq!(status, StatusCode::OK, "{seen}");
        let data = &seen["data"];
        assert_eq!(data["number"], number.as_str());
        assert_eq!(data["contact"]["phone"], "090•••567");
        assert_eq!(data["contact"]["name"], "N. An");
        assert_eq!(data["ship_to"]["phone"], "090•••567");
        assert!(data["ship_to"]["address"].get("line").is_none(), "{data}");
        assert_eq!(data["ship_to"]["address"]["commune_code"], "00004");
        assert!(!seen.to_string().contains("0901234567"), "{seen}");

        // Token chỉ mở đúng một đơn; không token thì 401.
        let (status, _) = public(
            &guests.shop,
            "GET",
            &format!("/public/orders/{other}"),
            Some(&token),
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, missing) = public(&guests.shop, "GET", &uri, None, None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(missing["code"], "orders.lookup_token_invalid");

        // Quá hạn (mặc định 30 phút) thì token hết dùng được.
        lookup_token::Entity::update_many()
            .col_expr(
                lookup_token::Column::ExpiresAt,
                Expr::value(Timestamp::now() - SignedDuration::from_mins(1)),
            )
            .exec(guests.shop.database().write())
            .await
            .unwrap();
        let (status, _) = public(&guests.shop, "GET", &uri, Some(&token), None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn huy_don_vang_lai_cung_chinh_sach_va_goi_lai_cung_khoa() {
    TestDatabase::run(async |test| {
        let guests = guests(test, false).await;
        let (order, number) = guest_order(&guests.shop).await;
        let token = access_token(&guests, &number).await;
        let uri = format!("/public/orders/{order}/cancel");
        let cancel = json!({"reason": "đổi ý"});
        let (status, cancelled) =
            public(&guests.shop, "POST", &uri, Some(&token), Some("k1"), Some(cancel.clone()))
                .await;
        assert_eq!(status, StatusCode::OK, "{cancelled}");
        assert_eq!(cancelled["data"]["status"], "cancelled");
        // Gọi lại cùng khoá: cùng kết quả.
        let (status, again) =
            public(&guests.shop, "POST", &uri, Some(&token), Some("k1"), Some(cancel)).await;
        assert_eq!(status, StatusCode::OK, "{again}");
        assert_eq!(again["data"]["status"], "cancelled");
        // Cùng khoá, nội dung khác: 422.
        let (status, reused) = public(
            &guests.shop,
            "POST",
            &uri,
            Some(&token),
            Some("k1"),
            Some(json!({"reason": "khác"})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{reused}");
        assert_eq!(reused["code"], "orders.idempotency_key_reused");

        // Đơn đã giao xong thì không tự huỷ được (cùng chính sách với người có tài khoản).
        let (order, number) = guest_order(&guests.shop).await;
        let id: uuid::Uuid = order.parse().unwrap();
        for to in ["preparing", "shipping"] {
            let etag = super::lifecycle::etag(&guests.shop, id).await;
            let (status, moved) =
                super::lifecycle::transition(&guests.shop, id, to, Some(&etag)).await;
            assert_eq!(status, StatusCode::OK, "{to}: {moved}");
        }
        let token = access_token_again(&guests, &number).await;
        let (status, refused) = public(
            &guests.shop,
            "POST",
            &format!("/public/orders/{order}/cancel"),
            Some(&token),
            None,
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{refused}");
        assert_eq!(refused["code"], "orders.cancel_not_allowed");
    })
    .await
    .unwrap();
}

/// Tra lần nữa (bỏ chờ gửi lại của mã trước).
async fn access_token_again(guests: &Guests, number: &str) -> String {
    lookup_code::Entity::update_many()
        .col_expr(
            lookup_code::Column::CreatedAt,
            Expr::value(Timestamp::now() - SignedDuration::from_mins(5)),
        )
        .exec(guests.shop.database().write())
        .await
        .unwrap();
    access_token(guests, number).await
}

#[tokio::test]
async fn doi_dia_chi_can_otp_lan_hai() {
    TestDatabase::run(async |test| {
        let guests = guests(test, false).await;
        let (order, number) = guest_order(&guests.shop).await;
        let token = access_token(&guests, &number).await;
        let address = json!({"country": "VN", "province_code": "01", "commune_code": "00004",
            "line": "99 Hàng Bài"});
        let uri = format!("/public/orders/{order}/address");
        // Chưa xin mã lần hai: mã nào cũng sai (mã tra đơn không dùng được).
        let first = last_code(&guests.sms);
        let (status, refused) = public(
            &guests.shop,
            "POST",
            &uri,
            Some(&token),
            None,
            Some(json!({"address": address, "code": first})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(refused["code"], "orders.lookup_code_invalid");

        let (status, sent) = public(
            &guests.shop,
            "POST",
            &format!("/public/orders/{order}/address/otp"),
            Some(&token),
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{sent}");
        let code = last_code(&guests.sms);

        // Phí giao đổi theo địa chỉ mới: 409, không đổi gì, mã chưa bị dùng.
        *guests.shop.shipping.fee.lock().unwrap() = 45_000;
        let change = json!({"address": address, "code": code, "recipient_name": "Bình"});
        let (status, conflict) =
            public(&guests.shop, "POST", &uri, Some(&token), None, Some(change.clone())).await;
        assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
        assert_eq!(conflict["code"], "orders.address_change_needs_payment");

        *guests.shop.shipping.fee.lock().unwrap() = 30_000;
        let (status, changed) =
            public(&guests.shop, "POST", &uri, Some(&token), Some("a1"), Some(change.clone()))
                .await;
        assert_eq!(status, StatusCode::OK, "{changed}");
        assert_eq!(changed["data"]["ship_to"]["recipient_name"], "Bình");
        // Gọi lại cùng khoá: không cần mã mới.
        let (status, again) =
            public(&guests.shop, "POST", &uri, Some(&token), Some("a1"), Some(change)).await;
        assert_eq!(status, StatusCode::OK, "{again}");

        let boss = Call::as_user(&guests.shop, "boss");
        let (_, staff_view) = boss.send("GET", &format!("/{order}"), None).await;
        let stored =
            crate::entity::order::Entity::find_by_id(uuid::Uuid::parse_str(&order).unwrap())
                .scoped(&guests.shop.default_tenant())
                .one(guests.shop.database().read())
                .await
                .unwrap()
                .unwrap();
        assert!(stored.ship_to.unwrap().contains("99 Hàng Bài"), "{staff_view}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn mo_ho_tro_va_gan_don_vao_tai_khoan() {
    TestDatabase::run(async |test| {
        let guests = guests(test, true).await;
        let (order, number) = guest_order(&guests.shop).await;
        let (second, _) = guest_order(&guests.shop).await;
        let token = access_token(&guests, &number).await;
        let uri = format!("/public/orders/{order}/support");
        let message = json!({"message": "Khi nào giao?"});
        let (status, opened) =
            public(&guests.shop, "POST", &uri, Some(&token), Some("s1"), Some(message.clone()))
                .await;
        assert_eq!(status, StatusCode::CREATED, "{opened}");
        let (_, again) =
            public(&guests.shop, "POST", &uri, Some(&token), Some("s1"), Some(message)).await;
        assert_eq!(opened["data"]["ticket_id"], again["data"]["ticket_id"]);
        {
            let tickets = guests.support.tickets.lock().unwrap();
            assert_eq!(tickets.len(), 1);
            assert_eq!(tickets[0].requester, Requester::VerifiedPhone(PHONE.to_owned()));
            assert_eq!(tickets[0].links, vec![format!("orders.order:{order}")]);
        }

        // Trả lời tự động: chỉ chủ đơn (số đã xác thực trùng), người khác thì không.
        let source = guests.support.facts.lock().unwrap()[0].clone();
        let intents: Vec<FactIntent> = source.intents();
        assert_eq!(intents[0].intent, "orders.order_status");
        let tenant = guests.shop.default_tenant();
        let params = json!({"order_number": number});
        let owner = Requester::VerifiedPhone(PHONE.to_owned());
        let answer = source.answer(&tenant, &owner, "orders.order_status", &params, "en").await;
        assert_eq!(
            answer.unwrap().unwrap(),
            format!("Order {number} is confirmed and waiting to be prepared.")
        );
        let stranger = Requester::VerifiedPhone("+84909999999".to_owned());
        let answer = source.answer(&tenant, &stranger, "orders.order_status", &params, "en").await;
        assert_eq!(answer.unwrap(), None);

        // `auth` hỏi số đã xác thực của token (tạo tài khoản từ số điện thoại, D-202).
        let lookups = guests.shop.api::<dyn crate::api::OrderLookups>().unwrap();
        assert_eq!(lookups.verified_phone(&tenant, &token).await.unwrap(), Some(PHONE.to_owned()));
        assert_eq!(lookups.verified_phone(&tenant, "nope").await.unwrap(), None);

        // Gắn đơn: cả hai đơn vãng lai cùng số về tài khoản `buyer1`.
        let buyer = Call::as_user(&guests.shop, "buyer1");
        let (status, claimed) =
            buyer.send("POST", "/me/orders/claim", Some(json!({"lookup_token": token}))).await;
        assert_eq!(status, StatusCode::OK, "{claimed}");
        let ids = claimed["data"]["order_ids"].as_array().unwrap();
        assert_eq!(ids.len(), 2, "{claimed}");
        assert!(ids.contains(&json!(order)) && ids.contains(&json!(second)));
        let (status, mine) = buyer.send("GET", &format!("/me/orders/{second}"), None).await;
        assert_eq!(status, StatusCode::OK, "{mine}");
        // Gọi lại: không còn gì để gắn.
        let (_, again) =
            buyer.send("POST", "/me/orders/claim", Some(json!({"lookup_token": token}))).await;
        assert_eq!(again["data"]["order_ids"], json!([]));
        // Người mua khác không gắn lại được đơn đã có chủ.
        let other = Call::as_user(&guests.shop, "buyer2");
        let (_, none) =
            other.send("POST", "/me/orders/claim", Some(json!({"lookup_token": token}))).await;
        assert_eq!(none["data"]["order_ids"], json!([]));

        // Hỗ trợ cho đơn của chính mình (đã đăng nhập).
        let (status, opened) = buyer
            .send("POST", &format!("/me/orders/{order}/support"), Some(json!({"message": "Hỏi"})))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{opened}");
        let (status, _) = other
            .send("POST", &format!("/me/orders/{order}/support"), Some(json!({"message": "Hỏi"})))
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chua_cai_support_thi_404() {
    TestDatabase::run(async |test| {
        let guests = guests(test, false).await;
        let (order, number) = guest_order(&guests.shop).await;
        let token = access_token(&guests, &number).await;
        let (status, answer) = public(
            &guests.shop,
            "POST",
            &format!("/public/orders/{order}/support"),
            Some(&token),
            None,
            Some(json!({"message": "?"})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{answer}");
        assert_eq!(answer["code"], "orders.support_unavailable");
    })
    .await
    .unwrap();
}

#[test]
fn moi_cau_tra_loi_tu_dong_co_ban_dich() {
    let vi: Value = serde_json::from_str(include_str!("../../locales/vi.json")).unwrap();
    for phrase in crate::facts::phrases() {
        assert!(vi.get(format!("text:{phrase}")).is_some(), "thiếu bản dịch: {phrase}");
    }
}

#[tokio::test]
async fn cung_ma_gui_song_song_chi_mo_mot_token() {
    TestDatabase::run(async |test| {
        let guests = guests(test, false).await;
        let (_, number) = guest_order(&guests.shop).await;
        let body = json!({"number": number, "phone": "0901234567"});
        let (status, sent) =
            public(&guests.shop, "POST", "/public/orders/lookup", None, None, Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{sent}");
        let confirm = format!(
            "/public/orders/lookup/{}/confirm",
            sent["data"]["lookup_id"].as_str().unwrap()
        );
        let code = last_code(&guests.sms);
        let attempts = (0..8).map(|_| {
            public(&guests.shop, "POST", &confirm, None, None, Some(json!({"code": code})))
        });
        let answers = futures_util::future::join_all(attempts).await;
        let opened = answers.iter().filter(|(status, _)| *status == StatusCode::OK).count();
        assert_eq!(opened, 1, "{answers:?}");
        assert!(answers.iter().all(|(status, _)| {
            *status == StatusCode::OK || *status == StatusCode::UNPROCESSABLE_ENTITY
        }));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gan_don_bang_token_sai_la_loi_cua_truong() {
    TestDatabase::run(async |test| {
        let guests = guests(test, false).await;
        let buyer = Call::as_user(&guests.shop, "buyer1");
        let (status, answer) =
            buyer.send("POST", "/me/orders/claim", Some(json!({"lookup_token": "sai"}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
        assert_eq!(answer["errors"][0]["field"], "lookup_token");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doan_ma_tra_don_dong_thoi_van_bi_chan_sau_nam_lan() {
    TestDatabase::run(async |test| {
        let guests = guests(test, false).await;
        let (_, number) = guest_order(&guests.shop).await;
        let body = json!({"number": number, "phone": "0901234567"});
        let (status, sent) =
            public(&guests.shop, "POST", "/public/orders/lookup", None, None, Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{sent}");
        let id = sent["data"]["lookup_id"].as_str().unwrap();
        let confirm = format!("/public/orders/lookup/{id}/confirm");
        let right = last_code(&guests.sms);
        let wrong = json!({"code": if right == "000000" { "111111" } else { "000000" }});
        // Tám lần đoán cùng lúc: trước đây mỗi lần đọc số lượt rồi mới so mã, nên cả loạt cùng thấy
        // dưới giới hạn và cùng được so — giới hạn 5 lần không chặn được dò mã song song.
        let guess = || public(&guests.shop, "POST", &confirm, None, None, Some(wrong.clone()));
        let results =
            tokio::join!(guess(), guess(), guess(), guess(), guess(), guess(), guess(), guess());
        for (status, _) in
            [results.0, results.1, results.2, results.3, results.4, results.5, results.6, results.7]
        {
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        }
        // Mỗi lần được so mã là một lần thử đã đếm: không lần nào quá 5.
        let row = lookup_code::Entity::find_by_id(uuid::Uuid::parse_str(id).unwrap())
            .one(guests.shop.database().read())
            .await
            .unwrap()
            .unwrap();
        assert!(row.attempts <= 5, "{} guesses were checked", row.attempts);
        let (status, _) =
            public(&guests.shop, "POST", &confirm, None, None, Some(json!({"code": right}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "mã hỏng sau 5 lần sai");
    })
    .await
    .unwrap();
}
