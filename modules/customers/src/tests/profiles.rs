//! Hồ sơ khách: dựng từ event đơn, nhận diện theo số đã xác thực, che liên hệ, phạm vi chi nhánh,
//! dữ liệu cá nhân.

use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use serde_json::json;
use uuid::Uuid;

use super::{BUYER, Order, call, drain, events_named, minor, shop};

#[tokio::test]
async fn profile_built_from_order_events_once() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let order = Order::of(BUYER, shop.tree.a, 300_000);
        // Mỗi event giao hai lần: tổng chi, số đơn không cộng đôi.
        for _ in 0..2 {
            shop.order_event(&order.placed()).await;
        }
        drain(&shop).await;
        for _ in 0..2 {
            shop.order_event(&order.status("delivered")).await;
            shop.order_event(&order.status("completed")).await;
        }
        drain(&shop).await;
        let (status, me) = call(&shop, "GET", "/me", "buyer", None).await;
        assert_eq!(status, StatusCode::OK, "{me}");
        assert_eq!(me["data"]["orders_count"], 1);
        assert_eq!(minor(&me["data"]["total_spent"]), 300_000);

        // Hoàn 100.000 (giao hai lần) → tổng chi 200.000.
        let refund = Uuid::now_v7();
        shop.order_event(&order.refunded(refund, 100_000)).await;
        shop.order_event(&order.refunded(refund, 100_000)).await;
        drain(&shop).await;
        let id = me["data"]["id"].as_str().unwrap().to_owned();
        let (_, profile) = call(&shop, "GET", &format!("/{id}"), "boss", None).await;
        assert_eq!(minor(&profile["data"]["total_spent"]), 200_000);
        assert_eq!(profile["data"]["org_id"], json!(shop.tree.a));
        let (_, orders) = call(&shop, "GET", &format!("/{id}/orders"), "boss", None).await;
        let rows = orders["data"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["status"], "completed");
        assert_eq!(minor(&rows[0]["refunded"]), 100_000);

        // Đơn bị huỷ không tính vào số đơn.
        let cancelled = Order::of(BUYER, shop.tree.a, 50_000);
        shop.order_event(&cancelled.placed()).await;
        shop.order_event(&cancelled.status("cancelled")).await;
        shop.order_event(&cancelled.status("cancelled")).await;
        drain(&shop).await;
        let (_, profile) = call(&shop, "GET", &format!("/{id}"), "boss", None).await;
        assert_eq!(profile["data"]["orders_count"], 1);
        assert_eq!(minor(&profile["data"]["total_spent"]), 200_000);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn guest_orders_with_same_phone_key_share_one_profile() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        for _ in 0..2 {
            let mut order = Order::of(0, shop.tree.b, 100_000);
            order.buyer = None;
            order.buyer_key = Some("phone-key-1".to_owned());
            shop.order_event(&order.placed()).await;
        }
        drain(&shop).await;
        let (_, list) = call(&shop, "GET", "", "boss", None).await;
        let rows = list["data"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{list}");
        assert_eq!(rows[0]["orders_count"], 2);
        assert_eq!(rows[0]["user_id"], json!(null));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn phone_profile_links_only_to_verified_phone_account() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let body = json!({"phone": "0912 345 412", "full_name": "Nguyễn Văn Nam"});
        let (status, created) = call(&shop, "POST", "", "boss", Some(body.clone())).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let counter = created["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(created["data"]["phone_masked"], "09•• ••• 412");
        // Cùng số lần hai → 409 kèm ID hồ sơ có sẵn.
        let (status, taken) = call(&shop, "POST", "", "boss", Some(body)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(taken["code"], "customers.phone_taken");
        assert!(taken["detail"].as_str().unwrap().contains(&counter), "{taken}");

        // Tài khoản chưa xác thực số: hồ sơ mới, không chiếm hồ sơ ở quầy.
        let (_, mine) = call(&shop, "GET", "/me", "buyer2", None).await;
        assert_ne!(mine["data"]["id"].as_str().unwrap(), counter);
        assert_eq!(mine["data"]["phone"], json!(null));

        // Tài khoản đã xác thực đúng số: hồ sơ ở quầy về tài khoản.
        shop.accounts.0.lock().unwrap().insert(Uuid::from_u128(103), "+84912345412".to_owned());
        let (_, mine) = call(&shop, "GET", "/me", "buyer3", None).await;
        assert_eq!(mine["data"]["id"].as_str().unwrap(), counter);
        assert_eq!(mine["data"]["phone"], "+84912345412");
        assert_eq!(mine["data"]["full_name"], "Nguyễn Văn Nam");
        assert_eq!(events_named(&shop, "customers.profile_linked").await.len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn contact_is_masked_and_reveal_is_audited() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let body =
            json!({"phone": "0912345412", "email": "Nam.Nguyen@Gmail.com", "full_name": "Nam"});
        let (_, created) = call(&shop, "POST", "", "boss", Some(body)).await;
        let id = created["data"]["id"].as_str().unwrap().to_owned();

        let (_, list) = call(&shop, "GET", "?q=nam", "reader", None).await;
        let row = &list["data"][0];
        assert_eq!(row["phone_masked"], "09•• ••• 412");
        assert_eq!(row["email_masked"], "n•••@gmail.com");
        assert!(row.get("phone").is_none() && row.get("email").is_none(), "{row}");
        // Tìm theo số (dạng khác) và theo email.
        let (_, by_phone) = call(&shop, "GET", "?q=%2B84912345412", "reader", None).await;
        assert_eq!(by_phone["data"].as_array().unwrap().len(), 1);
        let (_, by_email) = call(&shop, "GET", "?q=nam.nguyen%40gmail.com", "reader", None).await;
        assert_eq!(by_email["data"].as_array().unwrap().len(), 1);

        let (status, _) = call(&shop, "GET", &format!("/{id}/contact"), "reader", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, contact) =
            call(&shop, "GET", &format!("/{id}/contact?reason=giao%20h%C3%A0ng"), "boss", None)
                .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(contact["data"]["phone"], "+84912345412");
        assert_eq!(contact["data"]["email"], "nam.nguyen@gmail.com");
        let audited = events_named(&shop, "customers.contact_revealed").await;
        assert_eq!(audited.len(), 1);
        assert_eq!(audited[0]["after"]["reason"], "giao hàng");
        assert_eq!(audited[0]["after"]["viewer_id"], json!(Uuid::from_u128(1)));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn store_staff_sees_only_own_branch_profiles() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let (status, _) = call(
            &shop,
            "POST",
            "",
            "staff_a",
            Some(json!({
                "phone": "0911111111", "org_id": shop.tree.a,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, refused) = call(
            &shop,
            "POST",
            "",
            "staff_a",
            Some(json!({
                "phone": "0922222222", "org_id": shop.tree.b,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
        let (_, other) = call(
            &shop,
            "POST",
            "",
            "boss",
            Some(json!({
                "phone": "0933333333", "org_id": shop.tree.b,
            })),
        )
        .await;
        let other = other["data"]["id"].as_str().unwrap().to_owned();

        let (_, list) = call(&shop, "GET", "", "staff_a", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 1);
        let (status, _) = call(&shop, "GET", &format!("/{other}"), "staff_a", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = call(
            &shop,
            "PUT",
            &format!("/{other}"),
            "staff_a",
            Some(json!({
                "full_name": "X",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (_, list) = call(&shop, "GET", "", "boss", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn update_checks_values_once_and_if_match() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let (_, created) =
            call(&shop, "POST", "", "boss", Some(json!({"phone": "0911111111"}))).await;
        let id = created["data"]["id"].as_str().unwrap().to_owned();
        let (status, problem) = call(
            &shop,
            "PUT",
            &format!("/{id}"),
            "boss",
            Some(json!({
                "email": "not-an-email", "gender": "x", "birthday": "2999-01-01",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem["errors"].as_array().unwrap().len(), 3, "{problem}");

        let answer = shop
            .request(axum::http::Method::PUT, &format!("/api/v1/customers/{id}"))
            .token("boss")
            .header("If-Match", "\"2000-01-01T00:00:00Z\"")
            .json(json!({"full_name": "Lan"}))
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), StatusCode::PRECONDITION_FAILED);
        let (status, updated) = call(
            &shop,
            "PUT",
            &format!("/{id}"),
            "boss",
            Some(json!({
                "full_name": "Trần Thị Lan", "gender": "female",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(updated["data"]["full_name"], "Trần Thị Lan");
        // Tìm không dấu.
        let (_, found) = call(&shop, "GET", "?q=tran%20thi", "boss", None).await;
        assert_eq!(found["data"].as_array().unwrap().len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn concurrent_first_visits_create_one_profile() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let visit = || call(&shop, "GET", "/me", "buyer", None);
        let answers = tokio::join!(visit(), visit(), visit(), visit(), visit());
        let answers = [answers.0, answers.1, answers.2, answers.3, answers.4];
        let ids: std::collections::BTreeSet<String> = answers
            .iter()
            .map(|(status, body)| {
                assert_eq!(*status, StatusCode::OK, "{body}");
                body["data"]["id"].as_str().unwrap().to_owned()
            })
            .collect();
        assert_eq!(ids.len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn export_and_user_deleted_keep_totals_drop_personal_fields() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        shop.accounts.0.lock().unwrap().insert(Uuid::from_u128(BUYER), "0912345412".to_owned());
        call(&shop, "PUT", "/me", "buyer", Some(json!({"full_name": "Nam", "email": "n@x.vn"})))
            .await;
        let order = Order::of(BUYER, shop.tree.a, 120_000);
        shop.order_event(&order.placed()).await;
        shop.order_event(&order.status("completed")).await;
        drain(&shop).await;
        let tenant = shop.default_tenant();
        let exported =
            shop.modules().personal_data().export(&tenant, Uuid::from_u128(BUYER)).await.unwrap();
        let mine = &exported["customers"];
        assert_eq!(mine["profile"]["full_name"], "Nam", "{exported:?}");
        assert_eq!(mine["orders"].as_array().unwrap().len(), 1);

        shop.user_deleted(Uuid::from_u128(BUYER)).await;
        drain(&shop).await;
        let (_, list) = call(&shop, "GET", "", "boss", None).await;
        let row = &list["data"][0];
        assert_eq!(row["full_name"], json!(null));
        assert_eq!(row["phone_masked"], json!(null));
        assert_eq!(row["user_id"], json!(null));
        assert_eq!(minor(&row["total_spent"]), 120_000);
    })
    .await
    .unwrap();
}
