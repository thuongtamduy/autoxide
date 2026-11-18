//! Phạm vi theo nút (bước 34) và phân công: nhánh khác không thấy phiếu, người bán trên sàn chỉ
//! thấy gian hàng mình, chia vòng bỏ qua người không có quyền, hai người cùng giao một phiếu thì
//! đúng một bên thắng.

use std::collections::HashMap;
use std::sync::Arc;

use autoxide_kernel::access::{Authenticator, Grant, Principal, Scope};
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{
    DEFAULT_TENANT, OrgId, OrgTree, TenantContext, TenantHint, root_of,
};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Harness, call, harness_with};

/// Bộ xác thực giả cho `principal_of`: người dùng của test theo ID.
pub(crate) struct Directory(pub HashMap<Uuid, Principal>);

#[async_trait::async_trait]
impl Authenticator for Directory {
    async fn authenticate(&self, _: Option<&str>, _: &TenantHint) -> Result<Principal, Error> {
        Err(Error::unauthenticated("auth.invalid_token", "The token is not valid."))
    }

    async fn principal_of(
        &self,
        _tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<Option<Principal>, Error> {
        Ok(self.0.get(&user_id).cloned())
    }
}

struct Tree {
    a: OrgId,
    b: OrgId,
    a1: OrgId,
}

const READ: &str = "support.tickets.read";
const REPLY: &str = "support.tickets.reply";
const SETTINGS: &str = "support.settings.manage";

fn staff(user: u128, org: OrgId, scope: Scope, permissions: &[&str]) -> Principal {
    Principal::with_grants(
        Uuid::from_u128(user),
        permissions.iter().map(|name| ((*name).to_owned(), Grant::new(org, scope))),
    )
}

async fn tree_app(test: &TestDatabase) -> (Harness, Tree) {
    let org = || OrgId::from_uuid(Uuid::now_v7());
    let tree = Tree { a: org(), b: org(), a1: org() };
    let root = root_of(DEFAULT_TENANT);
    let people = [
        ("north", staff(0x10, tree.a, Scope::Subtree, &[READ, REPLY])),
        ("south", staff(0x20, tree.b, Scope::Subtree, &[READ, REPLY])),
        ("seller", staff(0x30, tree.b, Scope::Unit, &[READ, REPLY])),
        ("boss", staff(0x40, root, Scope::Tenant, &[READ, REPLY, SETTINGS])),
        ("ann", staff(0x51, tree.a, Scope::Subtree, &[READ, REPLY])),
        ("ben", staff(0x52, tree.a, Scope::Subtree, &[READ, REPLY])),
        // Ở nhóm nhưng chỉ đọc: không bao giờ được giao.
        ("cat", staff(0x53, tree.a, Scope::Subtree, &[READ])),
    ];
    let directory: HashMap<Uuid, Principal> =
        people.iter().map(|(_, principal)| (principal.user_id(), principal.clone())).collect();
    let harness = harness_with(test, move |mut builder| {
        builder = builder.provide::<dyn Authenticator>(Arc::new(Directory(directory)));
        for (token, principal) in people {
            builder = builder.principal(token, principal);
        }
        builder
    })
    .await;
    let write = harness.app.database().begin_write().await.unwrap();
    let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, tree.a, root, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, tree.b, root, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, tree.a1, tree.a, 8).await.unwrap();
    write.commit().await.unwrap();
    (harness, tree)
}

