//! Phần của nhà vận hành nền tảng dùng được qua API (cả SQLite và Postgres): gói và giá, đăng ký của
//! khách, hạn mức, ngoại lệ, tạm dừng, sản phẩm cửa hàng ứng dụng, mở khách theo gói; thông báo (mẫu
//! nháp, nhật ký gửi, push trình duyệt); webhook gửi lại.

use autoxide_testkit::{FakeProviderServer, TestDatabase};
use serde_json::{Value, json};

use super::kit::{ADMIN, app_with, call_tool, drain, expect, get, id};

#[tokio::test]
async fn goi_gia_dang_ky_han_muc_va_mo_khach() {
    TestDatabase::run(async |test| {
        let app = app_with(test, |builder| {
            builder
                .setting("AUTOXIDE_PLANS_CACHE_SECONDS", "0")
                .tenant("acme")
                .user_in("acme", "owner", 0x0a, &["plans.billing.manage", "plans.subscriptions.read"])
        })
        .await;
        let acme = app.tenant("acme").await.unwrap().id().to_string();

        let plan = call_tool(&app, ADMIN, "plans.create_plan", json!({"code": "basic", "name": "Basic", "status": "active"})).await;
        let plan_id = plan["id"].as_str().unwrap_or_else(|| panic!("{plan}")).to_owned();
        call_tool(&app, ADMIN, "plans.list_plans", json!({})).await;
        let full = call_tool(&app, ADMIN, "plans.get_plan", json!({"id": plan_id})).await;
        let mut replaced = full.clone();
        replaced["description"] = json!("For small shops");
        for key in ["id", "prices", "created_at", "updated_at", "subscriber_count"] {
            replaced.as_object_mut().unwrap().remove(key);
        }
        expect(&app, "PUT", &format!("/api/v1/plans/plans/{plan_id}"), ADMIN, Some(replaced.clone()), 200).await;
        replaced["id"] = json!(plan_id);
        call_tool(&app, ADMIN, "plans.update_plan", replaced).await;

        let price = json!({"kind": "recurring", "interval": "month", "amount": {"amount": "199000", "currency": "VND"}});
        let price = expect(&app, "POST", &format!("/api/v1/plans/plans/{plan_id}/prices"), ADMIN, Some(price), 201).await;
        // Trả gói kèm giá mới.
        let price_id = price["data"]["prices"][0]["id"].as_str().unwrap_or_else(|| panic!("{price}")).to_owned();

        call_tool(&app, ADMIN, "plans.assign_subscription", json!({"tenant_id": acme, "plan_id": plan_id, "trial": false})).await;
        call_tool(&app, ADMIN, "plans.get_subscription", json!({"tenant_id": acme})).await;
        call_tool(&app, ADMIN, "plans.list_usage", json!({"tenant_id": acme})).await;
        get(&app, &format!("/api/v1/plans/tenants/{acme}/usage")).await;
        call_tool(
            &app,
            ADMIN,
            "plans.set_override",
            json!({"tenant_id": acme, "kind": "module", "key": "content", "enabled": true, "reason": "Pilot"}),
        )
        .await;

        // Chủ khách tự chuyển lên gói đắt hơn.
        let pro = call_tool(&app, ADMIN, "plans.create_plan", json!({"code": "pro", "name": "Pro", "status": "active"})).await;
        let pro_id = pro["id"].as_str().unwrap().to_owned();
        let price = json!({"kind": "recurring", "interval": "month", "amount": {"amount": "299000", "currency": "VND"}});
        let pro = expect(&app, "POST", &format!("/api/v1/plans/plans/{pro_id}/prices"), ADMIN, Some(price), 201).await;
        let pro_price = pro["data"]["prices"][0]["id"].clone();
        let change = json!({"plan_id": pro_id, "price_id": pro_price, "reference": "upgrade-1"});
        call_tool(&app, "owner", "plans.change_subscription", change).await;

        expect(&app, "POST", &format!("/api/v1/plans/prices/{price_id}/deactivate"), ADMIN, None, 200).await;

        // Sản phẩm của cửa hàng ứng dụng gắn với gói rồi gỡ.
        let link = "/api/v1/plans/store/products/app_store/com.example.basic";
        expect(&app, "PUT", link, ADMIN, Some(json!({"plan_id": plan_id})), 200).await;
        let products = get(&app, "/api/v1/plans/store/products").await;
        assert_eq!(products["data"].as_array().unwrap().len(), 1, "{products}");
        expect(&app, "DELETE", link, ADMIN, None, 204).await;

        // Mở khách mới theo gói; gói khác thôi bán.
        let provisioned = call_tool(
            &app,
            ADMIN,
            "plans.provision_tenant",
            json!({"name": "Gamma", "slug": "gamma", "admin_email": "owner@gamma.example", "plan_id": plan_id}),
        )
        .await;
        let provisioning = provisioned["id"].as_str().unwrap_or_else(|| panic!("{provisioned}")).to_owned();
        call_tool(&app, ADMIN, "plans.get_provisioning", json!({"id": provisioning})).await;
        call_tool(&app, ADMIN, "plans.list_signups", json!({})).await;
        let old = call_tool(&app, ADMIN, "plans.create_plan", json!({"code": "old", "name": "Old", "status": "active"})).await;
        call_tool(&app, ADMIN, "plans.retire_plan", json!({"id": old["id"]})).await;

        let reason = json!({"reason": "Unpaid invoices"});
        expect(&app, "POST", &format!("/api/v1/plans/tenants/{acme}/subscription/suspend"), ADMIN, Some(reason), 204).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn thong_bao_mau_nhap_nhat_ky_gui_va_push() {
    TestDatabase::run(async |test| {
        let app = app_with(test, |builder| builder).await;
        get(&app, "/api/v1/notifications/branding").await;
        call_tool(&app, ADMIN, "notifications.list_kinds", json!({})).await;

        let kinds = get(&app, "/api/v1/notifications/kinds").await;
        let kind = kinds["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|kind| kind["channels"].as_array().is_some_and(|channels| channels.contains(&json!("email"))))
            .unwrap_or_else(|| panic!("no email kind in {kinds}"))["key"]
            .as_str()
            .unwrap()
            .to_owned();
        let draft = format!("/api/v1/notifications/templates/{kind}/email/en/draft");
        let body = json!({"subject": "Hello", "body_text": "Hello there"});
        expect(&app, "PUT", &draft, ADMIN, Some(body), 200).await;
        get(&app, &draft).await;
        expect(&app, "DELETE", &draft, ADMIN, None, 204).await;

        let send = json!({"kind": kind, "recipient": {"email": "x@example.com"}, "dedupe_key": "usable-1"});
        expect(&app, "POST", "/api/v1/notifications/messages", ADMIN, Some(send), 200).await;
        drain(&app).await;
        let messages = call_tool(&app, ADMIN, "notifications.list_messages", json!({})).await;
        let message = messages[0]["id"].as_str().unwrap_or_else(|| panic!("{messages}")).to_owned();
        get(&app, &format!("/api/v1/notifications/messages/{message}")).await;

        let subscription = json!({"endpoint": "https://push.example/send/abc",
                                  "keys": {"p256dh": "B".repeat(87), "auth": "a".repeat(22)}});
        let created = expect(&app, "POST", "/api/v1/notifications/me/push-subscriptions", ADMIN, Some(subscription), 201).await;
        let path = format!("/api/v1/notifications/me/push-subscriptions/{}", id(&created));
        expect(&app, "DELETE", &path, ADMIN, None, 204).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn webhook_giao_roi_gui_lai() {
    TestDatabase::run(async |test| {
        let server = FakeProviderServer::start().await.unwrap();
        server.reply("POST", "/hook", 200, json!({}));
        let app = app_with(test, |builder| {
            builder
                .setting("AUTOXIDE_WEBHOOKS_ALLOW_HTTP", "true")
                .setting("AUTOXIDE_WEBHOOKS_ALLOW_PRIVATE_NETWORKS", "true")
        })
        .await;
        let hook = json!({"url": format!("{}/hook", server.url()), "events": ["*"]});
        expect(&app, "POST", "/api/v1/webhooks", ADMIN, Some(hook), 201).await;
        let entry = json!({"kind": "post", "title": "Hello"});
        expect(&app, "POST", "/api/v1/content/entries", ADMIN, Some(entry), 201).await;
        drain(&app).await;
        let deliveries = get(&app, "/api/v1/webhooks/deliveries").await;
        let delivery: Value = deliveries["data"][0]["id"].clone();
        assert!(delivery.is_string(), "{deliveries}");
        let delivery = delivery.as_str().unwrap();
        expect(
            &app,
            "POST",
            &format!("/api/v1/webhooks/deliveries/{delivery}/redeliver"),
            ADMIN,
            None,
            202,
        )
        .await;
        drain(&app).await;
        call_tool(&app, ADMIN, "webhooks.redeliver", json!({"id": delivery})).await;
        assert!(server.take_requests().len() >= 2);
    })
    .await
    .unwrap();
}
