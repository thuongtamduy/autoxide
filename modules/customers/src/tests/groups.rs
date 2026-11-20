//! Nhóm khách: nhóm tay, nhóm theo quy tắc, cắm vào `orders` (nhóm, mã nhóm giá), MCP.

use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{BUYER, Order, call, drain, shop};

fn tool(name: &str, arguments: Value) -> CallToolRequestParams {
    CallToolRequestParams::new(name.to_owned())
        .with_arguments(arguments.as_object().unwrap().clone())
}

#[tokio::test]
async fn rule_group_membership_updates_and_feeds_orders_pricing() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let body = json!({
            "name": "VIP", "kind": "rule", "price_group": "vip",
            "rule": {"total_spent_min": 200000, "orders_count_min": 1},
        });
        let (status, created) = call(&shop, "POST", "/groups", "boss", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let group = created["data"]["id"].as_str().unwrap().to_owned();
        let tenant = shop.default_tenant();
        let buyer = Uuid::from_u128(BUYER);

        // Chưa mua: không thuộc nhóm, `orders` không thấy nhóm.
        let profile = shop.buyer_groups().profile_of(&tenant, buyer).await.unwrap();
        assert!(profile.groups.is_empty());

        let order = Order::of(BUYER, shop.tree.a, 300_000);
        shop.order_event(&order.placed()).await;
        shop.order_event(&order.status("completed")).await;
        drain(&shop).await;
        let (_, preview) =
            call(&shop, "GET", &format!("/groups/{group}/preview"), "boss", None).await;
        assert_eq!(preview["data"]["matching"], 1);
        let (_, one) = call(&shop, "GET", &format!("/groups/{group}"), "boss", None).await;
        assert_eq!(one["data"]["members"], 1);
        let profile = shop.buyer_groups().profile_of(&tenant, buyer).await.unwrap();
        assert_eq!(profile.groups, vec![group.parse::<Uuid>().unwrap()]);
        assert_eq!(profile.price_group.as_deref(), Some("vip"));
        let (_, in_group) = call(&shop, "GET", &format!("?group_id={group}"), "boss", None).await;
        assert_eq!(in_group["data"].as_array().unwrap().len(), 1);

        // Hoàn tiền xuống dưới ngưỡng → rời nhóm.
        shop.order_event(&order.refunded(Uuid::now_v7(), 150_000)).await;
        drain(&shop).await;
        let profile = shop.buyer_groups().profile_of(&tenant, buyer).await.unwrap();
        assert!(profile.groups.is_empty(), "{profile:?}");

        // Đổi quy tắc → cả nhóm đánh giá lại (job theo nhóm).
        let (status, _) = call(
            &shop,
            "PUT",
            &format!("/groups/{group}"),
            "boss",
            Some(json!({
                "rule": {"total_spent_min": 100000},
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        drain(&shop).await;
        let (_, one) = call(&shop, "GET", &format!("/groups/{group}"), "boss", None).await;
        assert_eq!(one["data"]["members"], 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn manual_groups_and_rule_checks() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let (status, problem) = call(
            &shop,
            "POST",
            "/groups",
            "boss",
            Some(json!({
                "name": "Bad", "kind": "rule", "rule": {"last_order_within_days": 0},
                "price_group": "Bad Code",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem["errors"].as_array().unwrap().len(), 2, "{problem}");

        let (_, manual) = call(&shop, "POST", "/groups", "boss", Some(json!({"name": "Sỉ"}))).await;
        let manual = manual["data"]["id"].as_str().unwrap().to_owned();
        let (status, taken) =
            call(&shop, "POST", "/groups", "boss", Some(json!({"name": "Sỉ"}))).await;
        assert_eq!(
            (status, taken["code"].as_str()),
            (StatusCode::CONFLICT, Some("customers.group_name_taken"))
        );
        let (_, rule) = call(
            &shop,
            "POST",
            "/groups",
            "boss",
            Some(json!({
                "name": "Mua gần đây", "kind": "rule", "rule": {"last_order_within_days": 30},
            })),
        )
        .await;
        let rule = rule["data"]["id"].as_str().unwrap().to_owned();

        let (_, created) =
            call(&shop, "POST", "", "boss", Some(json!({"phone": "0911111111"}))).await;
        let profile = created["data"]["id"].as_str().unwrap().to_owned();
        for _ in 0..2 {
            let (status, _) =
                call(&shop, "POST", &format!("/groups/{manual}/members/{profile}"), "boss", None)
                    .await;
            assert_eq!(status, StatusCode::NO_CONTENT);
        }
        let (_, one) = call(&shop, "GET", &format!("/groups/{manual}"), "boss", None).await;
        assert_eq!(one["data"]["members"], 1);
        let (status, problem) =
            call(&shop, "POST", &format!("/groups/{rule}/members/{profile}"), "boss", None).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("customers.group_not_manual"))
        );
        let (status, _) =
            call(&shop, "DELETE", &format!("/groups/{manual}/members/{profile}"), "boss", None)
                .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, one) = call(&shop, "GET", &format!("/groups/{manual}"), "boss", None).await;
        assert_eq!(one["data"]["members"], 0);

        let (status, _) = call(&shop, "DELETE", &format!("/groups/{manual}"), "boss", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = call(&shop, "GET", &format!("/groups/{manual}"), "boss", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // Người chỉ có `customers.read` xem được danh sách nhóm, không sửa được.
        let (status, list) = call(&shop, "GET", "/groups", "reader", None).await;
        assert_eq!((status, list["data"].as_array().unwrap().len()), (StatusCode::OK, 1));
        let (status, _) =
            call(&shop, "POST", "/groups", "reader", Some(json!({"name": "X"}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn mcp_tools_mask_contact_and_share_services() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        call(&shop, "POST", "", "boss", Some(json!({"phone": "0912345412", "full_name": "Hoa"})))
            .await;
        let client = shop.mcp("boss").await.unwrap();
        let found = client
            .call_tool(tool("customers.search_customers", json!({"q": "hoa"})))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        let row = &found["data"][0];
        assert_eq!(row["phone_masked"], "09•• ••• 412", "{found}");
        let id = row["id"].as_str().unwrap().to_owned();
        let contact = client
            .call_tool(tool("customers.reveal_contact", json!({"id": id, "reason": "gọi lại"})))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(contact["data"]["phone"], "+84912345412");
        let group = client
            .call_tool(tool("customers.upsert_group", json!({"name": "Thân thiết"})))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        let group_id = group["data"]["id"].as_str().unwrap().to_owned();
        let preview = client
            .call_tool(tool("customers.preview_group", json!({"id": group_id})))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(preview["data"]["matching"], 0);
    })
    .await
    .unwrap();
}
