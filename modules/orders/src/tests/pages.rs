//! Danh sách có phân trang (bước 70 P9 mục 3, D-20261125-list-pages): quy tắc và danh sách chặn COD, báo cáo bán
//! hàng (keyset trên tổng của nhóm, cùng tổng thì theo khoá), tra bảo hành qua REST và tool MCP cùng trang.

use super::CatalogExt as _;
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::TestDatabase;
use autoxide_testkit::mcp::CallToolRequestParams;
use axum::http::StatusCode;
use jiff::SignedDuration;
use sea_orm::{ActiveModelTrait, Set};
use serde_json::{Value, json};
use uuid::Uuid;

use super::checkout::{body, checkout, ready_cart};
use super::{Call, Shop, shop, shop_with};
use crate::entity::warranty;
use autoxide_kernel::tenancy::OrgId;

fn tool(name: &str, arguments: Value) -> CallToolRequestParams {
    CallToolRequestParams::new(name.to_owned())
        .with_arguments(arguments.as_object().unwrap().clone())
}

/// Theo `next_cursor` của `uri` (đã có `limit`, có `?`) tới hết; trả trường `field` của mọi dòng theo thứ tự.
async fn walk(shop: &Shop, uri: &str, field: &str) -> Vec<Value> {
    let boss = Call::as_user(shop, "boss");
    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page_uri = match &cursor {
            Some(cursor) => format!("{uri}&cursor={cursor}"),
            None => uri.to_owned(),
        };
        let (status, page) = boss.send("GET", &page_uri, None).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        seen.extend(page["data"].as_array().unwrap().iter().map(|row| row[field].clone()));
        match page["pagination"]["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => return seen,
        }
    }
}

/// Một đơn COD của `buyer1` ở nút `org`, mỗi đơn một sản phẩm mới giá 100.000.
async fn order_at(shop: &Shop, org: OrgId) {
    let item = shop.catalog.add(org, 100_000);
    shop.inventory.stock(org, item, 5);
    let buyer = Call::as_user(shop, "buyer1");
    let (cart, _, quote) = ready_cart(&buyer, org, item, 1).await;
    let (status, placed) =
        checkout(&buyer, Some(&Uuid::now_v7().to_string()), body(&cart, &quote, "cod")).await;
    assert_eq!(status, StatusCode::CREATED, "{placed}");
}

fn urlencode(text: &str) -> String {
    text.replace('+', "%2B").replace(':', "%3A")
}

#[tokio::test]
async fn quy_tac_va_danh_sach_chan_cod_theo_trang() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let boss = Call::as_user(&shop, "boss");
        let mut rules = Vec::new();
        for min_orders in 1..=5 {
            let rule = json!({"org_id": shop.tree.a.as_uuid(), "min_orders": min_orders, "max_fail_ratio_bp": 5000});
            let (status, created) = boss.send("POST", "/risk/cod-rules", Some(rule)).await;
            assert_eq!(status, StatusCode::CREATED, "{created}");
            rules.push(created["data"]["id"].clone());
        }
        // Mới nhất trước (mặc định), cũ nhất trước với `order=asc`.
        let newest: Vec<Value> = rules.iter().rev().cloned().collect();
        assert_eq!(walk(&shop, "/risk/cod-rules?limit=2", "id").await, newest);
        assert_eq!(walk(&shop, "/risk/cod-rules?limit=2&order=asc", "id").await, rules);

        // Danh sách chặn: hết hạn sớm nhất trước.
        let mut blocks = Vec::new();
        for (days, phone) in [(30, "0977000111"), (10, "0977000222"), (20, "0977000333")] {
            let expires = Timestamp::now() + SignedDuration::from_hours(24 * days);
            let block = json!({"phone": phone, "reason_code": "fake_orders", "expires_at": expires});
            let (status, created) = boss.send("POST", "/risk/cod-blocks", Some(block)).await;
            assert_eq!(status, StatusCode::CREATED, "{created}");
            blocks.push((days, created["data"]["id"].clone()));
        }
        blocks.sort_by_key(|(days, _)| *days);
        let soonest: Vec<Value> = blocks.into_iter().map(|(_, id)| id).collect();
        assert_eq!(walk(&shop, "/risk/cod-blocks?limit=1", "id").await, soonest);
        let (_, counted) = boss.send("GET", "/risk/cod-blocks?limit=1&count=true", None).await;
        assert_eq!(counted["pagination"]["total"], 3);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bao_cao_ban_hang_theo_trang_tren_tong_cua_nhom() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        for org in [shop.tree.a, shop.tree.a, shop.tree.b] {
            order_at(&shop, org).await;
        }
        let from = urlencode(&(Timestamp::now() - SignedDuration::from_hours(24)).to_rfc3339());
        let to = urlencode(&(Timestamp::now() + SignedDuration::from_hours(24)).to_rfc3339());
        let uri = |group: &str, extra: &str| {
            format!("/reports/sales?group_by={group}&from={from}&to={to}&limit=1{extra}")
        };
        // Theo nút: A (2 đơn) trước B; `order=desc` đảo lại.
        let (a, b) =
            (json!(shop.tree.a.as_uuid().to_string()), json!(shop.tree.b.as_uuid().to_string()));
        assert_eq!(walk(&shop, &uri("org", ""), "key").await, vec![a.clone(), b.clone()]);
        assert_eq!(walk(&shop, &uri("org", "&order=desc"), "key").await, vec![b, a]);
        // Theo sản phẩm: ba sản phẩm cùng tổng (hoà) — theo khoá, không trùng, không sót.
        let products = walk(&shop, &uri("product", ""), "key").await;
        assert_eq!(products.len(), 3, "{products:?}");
        let mut sorted = products.clone();
        sorted.sort_by(|x, y| x.as_str().cmp(&y.as_str()));
        sorted.dedup();
        assert_eq!(products, sorted);
        let mut reversed = walk(&shop, &uri("product", "&order=desc"), "key").await;
        reversed.reverse();
        assert_eq!(reversed, products);
        // Theo ngày (`page_of`): một ngày; con trỏ của cách gom khác bị từ chối.
        assert_eq!(walk(&shop, &uri("day", ""), "orders").await, vec![json!(3)]);
        let boss = Call::as_user(&shop, "boss");
        let (_, first) = boss.send("GET", &uri("org", ""), None).await;
        let cursor = first["pagination"]["next_cursor"].as_str().unwrap().to_owned();
        let (status, wrong) =
            boss.send("GET", &format!("{}&cursor={cursor}", uri("channel", "")), None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{wrong}");
        // Cách gom sai và `limit` sai: một lỗi 422 nêu cả hai ô.
        let (status, error) = boss
            .send("GET", &format!("/reports/sales?group_by=x&from={from}&to={to}&limit=0"), None)
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
        let fields: Vec<&str> = error["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["field"].as_str().unwrap())
            .collect();
        assert!(fields.contains(&"group_by") && fields.contains(&"limit"), "{error}");
    })
    .await
    .unwrap();
}

