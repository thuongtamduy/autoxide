//! Nhóm nút ngoài cây trong báo cáo gộp (bước 38, D-232), qua REST và MCP, trên cả SQLite và
//! Postgres: nhóm không cấp quyền — luôn giao với phạm vi của người xem.

use std::collections::HashMap;
use std::sync::Arc;

use autoxide_kernel::access::{Grant, Principal, Scope};
use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::{OrgId, OrgTree, TenantContext};
use autoxide_organizations::api::OrgGroups;
use autoxide_testkit::{TestApp, TestDatabase, tool};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::test_support::{Sales, service, settle};

/// `OrgGroups` giả (như `organizations` công bố): nhóm → nút, cho mọi khách.
#[derive(Clone, Default)]
struct FakeGroups(Arc<HashMap<Uuid, Vec<OrgId>>>);

#[async_trait::async_trait]
impl OrgGroups for FakeGroups {
    async fn members(
        &self,
        _tenant: &TenantContext,
        group: Uuid,
    ) -> Result<Option<Vec<OrgId>>, Error> {
        Ok(self.0.get(&group).cloned())
    }
}

impl Module for FakeGroups {
    fn name(&self) -> &'static str {
        "fakegroups"
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.provide::<dyn OrgGroups>(Arc::new(self.clone()))
    }
}

struct Fixture {
    app: TestApp,
    /// Nhóm có cả `north` và `south`.
    both: Uuid,
    /// Nhóm chỉ có `north`.
    only_north: Uuid,
}

/// Cha có hai nhánh `north` (100) và `south` (200) cùng 1 000 ở nút gốc; con `shop-a` bán 10 000.
async fn fixture(test: &TestDatabase) -> Fixture {
    let sales = Sales::default();
    let (north, south) = (OrgId::from_uuid(Uuid::now_v7()), OrgId::from_uuid(Uuid::now_v7()));
    let (both, only_north) = (Uuid::now_v7(), Uuid::now_v7());
    let groups = FakeGroups(Arc::new(HashMap::from([
        (both, vec![north, south]),
        (only_north, vec![north]),
    ])));
    let read = |user: u128, grant: Option<(OrgId, Scope)>| match grant {
        None => Principal::new(Uuid::from_u128(user), ["reporting.read".to_owned()]),
        Some((org, scope)) => Principal::with_grants(
            Uuid::from_u128(user),
            [("reporting.read".to_owned(), Grant::new(org, scope))],
        ),
    };
    let app = TestApp::builder(test)
        .tenant("parent")
        .child_tenant("shop-a", "parent", true)
        .module(sales.clone())
        .module(groups)
        .module(crate::module())
        .principal_in("parent", "boss", read(1, None))
        .principal_in("parent", "north", read(2, Some((north, Scope::Subtree))))
        .build()
        .await
        .unwrap();
    let parent = app.tenant("parent").await.unwrap();
    let shop_a = app.tenant("shop-a").await.unwrap();
    let write = parent.database().begin_write().await.unwrap();
    let root = OrgTree::insert_root(&write, parent.id()).await.unwrap();
    OrgTree::insert(&write, parent.id(), north, root, 8).await.unwrap();
    OrgTree::insert(&write, parent.id(), south, root, 8).await.unwrap();
    write.commit().await.unwrap();
    let vnd = |minor| Money::new(minor, Currency::VND);
    sales.sell(&parent, Some(north), vnd(100)).await;
    sales.sell(&parent, Some(south), vnd(200)).await;
    sales.sell(&parent, None, vnd(1_000)).await;
    sales.sell(&shop_a, None, vnd(10_000)).await;
    settle(&app, &service(&app)).await;
    Fixture { app, both, only_north }
}

fn amount(group: &Value) -> i64 {
    group["by_currency"]
        .as_array()
        .unwrap()
        .iter()
        .map(|total| total["amount"].as_str().unwrap().parse::<i64>().unwrap())
        .sum()
}

async fn summary(fixture: &Fixture, token: &str, extra: &str) -> (u16, Value) {
    let response = fixture
        .app
        .get(&format!("/api/v1/reporting/summary?kind=sales.amount{extra}"))
        .token(token)
        .send()
        .await
        .unwrap();
    (response.status().as_u16(), response.json())
}

