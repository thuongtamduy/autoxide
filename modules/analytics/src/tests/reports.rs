//! Gộp và báo cáo: số gộp khớp sự kiện thô, báo cáo chỉ đọc bảng gộp, phễu, website của nút khác
//! không thấy số liệu, hợp đồng `api` cho module khác, tool MCP chỉ đọc.

use autoxide_kernel::tenancy::OrgId;
use autoxide_testkit::{TestDatabase, tool};
use axum::http::StatusCode;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};

use super::{CHROME, Harness, SAFARI, at, browser, harness, page};
use crate::api::Analytics;
use crate::entity::{daily, event};

/// Một ngày (10/11) trên website `org`: ba khách.
/// - khách 1 (Chrome, 203.0.113.x): vào từ Google xem `/` rồi `/p/1` (xem sản phẩm, thêm giỏ);
/// - khách 2 (Safari, 198.51.100.x): vào từ chiến dịch Facebook xem `/p/1` (xem sản phẩm), thoát;
/// - khách 3 (Chrome, 192.0.2.x): vào thẳng xem `/`, thoát.
async fn one_day(h: &Harness, org: Option<OrgId>) {
    let first = browser("203.0.113.1", CHROME);
    let second = browser("198.51.100.1", SAFARI);
    let third = browser("192.0.2.1", CHROME);
    let product = |kind: &str| json!({"type": kind, "path": "/p/1", "referrer": "https://shop.example/", "object": "shop.product:p-1"});
    h.send(org, &first, None, vec![
        json!({"type": "analytics.page_view", "path": "/", "referrer": "https://www.google.com/"}),
        json!({"type": "analytics.page_view", "path": "/p/1", "referrer": "https://shop.example/"}),
        product("shop.product_viewed"),
        product("shop.cart_added"),
    ], at(10, 9))
    .await;
    h.send(org, &second, None, vec![
        json!({"type": "analytics.page_view", "path": "/p/1", "utm": {"source": "facebook", "campaign": "tet"}}),
        json!({"type": "shop.product_viewed", "path": "/p/1", "object": "shop.product:p-1"}),
    ], at(10, 10))
    .await;
    h.send(org, &third, None, vec![page("/")], at(10, 11)).await;
}

async fn rows(h: &Harness, dimension: &str) -> Vec<(String, i64, i64, i64, i64)> {
    let mut rows: Vec<_> = daily::Entity::find()
        .filter(daily::Column::Dimension.eq(dimension))
        .all(h.app.database().read())
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.value, row.views, row.visitors, row.events, row.bounces))
        .collect();
    rows.sort();
    rows
}