async fn open_at(app: &TestApp, org: OrgId, subject: &str) -> String {
    let body =
        json!({"org_id": org.as_uuid(), "category": "order", "subject": subject, "body": "?"});
    let (status, opened) = call(app, "POST", "/public/tickets", "buyer", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{opened}");
    opened["data"]["id"].as_str().unwrap().to_owned()
}

fn subjects(list: &Value) -> Vec<String> {
    let mut out: Vec<String> = list["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|ticket| ticket["subject"].as_str().unwrap().to_owned())
        .collect();
    out.sort();
    out
}

#[tokio::test]
async fn nhan_vien_nhanh_khac_khong_thay_phieu() {
    TestDatabase::run(async |test| {
        let (harness, tree) = tree_app(test).await;
        let app = &harness.app;
        let in_a1 = open_at(app, tree.a1, "a1").await;
        let in_b = open_at(app, tree.b, "b").await;

        let (_, north) = call(app, "GET", "/tickets", "north", None).await;
        assert_eq!(subjects(&north), ["a1"]);
        let (_, boss) = call(app, "GET", "/tickets", "boss", None).await;
        assert_eq!(subjects(&boss), ["a1", "b"]);
        let (status, _) = call(app, "GET", &format!("/tickets/{in_b}"), "north", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = call(app, "GET", &format!("/tickets/{in_a1}"), "north", None).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = call(
            app,
            "POST",
            &format!("/tickets/{in_b}/messages"),
            "north",
            Some(json!({"body": "Xin chào"})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nguoi_ban_tren_san_chi_thay_phieu_gian_hang_minh() {
    TestDatabase::run(async |test| {
        let (harness, tree) = tree_app(test).await;
        let app = &harness.app;
        open_at(app, tree.a, "shop-a").await;
        let mine = open_at(app, tree.b, "shop-b").await;

        let (_, seller) = call(app, "GET", "/tickets", "seller", None).await;
        assert_eq!(subjects(&seller), ["shop-b"]);
        let (status, _) = call(
            app,
            "PATCH",
            &format!("/tickets/{mine}"),
            "seller",
            Some(json!({"status": "solved"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        // Người bán không tạo phiếu ở gian hàng khác.
        let body = json!({"org_id": tree.a.as_uuid(), "requester": {"email": "k@example.com"},
                          "category": "order", "subject": "x", "body": "y"});
        let (status, problem) = call(app, "POST", "/tickets", "seller", Some(body)).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("org_out_of_scope"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chia_vong_khong_giao_cho_nguoi_khong_co_quyen() {
    TestDatabase::run(async |test| {
        let (harness, tree) = tree_app(test).await;
        let app = &harness.app;
        let (status, team) = call(
            app,
            "POST",
            "/teams",
            "boss",
            Some(json!({"org_id": tree.a.as_uuid(), "name": "Đơn hàng"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{team}");
        let team = team["data"]["id"].as_str().unwrap().to_owned();
        for user in [0x51u128, 0x52] {
            let path = format!("/teams/{team}/members/{}", Uuid::from_u128(user));
            let (status, body) =
                call(app, "PUT", &path, "boss", Some(json!({"accepting": true}))).await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
        // `cat` chỉ đọc: không thêm được vào nhóm.
        let path = format!("/teams/{team}/members/{}", Uuid::from_u128(0x53));
        let (status, problem) =
            call(app, "PUT", &path, "boss", Some(json!({"accepting": true}))).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("support.assignee_out_of_scope"))
        );
        // Quy tắc: phiếu loại `order` của nhánh A → nhóm.
        let rule = json!({"org_id": tree.a.as_uuid(), "category": "order", "team_id": team});
        let (status, body) = call(app, "POST", "/routing-rules", "boss", Some(rule)).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");

        let mut assignees = Vec::new();
        for n in 0..4 {
            let id = open_at(app, tree.a1, &format!("t{n}")).await;
            let (_, got) = call(app, "GET", &format!("/tickets/{id}"), "boss", None).await;
            assert_eq!(got["data"]["team_id"], team.as_str());
            assignees.push(got["data"]["assignee_user_id"].as_str().unwrap().to_owned());
        }
        let (ann, ben) = (Uuid::from_u128(0x51).to_string(), Uuid::from_u128(0x52).to_string());
        assert_eq!(assignees, [ann.clone(), ben.clone(), ann.clone(), ben], "round robin");

        // Thành viên tự tắt "nhận việc": không được giao nữa.
        let path = format!("/teams/{team}/members/{ann}");
        let (status, _) = call(app, "PUT", &path, "ann", Some(json!({"accepting": false}))).await;
        assert_eq!(status, StatusCode::OK);
        let id = open_at(app, tree.a, "t5").await;
        let (_, got) = call(app, "GET", &format!("/tickets/{id}"), "boss", None).await;
        assert_eq!(got["data"]["assignee_user_id"], Uuid::from_u128(0x52).to_string());
        // Phiếu nhánh B không khớp quy tắc của A.
        let id = open_at(app, tree.b, "b").await;
        let (_, got) = call(app, "GET", &format!("/tickets/{id}"), "boss", None).await;
        assert!(got["data"]["team_id"].is_null());
        // Giao bằng tay cho người chỉ đọc: từ chối.
        let (status, _) = call(
            app,
            "PATCH",
            &format!("/tickets/{id}"),
            "boss",
            Some(json!({"assignee_user_id": Uuid::from_u128(0x53)})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, _) = call(
            app,
            "PATCH",
            &format!("/tickets/{id}"),
            "boss",
            Some(json!({"assignee_user_id": Uuid::from_u128(0x20)})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "south works in B");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn phan_cong_dong_thoi_mot_ben_thang() {
    TestDatabase::run(async |test| {
        let (harness, tree) = tree_app(test).await;
        let app = &harness.app;
        let id = open_at(app, tree.a, "race").await;
        let read =
            app.get(&format!("/api/v1/support/tickets/{id}")).token("boss").send().await.unwrap();
        let etag = read.header("etag").to_owned();
        let send = |who: u128| {
            let etag = etag.clone();
            let id = id.clone();
            async move {
                app.patch(&format!("/api/v1/support/tickets/{id}"))
                    .token("boss")
                    .header("if-match", &etag)
                    .json(json!({"assignee_user_id": Uuid::from_u128(who)}))
                    .send()
                    .await
                    .unwrap()
                    .status()
            }
        };
        let (one, two) = tokio::join!(send(0x51), send(0x52));
        let mut statuses = [one, two];
        statuses.sort();
        assert_eq!(statuses, [StatusCode::OK, StatusCode::PRECONDITION_FAILED]);
    })
    .await
    .unwrap();
}