/// Tool `orders.lookup_warranty` và `GET /api/v1/orders/warranties` cùng tham số trả cùng trang và cùng
/// `next_cursor`; theo `next_cursor` của tool tới hết được đúng phần còn lại.
#[tokio::test]
async fn tool_tra_bao_hanh_tra_dung_trang_nhu_rest() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let tenant = shop.default_tenant();
        let mut ids = Vec::new();
        for index in 0..3 {
            let row = warranty::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                org_id: Set(shop.tree.a.as_uuid()),
                order_id: Set(Uuid::now_v7()),
                order_line_id: Set(Uuid::now_v7()),
                product_id: Set(Uuid::now_v7()),
                variant_id: Set(Uuid::now_v7()),
                serial: Set(Some("SN-PAGE".to_owned())),
                serial_key: Set(format!("SN-PAGE-{index}")),
                buyer_user_id: Set(None),
                buyer_key: Set(None),
                starts_at: Set(Timestamp::now()),
                ends_at: Set(Timestamp::now() + SignedDuration::from_hours(24 * 365)),
                status: Set("active".to_owned()),
                created_at: Set(Timestamp::now()),
            }
            .insert(tenant.database().write())
            .await
            .unwrap();
            ids.push(json!(row.id.to_string()));
        }
        let (status, rest) = Call::as_user(&shop, "boss")
            .send("GET", "/warranties?serial=SN-PAGE&limit=2", None)
            .await;
        assert_eq!(status, StatusCode::OK, "{rest}");
        let client = shop.mcp("boss").await.unwrap();
        let first = client
            .call_tool(tool("orders.lookup_warranty", json!({"serial": "SN-PAGE", "limit": 2})))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(first["data"], rest["data"]);
        assert_eq!(first["pagination"], rest["pagination"]);
        let cursor = first["pagination"]["next_cursor"].clone();
        assert!(cursor.is_string());
        let second = client
            .call_tool(tool(
                "orders.lookup_warranty",
                json!({"serial": "SN-PAGE", "limit": 2, "cursor": cursor, "count": true}),
            ))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        let mut walked: Vec<Value> =
            first["data"].as_array().unwrap().iter().map(|row| row["id"].clone()).collect();
        walked.extend(second["data"].as_array().unwrap().iter().map(|row| row["id"].clone()));
        assert!(second["pagination"]["next_cursor"].is_null());
        ids.reverse();
        assert_eq!(walked, ids, "mới nhất trước");
        client.cancel().await.unwrap();
    })
    .await
    .unwrap();
}

/// Thiếu quyền thì 403 trước khi tham số trang sai thành 422 (luật `permission-first` của xtask,
/// D-20261126-permission-before-page). Dựng không kèm lớp `x-permissions` của kernel để thấy chính handler kiểm.
#[tokio::test]
async fn thieu_quyen_la_403_truoc_tham_so_trang() {
    TestDatabase::run(async |test| {
        let shop =
            shop_with(test, |builder, _| builder.without_permission_gate().user("nobody", 9, &[]))
                .await;
        let nobody = Call::as_user(&shop.app, "nobody");
        for uri in ["/returns?limit=0", "/reviews?limit=0"] {
            let (status, problem) = nobody.send("GET", uri, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{uri}: {problem}");
            assert_eq!(problem["code"], "permission_denied", "{uri}: {problem}");
        }
        // Có quyền thì cùng tham số là lỗi tham số.
        let boss = Call::as_user(&shop.app, "boss");
        for uri in ["/returns?limit=0", "/reviews?limit=0"] {
            let (status, problem) = boss.send("GET", uri, None).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{uri}: {problem}");
        }
    })
    .await
    .unwrap();
}