#[tokio::test]
async fn gop_theo_ngay_khop_su_kien_tho() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        one_day(&h, None).await;
        assert_eq!(h.service.rollup_tenant(&h.tenant, at(10, 12)).await.unwrap(), 1);
        // Tổng: 4 lượt xem trang, 3 khách, 7 sự kiện, 2 phiên thoát (khách 2 và 3).
        assert_eq!(rows(&h, "total").await, [(String::new(), 4, 3, 7, 2)]);
        assert_eq!(
            rows(&h, "page").await,
            [("/".to_owned(), 2, 2, 0, 0), ("/p/1".to_owned(), 2, 2, 0, 0)]
        );
        // Nguồn: chỉ lượt xem đến từ ngoài website (bấm link trong website không tính).
        assert_eq!(
            rows(&h, "source").await,
            [
                ("direct".to_owned(), 1, 1, 0, 0),
                ("facebook".to_owned(), 1, 1, 0, 0),
                ("google.com".to_owned(), 1, 1, 0, 0),
            ]
        );
        assert_eq!(rows(&h, "campaign").await, [("tet".to_owned(), 1, 1, 0, 0)]);
        assert_eq!(rows(&h, "object").await, [("shop.product:p-1".to_owned(), 3, 2, 3, 0)]);
        assert_eq!(
            rows(&h, "type").await,
            [
                ("analytics.page_view".to_owned(), 0, 3, 4, 0),
                ("shop.cart_added".to_owned(), 0, 1, 1, 0),
                ("shop.product_viewed".to_owned(), 0, 2, 2, 0),
            ]
        );
        // Gộp lại (job chạy mỗi giờ): cùng kết quả, không nhân đôi.
        h.service.rollup_tenant(&h.tenant, at(10, 13)).await.unwrap();
        assert_eq!(rows(&h, "total").await, [(String::new(), 4, 3, 7, 2)]);
        // Ngày cũ đã gộp không gộp lại; ngày cũ chưa gộp thì gộp.
        assert_eq!(h.service.rollup_tenant(&h.tenant, at(20, 0)).await.unwrap(), 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bao_cao_chi_doc_bang_gop() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        one_day(&h, None).await;
        let app = &h.app;
        let get = |path: String| async move {
            let answer =
                app.get(&format!("/api/v1/analytics{path}")).token("boss").send().await.unwrap();
            assert_eq!(answer.status(), StatusCode::OK, "{}", answer.text());
            answer.json()["data"].clone()
        };
        let range = "from=2026-11-01&to=2026-11-30";
        // Chưa gộp: báo cáo trống dù có sự kiện thô.
        let overview = get(format!("/overview?{range}")).await;
        assert_eq!(overview["visitors"], 0);
        h.service.rollup_tenant(&h.tenant, at(10, 12)).await.unwrap();
        // Xoá sự kiện thô: báo cáo vẫn đủ.
        event::Entity::delete_many().exec(h.app.database().write()).await.unwrap();
        let overview = get(format!("/overview?{range}")).await;
        assert_eq!((overview["visitors"].clone(), overview["views"].clone()), (json!(3), json!(4)));
        assert_eq!(overview["bounces"], 2);
        assert_eq!(overview["bounce_rate"], 0.6667);
        assert_eq!(overview["days"], json!([{"day": "2026-11-10", "views": 4, "visitors": 3}]));
        assert_eq!(overview["top_sources"].as_array().unwrap().len(), 3);
        let pages = get(format!("/pages?{range}&limit=1")).await;
        assert_eq!(pages, json!([{"value": "/", "views": 2, "visitors": 2, "events": 0}]));
        let campaigns = get(format!("/campaigns?{range}")).await;
        assert_eq!(campaigns[0]["value"], "tet");
        let objects = get(format!("/objects?{range}&kind=shop.product")).await;
        assert_eq!(objects[0]["value"], "shop.product:p-1");
        let types = get(format!("/types?{range}")).await;
        assert_eq!(types[0]["value"], "analytics.page_view");
        let funnel = get(format!("/funnels/shop.checkout?{range}")).await;
        let sessions: Vec<Value> = funnel["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|step| step["sessions"].clone())
            .collect();
        assert_eq!(sessions, [json!(2), json!(1), json!(0)]);
        assert_eq!(funnel["steps"][1]["from_start"], 0.5);

        // Khoảng quá 400 ngày, ngược, phễu lạ, website lạ.
        let code = |path: String| async move {
            let answer = app.get(&format!("/api/v1/analytics{path}")).token("boss").send();
            answer.await.unwrap().json()["code"].clone()
        };
        assert_eq!(
            code("/overview?from=2025-01-01&to=2026-11-30".into()).await,
            "analytics.range_too_long"
        );
        assert_eq!(
            code("/overview?from=2026-11-30&to=2026-11-01".into()).await,
            "analytics.range_reversed"
        );
        assert_eq!(code("/funnels/shop.nothing".into()).await, "analytics.funnel_unknown");
        let unknown = uuid::Uuid::now_v7();
        assert_eq!(code(format!("/overview?site={unknown}")).await, "analytics.site_unknown");
        assert_eq!(code("/objects?kind=Bad%20Kind".into()).await, "analytics.kind_invalid");
        // Người mua không có `analytics.read`.
        let answer = h.app.get("/api/v1/analytics/overview").token("buyer").send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::FORBIDDEN);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn website_cua_nut_khac_khong_thay_so_lieu() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        one_day(&h, Some(h.tree.a)).await;
        h.service.rollup_tenant(&h.tenant, at(10, 12)).await.unwrap();
        let app = &h.app;
        let visitors = |site: OrgId, who: &'static str| {
            let uri = format!(
                "/api/v1/analytics/overview?site={}&from=2026-11-10&to=2026-11-10",
                site.as_uuid()
            );
            async move {
                let answer = app.get(&uri).token(who).send().await.unwrap();
                (answer.status(), answer.json())
            }
        };
        // Website A: người của nhánh A và người đọc mọi website thấy; website B và cả khách: trống.
        let (status, body) = visitors(h.tree.a, "north").await;
        assert_eq!((status, body["data"]["visitors"].clone()), (StatusCode::OK, json!(3)));
        assert_eq!(visitors(h.tree.a, "boss").await.1["data"]["visitors"], 3);
        assert_eq!(visitors(h.tree.b, "boss").await.1["data"]["visitors"], 0);
        let root = autoxide_kernel::tenancy::root_of(h.tenant.id());
        assert_eq!(visitors(root, "boss").await.1["data"]["visitors"], 0);
        // Người của nhánh A không xem được website B.
        let (status, body) = visitors(h.tree.b, "north").await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["code"], "org_out_of_scope");
        // Hợp đồng `api`: thịnh hành theo website.
        let top = h.service.top_objects(&h.tenant, Some(h.tree.a), "shop.product", 400, 5).await;
        let top = top.unwrap();
        assert_eq!((top[0].id.as_str(), top[0].views), ("p-1", 3));
        let none = h.service.top_objects(&h.tenant, Some(h.tree.b), "shop.product", 400, 5).await;
        assert!(none.unwrap().is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tool_mcp_chi_doc_va_theo_quyen() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        one_day(&h, None).await;
        h.service.rollup_tenant(&h.tenant, at(10, 12)).await.unwrap();
        let client = h.app.mcp("boss").await.unwrap();
        let tools = client.list_all_tools().await.unwrap();
        let mine: Vec<_> = tools.iter().filter(|tool| tool.name.starts_with("analytics")).collect();
        assert_eq!(mine.len(), 4, "{:?}", tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>());
        assert!(mine.iter().all(|tool| {
            tool.annotations.as_ref().and_then(|a| a.read_only_hint) == Some(true)
        }));
        let name = mine.iter().find(|tool| tool.name.ends_with("top_pages")).unwrap().name.clone();
        let result = client
            .call_tool(tool(&name, json!({"from": "2026-11-01", "to": "2026-11-30", "limit": 1})))
            .await
            .unwrap();
        let text = format!("{:?}", result.content);
        assert!(text.contains("/p/1") || text.contains("\\\"/\\\""), "{text}");
    })
    .await
    .unwrap();
}
