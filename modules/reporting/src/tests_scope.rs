//! Test quyền xem báo cáo gộp (rò rỉ giữa khách, giữa nhánh), qua REST và MCP, trên cả SQLite và
//! Postgres.

use autoxide_kernel::access::{Grant, Principal, Scope};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::{OrgId, OrgTree, TenantContext, root_of};
use autoxide_testkit::{TestApp, TestDatabase, tool};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::test_support::{Sales, service, settle, tree};

struct Fixture {
    app: TestApp,
    parent: TenantContext,
    shop_a: TenantContext,
    shop_b: TenantContext,
    stranger: TenantContext,
    north: OrgId,
    south: OrgId,
}

fn vnd(minor: i64) -> Money {
    Money::new(minor, Currency::VND)
}

/// Cha có hai nhánh `north` (100) và `south` (200) cùng 1 000 ở nút gốc; `shop_a` bán 10 000,
/// `shop_b` 20 000, khách lạ 40 000.
async fn fixture(test: &TestDatabase) -> Fixture {
    let sales = Sales::default();
    let (north, south) = (OrgId::from_uuid(Uuid::now_v7()), OrgId::from_uuid(Uuid::now_v7()));
    let read = |user: u128, grant: Option<(OrgId, Scope)>| match grant {
        None => Principal::new(Uuid::from_u128(user), ["reporting.read".to_owned()]),
        Some((org, scope)) => Principal::with_grants(
            Uuid::from_u128(user),
            [("reporting.read".to_owned(), Grant::new(org, scope))],
        ),
    };
    let app = tree(TestApp::builder(test), &sales)
        .principal_in("parent", "boss", read(1, None))
        .principal_in("parent", "north", read(2, Some((north, Scope::Subtree))))
        .principal_in("parent", "unit", read(3, Some((south, Scope::Unit))))
        .principal_in("shop-a", "shop-a", read(4, None))
        .principal_in("stranger", "stranger", read(5, None))
        .principal_in("parent", "nobody", Principal::new(Uuid::from_u128(6), Vec::<String>::new()))
        .principal_in(
            "parent",
            "manager",
            Principal::new(
                Uuid::from_u128(7),
                ["reporting.read", "reporting.manage"].map(str::to_owned),
            ),
        )
        .build()
        .await
        .unwrap();
    let tenant = async |slug: &str| app.tenant(slug).await.unwrap();
    let (parent, shop_a, shop_b, stranger) = (
        tenant("parent").await,
        tenant("shop-a").await,
        tenant("shop-b").await,
        tenant("stranger").await,
    );
    let write = parent.database().begin_write().await.unwrap();
    let root = OrgTree::insert_root(&write, parent.id()).await.unwrap();
    OrgTree::insert(&write, parent.id(), north, root, 8).await.unwrap();
    OrgTree::insert(&write, parent.id(), south, root, 8).await.unwrap();
    write.commit().await.unwrap();
    sales.sell(&parent, Some(north), vnd(100)).await;
    sales.sell(&parent, Some(south), vnd(200)).await;
    sales.sell(&parent, None, vnd(1_000)).await;
    sales.sell(&shop_a, None, vnd(10_000)).await;
    sales.sell(&shop_b, None, vnd(20_000)).await;
    sales.sell(&stranger, None, vnd(40_000)).await;
    settle(&app, &service(&app)).await;
    Fixture { app, parent, shop_a, shop_b, stranger, north, south }
}

/// `(status, tổng tiền VND)` của `sales.amount` mà `token` thấy với tham số thêm `extra`.
async fn summary(fixture: &Fixture, token: &str, extra: &str) -> (u16, i64) {
    let response = fixture
        .app
        .get(&format!("/api/v1/reporting/summary?kind=sales.amount{extra}"))
        .token(token)
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    if status != 200 {
        return (status, 0);
    }
    (status, amount(&response.json()["data"]))
}

fn amount(data: &Value) -> i64 {
    data["totals"]["by_currency"]
        .as_array()
        .unwrap()
        .iter()
        .map(|total| total["amount"].as_str().unwrap().parse::<i64>().unwrap())
        .sum()
}