#[tokio::test]
async fn nhom_gom_nut_hai_nhanh_nguoi_mot_nhanh_chi_thay_phan_minh() {
    TestDatabase::run(async |test| {
        let fixture = fixture(test).await;
        let both = format!("&group_ids={}", fixture.both);
        // Người có cả khách: 100 + 200, không có nút gốc hay khách con; không bỏ nút nào.
        let (status, body) = summary(&fixture, "boss", &both).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(amount(&body["data"]["totals"]), 300);
        assert_eq!(body["data"]["totals"]["excluded_org_count"], 0);
        // Người chỉ có nhánh north: 100; nút south bị bỏ, chỉ lộ thành số đếm.
        let (_, body) = summary(&fixture, "north", &both).await;
        assert_eq!(amount(&body["data"]["totals"]), 100, "{body}");
        assert_eq!(body["data"]["totals"]["excluded_org_count"], 1);
        // Không hỏi nhóm thì không có trường này.
        let (_, body) = summary(&fixture, "north", "").await;
        assert!(body["data"]["totals"].get("excluded_org_count").is_none());

        // MCP: cùng câu trả lời.
        let client = fixture.app.mcp("north").await.unwrap();
        let input = json!({"kind": "sales.amount", "group_ids": fixture.both.to_string()});
        let answer = client.call_tool(tool("reporting.summary", input)).await.unwrap();
        let data = answer.structured_content.unwrap();
        assert_eq!(amount(&data["data"]["totals"]), 100, "{data}");
        assert_eq!(data["data"]["totals"]["excluded_org_count"], 1);
        let input = json!({"kind": "sales.amount", "group_ids": fixture.both.to_string()});
        let facts = client.call_tool(tool("reporting.facts", input)).await.unwrap();
        let rows = facts.structured_content.unwrap()["data"].as_array().unwrap().len();
        assert_eq!(rows, 1, "chỉ số liệu của nhánh north");
        client.cancel().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn group_by_group_tinh_nut_o_hai_nhom() {
    TestDatabase::run(async |test| {
        let fixture = fixture(test).await;
        let extra = format!("&group_by=group&group_ids={},{}", fixture.both, fixture.only_north);
        let (status, body) = summary(&fixture, "boss", &extra).await;
        assert_eq!(status, 200, "{body}");
        let groups = body["data"]["groups"].as_array().unwrap();
        let of =
            |id: Uuid| groups.iter().find(|group| group["key"] == id.to_string()).unwrap().clone();
        assert_eq!(amount(&of(fixture.both)), 300);
        assert_eq!(amount(&of(fixture.only_north)), 100);
        // Nút north ở hai nhóm: tổng các nhóm (400) lớn hơn tổng chung (300).
        assert_eq!(amount(&body["data"]["totals"]), 300);

        // Người một nhánh: mỗi nhóm có số nút bị bỏ của riêng nó.
        let (_, body) = summary(&fixture, "north", &extra).await;
        let groups = body["data"]["groups"].as_array().unwrap();
        let both = groups.iter().find(|group| group["key"] == fixture.both.to_string()).unwrap();
        assert_eq!((amount(both), both["excluded_org_count"].as_u64()), (100, Some(1)));
        let north =
            groups.iter().find(|group| group["key"] == fixture.only_north.to_string()).unwrap();
        assert_eq!((amount(north), north["excluded_org_count"].as_u64()), (100, Some(0)));

        // `group_by=group` cần `group_ids`.
        let (status, body) = summary(&fixture, "boss", "&group_by=group").await;
        assert_eq!(status, 422, "{body}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nhom_khong_co_tra_404() {
    TestDatabase::run(async |test| {
        let fixture = fixture(test).await;
        let (status, body) =
            summary(&fixture, "boss", &format!("&group_ids={}", Uuid::now_v7())).await;
        assert_eq!((status, body["code"].as_str()), (404, Some("reporting.group_not_found")));
    })
    .await
    .unwrap();
}