#[tokio::test]
async fn parent_sees_whole_tree_and_child_only_itself() {
    TestDatabase::run(async |test| {
        let fixture = fixture(test).await;
        // Cha (độ sâu `tenant`): mình + hai con, kể cả con ở database khác; không có khách lạ.
        assert_eq!(summary(&fixture, "boss", "").await, (200, 31_300));
        // child_sees_only_own_subtree: con không thấy số liệu của cha hay của anh em.
        assert_eq!(summary(&fixture, "shop-a", "").await, (200, 10_000));
        assert_eq!(summary(&fixture, "stranger", "").await, (200, 40_000));
        let (status, _) = summary(&fixture, "nobody", "").await;
        assert_eq!(status, 403);
        // Nhóm theo khách: mỗi khách nguồn một nhóm.
        let response = fixture
            .app
            .get("/api/v1/reporting/summary?kind=sales.amount&group_by=tenant")
            .token("boss")
            .send()
            .await
            .unwrap();
        let keys: Vec<String> = response.json()["data"]["groups"]
            .as_array()
            .unwrap()
            .iter()
            .map(|group| group["key"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(keys.len(), 3);
        assert!(keys.contains(&fixture.shop_b.id().to_string()));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn sibling_tenant_cannot_see_each_other() {
    TestDatabase::run(async |test| {
        let fixture = fixture(test).await;
        let other = format!("&tenant_ids={}", fixture.shop_b.id());
        assert_eq!(summary(&fixture, "shop-a", &other).await, (404, 0));
        let parent = format!("&tenant_ids={}", fixture.parent.id());
        assert_eq!(summary(&fixture, "shop-a", &parent).await, (404, 0));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn other_tree_not_found() {
    TestDatabase::run(async |test| {
        let fixture = fixture(test).await;
        let stranger = format!("&tenant_ids={}", fixture.stranger.id());
        assert_eq!(summary(&fixture, "boss", &stranger).await, (404, 0));
        // Một khách không tồn tại trả cùng câu trả lời: không lộ khách nào có thật.
        let made_up = format!("&tenant_ids={}", Uuid::now_v7());
        assert_eq!(summary(&fixture, "boss", &made_up).await, (404, 0));
        let response = fixture
            .app
            .get(&format!("/api/v1/reporting/facts?tenant_ids={}", fixture.stranger.id()))
            .token("boss")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 404);
        assert_eq!(response.json()["code"], "reporting.tenant_not_found");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn subtree_scope_only_sees_own_branch() {
    TestDatabase::run(async |test| {
        let fixture = fixture(test).await;
        // Nhánh `north`: chỉ 100 của nhánh, không thấy nút gốc, nhánh khác hay khách con.
        assert_eq!(summary(&fixture, "north", "").await, (200, 100));
        assert_eq!(summary(&fixture, "unit", "").await, (200, 200));
        let response = fixture
            .app
            .get("/api/v1/reporting/facts?kind=sales.amount")
            .token("north")
            .send()
            .await
            .unwrap();
        let rows = response.json()["data"].as_array().unwrap().clone();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["org_id"], fixture.north.to_string());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn client_filter_cannot_widen_scope() {
    TestDatabase::run(async |test| {
        let fixture = fixture(test).await;
        // Lọc nút ngoài phạm vi: giao với phạm vi → rỗng, không mở rộng.
        let south = format!("&org_ids={}", fixture.south);
        assert_eq!(summary(&fixture, "north", &south).await, (200, 0));
        let root = format!("&org_ids={},{}", root_of(fixture.parent.id()), fixture.north);
        assert_eq!(summary(&fixture, "north", &root).await, (200, 100));
        let child = format!("&tenant_ids={}", fixture.shop_a.id());
        assert_eq!(summary(&fixture, "north", &child).await, (404, 0));
        // Người độ sâu `tenant` thu hẹp được về một con.
        assert_eq!(summary(&fixture, "boss", &child).await, (200, 10_000));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn mcp_tools_same_visibility_as_rest() {
    TestDatabase::run(async |test| {
        let fixture = fixture(test).await;
        for (token, expected) in [("boss", 31_300), ("north", 100), ("shop-a", 10_000)] {
            let client = fixture.app.mcp(token).await.unwrap();
            let answer = client
                .call_tool(tool("reporting.summary", json!({"kind": "sales.amount"})))
                .await
                .unwrap();
            assert_eq!(answer.is_error, Some(false), "{answer:?}");
            let data = answer.structured_content.unwrap();
            assert_eq!(amount(&data["data"]), expected, "{token}");
            assert_eq!(summary(&fixture, token, "").await, (200, expected));
            let facts = client
                .call_tool(tool("reporting.facts", json!({"kind": "sales.amount"})))
                .await
                .unwrap()
                .structured_content
                .unwrap();
            let sum: i64 = facts["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["amount"]["amount"].as_str().unwrap().parse::<i64>().unwrap())
                .sum();
            assert_eq!(sum, expected, "{token}");
            let stranger =
                json!({"kind": "sales.amount", "tenant_ids": fixture.stranger.id().to_string()});
            let refused = client.call_tool(tool("reporting.summary", stranger)).await.unwrap();
            assert_eq!(refused.is_error, Some(true));
            client.cancel().await.unwrap();
        }
    })
    .await
    .unwrap();
}

/// Bước 70 P9 (D-20261125-list-pages): độ trễ là một trang mỗi khách trong cây (theo id khách), loại số liệu một
/// trang theo tên; theo `next_cursor` tới hết không trùng không sót, hai chiều; tool `reporting.kinds` cùng tham
/// số trả đúng trang REST trả.
#[tokio::test]
async fn do_tre_va_loai_so_lieu_phan_trang() {
    TestDatabase::run(async |test| {
        let fixture = fixture(test).await;
        let walk = async |first: &str, field: &str| {
            let mut seen = Vec::new();
            let mut uri = first.to_owned();
            loop {
                let response = fixture.app.get(&uri).token("manager").send().await.unwrap();
                assert_eq!(response.status().as_u16(), 200, "{}", response.json());
                let page = response.json();
                let rows = page["data"].as_array().unwrap();
                assert!(rows.len() <= 1, "{page}");
                seen.extend(rows.iter().map(|row| row[field].as_str().unwrap().to_owned()));
                match page["pagination"]["next_cursor"].as_str() {
                    Some(cursor) => uri = format!("{first}&cursor={cursor}"),
                    None => return seen,
                }
            }
        };
        let mut tenants: Vec<String> = [&fixture.parent, &fixture.shop_a, &fixture.shop_b]
            .iter()
            .map(|tenant| tenant.id().as_uuid().to_string())
            .collect();
        tenants.sort();
        assert_eq!(walk("/api/v1/reporting/lag?limit=1", "tenant_id").await, tenants);
        tenants.reverse();
        assert_eq!(walk("/api/v1/reporting/lag?limit=1&order=desc", "tenant_id").await, tenants);
        let kinds = walk("/api/v1/reporting/kinds?limit=1", "kind").await;
        assert_eq!(kinds, ["sales.amount", "sales.count"]);

        let rest = fixture.app.get("/api/v1/reporting/kinds?limit=1&order=desc").token("manager");
        let rest = rest.send().await.unwrap().json();
        let client = fixture.app.mcp("manager").await.unwrap();
        let page = client
            .call_tool(tool("reporting.kinds", json!({"limit": 1, "order": "desc"})))
            .await
            .unwrap();
        let page = page.structured_content.unwrap();
        assert_eq!(page["data"], rest["data"]);
        assert_eq!(page["pagination"], rest["pagination"]);
        let cursor = page["pagination"]["next_cursor"].clone();
        let next = client
            .call_tool(tool(
                "reporting.kinds",
                json!({"limit": 1, "order": "desc", "cursor": cursor}),
            ))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(next["data"][0]["kind"], "sales.amount");
        client.cancel().await.unwrap();
    })
    .await
    .unwrap();
}
