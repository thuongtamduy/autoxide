//! Test qua HTTP (router thật của kernel) và MCP (client thật): 201 + `Location`, `If-Match` bắt
//! buộc, `ETag` + 304 + `Cache-Control` ở menu công khai, permission, tool dùng chung service.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::usages::{ReferenceSource, Referrer};
use autoxide_media::api::Media;
use autoxide_testkit::{TestApp, TestDatabase, tool};
use axum::http::{Method, StatusCode, header};
use serde_json::{Value, json};

use super::{Categories, Items, fake_media};

/// Module giả cung cấp `shop.item` và `shop.category` cho sổ đối tượng.
struct Shop;

// `#[cfg(test)]` (file đã chỉ có trong test): bộ kiểm bản dịch bỏ qua câu mô tả permission giả.
#[cfg(test)]
impl Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn permissions(&self) -> Vec<autoxide_kernel::access::Permission> {
        vec![autoxide_kernel::access::Permission::new("shop.read", "See the shop.")]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.resource_type("shop.item", Arc::new(Items))?;
        context.reference_source(Arc::new(ShopReferences));
        context.resource_type("shop.category", Arc::new(Categories))
    }
}

/// Nguồn "đang dùng ở đâu" giả của `shop`, chỉ cài `referrers` (không ghi đè `referrers_page`): kiểm cài đặt
/// mặc định của kernel gộp được với trang keyset của `menus`. Chỉ trả cho `shop.item` / `p-many`.
struct ShopReferences;

#[async_trait::async_trait]
impl ReferenceSource for ShopReferences {
    async fn referrers(
        &self,
        _tenant: &TenantContext,
        kind: &str,
        id: &str,
        limit: usize,
    ) -> Result<Vec<Referrer>, Error> {
        if (kind, id) != ("shop.item", "p-many") {
            return Ok(Vec::new());
        }
        let mut rows: Vec<Referrer> = (1..=3)
            .map(|n| Referrer::new("shop.bundle", format!("b{n}"), format!("Bundle {n}"), "items"))
            .collect();
        rows.truncate(limit);
        Ok(rows)
    }
}

async fn app(test: &TestDatabase) -> TestApp {
    let media: Arc<dyn Media> = Arc::new(fake_media());
    TestApp::builder(test)
        .user("admin", 1, &["menus.read", "menus.manage"])
        .user("reader", 2, &["menus.read"])
        .provide::<dyn Media>(media)
        .module(Shop)
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

/// Gửi request tới `/api/v1/menus{uri}`; `who`: token (`None`: không đăng nhập).
async fn send(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: Option<&str>,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> autoxide_testkit::TestResponse {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1/menus{uri}"));
    if let Some(who) = who {
        request = request.token(who);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    for (name, value) in headers {
        request = request.header(name, value);
    }
    request.send().await.unwrap()
}

#[tokio::test]
async fn vong_doi_qua_rest() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let body = json!({
            "key": "main",
            "locale": "vi",
            "items": [
                {"ref": {"kind": "shop.category", "id": "c"}, "auto_children": true, "auto_children_limit": 2, "icon": "category"},
                {"label": "Khuyến mại", "url": "/sale", "highlight": "sale", "badge": {"text": "-50%", "tone": "sale"}},
                {"ref": {"kind": "shop.item", "id": "h1"}}
            ]
        });
        let denied = send(&app, "POST", "", Some("reader"), &[], Some(body.clone())).await;
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        let created = send(&app, "POST", "", Some("admin"), &[], Some(body.clone())).await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(created.header(header::LOCATION), format!("/api/v1/menus/{id}"));
        let again = send(&app, "POST", "", Some("admin"), &[], Some(body)).await;
        assert_eq!((again.status(), again.json()["code"].as_str()), (StatusCode::CONFLICT, Some("menus.menu_exists")));

        let bad = json!({"key": "x", "items": [{"ref": {"kind": "shop.item", "id": "x1"}}]});
        let bad = send(&app, "POST", "", Some("admin"), &[("accept-language", "vi")], Some(bad)).await;
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(bad.json()["code"], "menus.ref_not_found");
        assert_eq!(bad.json()["errors"][0]["field"], "items[0].ref.id");

        let read = send(&app, "GET", &format!("/{id}"), Some("reader"), &[], None).await;
        assert_eq!(read.status(), StatusCode::OK);
        assert_eq!(read.json()["data"]["items"][0]["ref"]["kind"], "shop.category");
        let etag = read.header(header::ETAG).to_owned();
        let listed = send(&app, "GET", "", Some("reader"), &[], None).await;
        assert_eq!(listed.json()["data"][0]["key"], "main");

        let public = send(&app, "GET", "/public/main?locale=vi", None, &[], None).await;
        assert_eq!(public.status(), StatusCode::OK, "{}", public.json());
        let shown = public.json();
        assert_eq!(shown["data"]["items"].as_array().unwrap().len(), 2, "hidden item left out");
        assert_eq!(shown["data"]["items"][0]["label"], "Category c");
        assert_eq!(shown["data"]["items"][0]["children"].as_array().unwrap().len(), 2);
        assert_eq!(shown["data"]["items"][1]["badge"]["text"], "-50%");
        assert_eq!(public.header(header::CACHE_CONTROL), "public, max-age=60");
        let tag = public.header(header::ETAG).to_owned();
        let cached = send(&app, "GET", "/public/main?locale=vi", None, &[("if-none-match", &tag)], None).await;
        assert_eq!(cached.status(), StatusCode::NOT_MODIFIED);
        // Khối `menus.menu` (bước 67) trả đúng menu công khai; khoá không có → `null`.
        let tenant = app.default_tenant();
        let site = autoxide_kernel::tenancy::Site::whole(&tenant);
        let request = autoxide_kernel::blocks::BlockRequest::new(&tenant, &site, "vi");
        let block = |id: &str, key: &str| ("menus.menu", autoxide_kernel::blocks::BlockRef::new(id, json!({"key": key})));
        let data = app.blocks().resolve(&request, &[block("a", "main"), block("b", "main"), block("c", "nope")]).await;
        assert_eq!(data["a"], shown["data"], "{data:?}");
        assert_eq!(data["b"], data["a"]);
        assert_eq!(data["c"], serde_json::Value::Null);

        let items = json!({"items": [{"label": "Trang chủ", "url": "/"}]});
        let missing = send(&app, "PUT", &format!("/{id}"), Some("admin"), &[], Some(items.clone())).await;
        assert_eq!(missing.status(), StatusCode::PRECONDITION_REQUIRED);
        let replaced = send(&app, "PUT", &format!("/{id}"), Some("admin"), &[("if-match", &etag)], Some(items.clone())).await;
        assert_eq!(replaced.status(), StatusCode::OK, "{}", replaced.json());
        let stale = send(&app, "PUT", &format!("/{id}"), Some("admin"), &[("if-match", &etag)], Some(items)).await;
        assert_eq!(stale.status(), StatusCode::PRECONDITION_FAILED);
        let fresh = send(&app, "GET", "/public/main?locale=vi", None, &[("if-none-match", &tag)], None).await;
        assert_eq!(fresh.status(), StatusCode::OK, "a change gives a new ETag");
        assert_eq!(fresh.json()["data"]["items"][0]["label"], "Trang chủ");

        let gone = send(&app, "DELETE", &format!("/{id}"), Some("admin"), &[], None).await;
        assert_eq!(gone.status(), StatusCode::NO_CONTENT);
        let missing = send(&app, "GET", "/public/main", None, &[], None).await;
        assert_eq!((missing.status(), missing.json()["code"].as_str()), (StatusCode::NOT_FOUND, Some("menus.public_menu_not_found")));
    })
    .await
    .unwrap();
}

/// Tool MCP gọi cùng service với REST: tạo qua MCP đọc được qua REST; `version` cũ bị từ chối.
#[tokio::test]
async fn tool_mcp_dung_chung_service() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let admin = app.mcp("admin").await.unwrap();
        let made = admin
            .call_tool(tool(
                "menus.create_menu",
                json!({"key": "footer", "items": [{"label": "Liên hệ", "url": "mailto:a@b.vn"}]}),
            ))
            .await
            .unwrap();
        let made = made.structured_content.unwrap();
        let id = made["data"]["id"].as_str().unwrap().to_owned();
        let version = made["data"]["version"].as_str().unwrap().to_owned();
        let read = send(&app, "GET", &format!("/{id}"), Some("admin"), &[], None).await;
        assert_eq!(read.json()["data"]["items"][0]["url"], "mailto:a@b.vn");
        assert_eq!(read.header(header::ETAG), version);

        let input = json!({"id": id, "version": version, "items": [{"label": "Home", "url": "/"}]});
        let replaced =
            admin.call_tool(tool("menus.replace_menu_items", input.clone())).await.unwrap();
        assert_ne!(replaced.is_error, Some(true), "{replaced:?}");
        let stale = admin.call_tool(tool("menus.replace_menu_items", input)).await.unwrap();
        assert_eq!(stale.is_error, Some(true));
        assert!(format!("{stale:?}").contains("precondition_failed"), "{stale:?}");

        let reader = app.mcp("reader").await.unwrap();
        let listed = reader.call_tool(tool("menus.list_menus", json!({}))).await.unwrap();
        assert_eq!(listed.structured_content.unwrap()["data"][0]["id"], id.as_str());
        let tools = reader.list_all_tools().await.unwrap();
        assert!(tools.iter().all(|tool| !tool.name.ends_with("delete_menu")), "needs menus.manage");
    })
    .await
    .unwrap();
}

/// Con trỏ rác (chữ lạ, base64 của JSON sai, của chuỗi có NUL) qua HTTP ra 422 ở trường `cursor`,
/// không bao giờ 500 (đặc tả 31 B3).
#[tokio::test]
async fn con_tro_rac_qua_http_ra_422() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        // base64url của `{"c":"x","v":{"s":"\u0000"},"id":"…"}` và của `{}`.
        let nul = "eyJjIjoieCIsInYiOnsicyI6Ilx1MDAwMCJ9LCJpZCI6IjAxOTNlM2E0LTdiMWMtN2YwMC05YTUxLTZmMmI4YzBkNGUxMSJ9";
        for cursor in ["nope", "%F0%9F%98%80", "e30", nul] {
            let answer = send(&app, "GET", &format!("?cursor={cursor}"), Some("reader"), &[], None).await;
            assert_eq!(answer.status(), StatusCode::UNPROCESSABLE_ENTITY, "{cursor}");
            assert_eq!(answer.json()["errors"][0]["field"], "cursor", "{cursor}");
        }
        // NUL thô trong query bị lớp HTTP của kernel từ chối sớm hơn (400, D-157) — vẫn không 500.
        let answer = send(&app, "GET", "?cursor=%00", Some("reader"), &[], None).await;
        assert_eq!(answer.status(), StatusCode::BAD_REQUEST, "{}", answer.text());
    })
    .await
    .unwrap();
}

/// Đề bài giao diện A6 (D-264): `POST` trả `ETag` (sửa tiếp không cần đọc lại), `PUT` giữ id của
/// mục có `id`, danh sách có số mục và người sửa cuối; câu lỗi từng ô theo `Accept-Language`.
#[tokio::test]
async fn giu_id_muc_etag_khi_tao_va_so_muc_trong_danh_sach() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let body = json!({"key": "footer", "items": [
            {"label": "A", "url": "/a", "children": [{"label": "A1", "url": "/a1"}]},
            {"label": "B", "url": "/b"}
        ]});
        let created = send(&app, "POST", "", Some("admin"), &[], Some(body)).await;
        assert_eq!(created.status(), StatusCode::CREATED);
        let etag = created.header(header::ETAG).to_owned();
        assert!(etag.starts_with('"'), "{etag}");
        let menu = created.json()["data"].clone();
        let id = menu["id"].as_str().unwrap().to_owned();
        let a = menu["items"][0]["id"].clone();
        let a1 = menu["items"][0]["children"][0]["id"].clone();

        // Đổi thứ tự, sửa nhãn, thêm một mục mới: hai mục cũ giữ id.
        let items = json!({"items": [
            {"label": "B mới", "url": "/b"},
            {"id": a, "label": "A đổi tên", "url": "/a", "children": [{"id": a1, "label": "A1", "url": "/a1"}]}
        ]});
        let replaced =
            send(&app, "PUT", &format!("/{id}"), Some("admin"), &[("if-match", &etag)], Some(items)).await;
        assert_eq!(replaced.status(), StatusCode::OK, "{}", replaced.json());
        let saved = replaced.json()["data"].clone();
        assert_eq!(saved["items"][1]["id"], a);
        assert_eq!(saved["items"][1]["label"], "A đổi tên");
        assert_eq!(saved["items"][1]["children"][0]["id"], a1);
        assert_ne!(saved["items"][0]["id"], menu["items"][1]["id"], "B was sent without an id");

        // Id lạ, hay hai mục cùng id: lỗi đúng ô, dịch theo ngôn ngữ.
        let etag = replaced.header(header::ETAG).to_owned();
        let strange = json!({"items": [
            {"id": uuid::Uuid::now_v7(), "label": "X", "url": "/x"},
            {"id": a, "label": "A", "url": "/a"},
            {"id": a, "label": "A again", "url": "/a2"}
        ]});
        let headers = [("if-match", etag.as_str()), ("accept-language", "vi")];
        let refused = send(&app, "PUT", &format!("/{id}"), Some("admin"), &headers, Some(strange)).await;
        assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let errors = refused.json()["errors"].clone();
        let fields: Vec<&str> = errors.as_array().unwrap().iter().map(|e| e["field"].as_str().unwrap()).collect();
        assert!(fields.contains(&"items[2].id"), "{errors}");
        assert!(fields.contains(&"items[0].id"), "{errors}");
        let unknown = errors.as_array().unwrap().iter().find(|e| e["field"] == "items[0].id").unwrap();
        assert_eq!(unknown["detail"], "không phải mục của menu này; bỏ `id` với mục mới");
        // Tạo menu mới không nhận `id`.
        let with_id = json!({"key": "x", "items": [{"id": a, "label": "A", "url": "/a"}]});
        let refused = send(&app, "POST", "", Some("admin"), &[], Some(with_id)).await;
        assert_eq!(refused.json()["errors"][0]["field"], "items[0].id");

        let listed = send(&app, "GET", "", Some("reader"), &[], None).await;
        let row = listed.json()["data"][0].clone();
        assert_eq!(row["item_count"], 3);
        assert_eq!(row["updated_by"], uuid::Uuid::from_u128(1).to_string());
    })
    .await
    .unwrap();
}

/// Hai khách (một dùng chung database, một database riêng) cùng menu `main`: menu công khai trả
/// menu của đúng khách theo header `autoxide-tenant`, kể cả khi menu của khách kia đã nằm trong
/// cache; người của khách B không đọc được menu của khách A qua bất kỳ `GET` hay tool MCP chỉ đọc
/// nào (bước 33).
#[tokio::test]
async fn hai_khach_cung_menu_khong_ro_ri() {
    TestDatabase::run(async |test| {
        let media: Arc<dyn Media> = Arc::new(fake_media());
        let all = ["menus.read", "menus.manage"];
        let app = TestApp::builder(test)
            .tenant("a")
            .tenant_dedicated("b")
            .user_in("a", "admin-a", 1, &all)
            .user_in("b", "admin-b", 2, &all)
            .provide::<dyn Media>(media)
            .module(Shop)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let marker = "marker-menus-a";
        let mut ids = Vec::new();
        for (who, label) in [("admin-a", marker), ("admin-b", "Menu B")] {
            let body = json!({"key": "main", "items": [
                {"label": label, "url": "/home", "children": [{"label": format!("{label} child"), "url": "/child"}]}
            ]});
            let created = send(&app, "POST", "", Some(who), &[], Some(body)).await;
            assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
            ids.push(created.json()["data"]["id"].as_str().unwrap().to_owned());
        }
        let item = {
            let read = send(&app, "GET", &format!("/{}", ids[0]), Some("admin-a"), &[], None).await;
            read.json()["data"]["items"][0]["id"].as_str().unwrap().to_owned()
        };
        // Đọc hai lượt: lượt sau trả từ cache, vẫn đúng khách.
        let mut tags = Vec::new();
        for _ in 0..2 {
            for (slug, label) in [("a", marker), ("b", "Menu B")] {
                let public = send(&app, "GET", "/public/main", None, &[("autoxide-tenant", slug)], None).await;
                assert_eq!(public.status(), StatusCode::OK, "{}", public.json());
                assert_eq!(public.json()["data"]["items"][0]["label"], label);
                tags.push(public.header(header::ETAG).to_owned());
            }
        }
        assert_ne!(tags[0], tags[1]);
        // `ETag` của khách A gửi ở khách B không ra 304.
        let headers = [("autoxide-tenant", "b"), ("if-none-match", tags[0].as_str())];
        let other = send(&app, "GET", "/public/main", None, &headers, None).await;
        assert_eq!(other.status(), StatusCode::OK);
        let missing = send(&app, "GET", &format!("/{}", ids[0]), Some("admin-b"), &[], None).await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND, "{}", missing.json());

        let seeded = [("id", ids[0].clone()), ("id", item)];
        let violations = autoxide_testkit::leak_check(&app, "admin-b", marker, &seeded).await.unwrap();
        assert_eq!(violations, Vec::<String>::new());
    })
    .await
    .unwrap();
}

/// Menu theo website (bước 34, D-238): mỗi nút một menu `main` riêng; website chưa có menu riêng
/// dùng menu của website cấp trên; người quản lý menu của một nhánh không đụng được nhánh khác.
#[tokio::test]
async fn menu_theo_website_va_nhanh() {
    use autoxide_kernel::access::Scope;
    use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree};

    TestDatabase::run(async |test| {
        let org = || OrgId::from_uuid(uuid::Uuid::now_v7());
        let (a, b) = (org(), org());
        let media: Arc<dyn Media> = Arc::new(fake_media());
        let manage = [("menus.read", Scope::Subtree), ("menus.manage", Scope::Subtree)];
        let app = TestApp::builder(test)
            .user("admin", 1, &["menus.read", "menus.manage"])
            .member("shop-b", 2, b, &manage)
            .provide::<dyn Media>(media)
            .module(Shop)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let write = app.database().begin_write().await.unwrap();
        let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, a, root, 8).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, b, root, 8).await.unwrap();
        write.commit().await.unwrap();

        let menu = |label: &str, org_id: Option<OrgId>| {
            let mut body = json!({"key": "main", "items": [{"label": label, "url": "/x"}]});
            if let Some(org) = org_id {
                body["org_id"] = json!(org.as_uuid());
            }
            body
        };
        let made = send(&app, "POST", "", Some("admin"), &[], Some(menu("Whole", None))).await;
        assert_eq!(made.status(), StatusCode::CREATED, "{}", made.json());
        assert_eq!(made.json()["data"]["org_id"], root.as_uuid().to_string());
        // PT-8: quyền toàn khách không gắn được menu vào nút không có ở khách (ID bịa).
        let made =
            send(&app, "POST", "", Some("admin"), &[], Some(menu("Ghost", Some(org())))).await;
        assert_eq!(made.status(), StatusCode::FORBIDDEN, "{}", made.json());
        assert_eq!(made.json()["code"], "org_out_of_scope");
        let made = send(&app, "POST", "", Some("admin"), &[], Some(menu("Shop A", Some(a)))).await;
        assert_eq!(made.status(), StatusCode::CREATED, "{}", made.json());
        let menu_a = made.json()["data"]["id"].as_str().unwrap().to_owned();

        let public = async |site: Option<OrgId>| {
            let uri = match site {
                Some(site) => format!("/public/main?site={}", site.as_uuid()),
                None => "/public/main".to_owned(),
            };
            let answer = send(&app, "GET", &uri, None, &[], None).await;
            assert_eq!(answer.status(), StatusCode::OK, "{}", answer.json());
            answer.json()["data"]["items"][0]["label"].as_str().unwrap().to_owned()
        };
        assert_eq!(public(Some(a)).await, "Shop A", "website A có menu riêng");
        assert_eq!(public(Some(b)).await, "Whole", "website B dùng menu của cả khách");
        assert_eq!(public(None).await, "Whole");
        let unknown =
            send(&app, "GET", &format!("/public/main?site={}", org().as_uuid()), None, &[], None)
                .await;
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
        assert_eq!(unknown.json()["code"], "site_not_found");

        // Người quản lý menu của B: không thấy, không sửa được menu của A; tạo menu cho A bị 403.
        let listed = send(&app, "GET", "", Some("shop-b"), &[], None).await;
        assert_eq!(listed.json()["data"], json!([]), "chưa có menu nào ở nhánh B");
        let hidden = send(&app, "GET", &format!("/{menu_a}"), Some("shop-b"), &[], None).await;
        assert_eq!(hidden.status(), StatusCode::NOT_FOUND);
        let refused =
            send(&app, "POST", "", Some("shop-b"), &[], Some(menu("Mine", Some(a)))).await;
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
        assert_eq!(refused.json()["code"], "org_out_of_scope");
        let own = send(&app, "POST", "", Some("shop-b"), &[], Some(menu("Shop B", Some(b)))).await;
        assert_eq!(own.status(), StatusCode::CREATED, "{}", own.json());
        assert_eq!(public(Some(b)).await, "Shop B", "cache theo website: B thấy menu mới của mình");
        let by_owner =
            send(&app, "GET", &format!("?owner_org={}", a.as_uuid()), Some("admin"), &[], None)
                .await;
        assert_eq!(by_owner.json()["data"].as_array().unwrap().len(), 1);
    })
    .await
    .unwrap();
}

/// Đề bài giao diện (menu, điểm 3 và 9): đọc menu kèm tình trạng từng đích (`include=targets`);
/// nhập menu cũ qua REST — không có module `content` thì báo rõ, người chỉ đọc bị từ chối.
#[tokio::test]
async fn doc_menu_kem_tinh_trang_dich_va_nhap_menu_cu() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let body = json!({
            "key": "main",
            "locale": "vi",
            "items": [
                {"ref": {"kind": "shop.item", "id": "p1"}},
                {"ref": {"kind": "shop.item", "id": "h1"}, "label": "Ẩn"}
            ]
        });
        let created = send(&app, "POST", "", Some("admin"), &[], Some(body)).await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();

        let plain = send(&app, "GET", &format!("/{id}"), Some("reader"), &[], None).await;
        assert_eq!(plain.json()["data"]["items"][0]["ref"].get("state"), None, "chỉ khi xin");
        let read =
            send(&app, "GET", &format!("/{id}?include=targets"), Some("reader"), &[], None).await;
        assert_eq!(read.status(), StatusCode::OK, "{}", read.json());
        let items = &read.json()["data"]["items"];
        assert_eq!(items[0]["ref"]["state"], "public");
        assert_eq!(items[0]["ref"]["title"], "p1@vi");
        assert_eq!(items[0]["ref"]["path"], "/items/p1");
        assert_eq!(items[1]["ref"]["state"], "not_public");
        assert_eq!(
            items[1]["ref"].get("title"),
            None,
            "không lộ tiêu đề của đối tượng chưa công khai"
        );
        let bad = send(&app, "GET", &format!("/{id}?include=all"), Some("reader"), &[], None).await;
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let dry = json!({"dry_run": true});
        let denied =
            send(&app, "POST", "/import-content", Some("reader"), &[], Some(dry.clone())).await;
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        let missing = send(&app, "POST", "/import-content", Some("admin"), &[], Some(dry)).await;
        assert_eq!(
            (missing.status(), missing.json()["code"].as_str()),
            (StatusCode::BAD_REQUEST, Some("menus.import_source_missing"))
        );
    })
    .await
    .unwrap();
}

/// "Đang dùng ở đâu" (bước 51, D-267): menu có mục trỏ tới một bản ghi hiện ra ở bản ghi đó, chỉ
/// cho người đọc được menu.
#[tokio::test]
async fn menu_tro_toi_ban_ghi_hien_o_dang_dung_o_dau() {
    TestDatabase::run(async |test| {
        let media: Arc<dyn Media> = Arc::new(fake_media());
        let app = TestApp::builder(test)
            .user("admin", 1, &["menus.read", "menus.manage", "shop.read"])
            .user("seller", 2, &["shop.read"])
            .provide::<dyn Media>(media)
            .module(Shop)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let body = json!({"key": "main", "locale": "vi", "items": [
            {"label": "Ấm", "ref": {"kind": "shop.item", "id": "h1"}}
        ]});
        let created = send(&app, "POST", "", Some("admin"), &[], Some(body)).await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let menu = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let usages = |who: &'static str| {
            let app = &app;
            async move {
                app.get("/api/v1/kernel/records/shop.item/h1/usages")
                    .token(who)
                    .send()
                    .await
                    .unwrap()
                    .json()
            }
        };
        let found = usages("admin").await;
        assert_eq!(found["data"][0]["kind"], "menus.menu", "{found}");
        assert_eq!(found["data"][0]["id"], menu);
        assert_eq!(found["data"][0]["field"], "menu_item");
        assert_eq!(usages("seller").await["data"], json!([]), "không đọc được menu thì không thấy");
    })
    .await
    .unwrap();
}

/// "Đang dùng ở đâu" là `Page<T>` (bước 70 P9 mục 3): mục menu (trang keyset trong database) và nguồn giả của
/// `shop` (cài đặt mặc định của kernel) gộp với một con trỏ chung — đi hết bằng `next_cursor` ra đủ 3 + 3 dòng,
/// không trùng không sót, hai chiều ngược nhau; `count=true` cho tổng của cả hai nguồn.
#[tokio::test]
async fn dang_dung_o_dau_chia_trang_qua_hai_nguon() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .user("admin", 1, &["menus.read", "menus.manage", "shop.read"])
            .user("seller", 2, &["shop.read"])
            .provide::<dyn Media>(Arc::new(fake_media()) as Arc<dyn Media>)
            .module(Shop)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let body = json!({"key": "main", "locale": "vi", "items": [
            {"label": "Một", "ref": {"kind": "shop.item", "id": "p-many"}},
            {"label": "Hai", "ref": {"kind": "shop.item", "id": "p-many"}},
            {"label": "Khác", "ref": {"kind": "shop.item", "id": "p-other"}}
        ]});
        let created = send(&app, "POST", "", Some("admin"), &[], Some(body)).await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let body = json!({"key": "footer", "locale": "vi", "items": [
            {"label": "Ba", "ref": {"kind": "shop.item", "id": "p-many"}}
        ]});
        let created = send(&app, "POST", "", Some("admin"), &[], Some(body)).await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());

        let walk = |who: &'static str, order: &'static str| {
            let app = &app;
            async move {
                let mut seen = Vec::new();
                let mut cursor: Option<String> = None;
                let mut first = true;
                loop {
                    let mut uri = format!(
                        "/api/v1/kernel/records/shop.item/p-many/usages?limit=2&order={order}"
                    );
                    if first {
                        uri.push_str("&count=true");
                    }
                    if let Some(cursor) = &cursor {
                        uri.push_str(&format!("&cursor={cursor}"));
                    }
                    let page = app.get(&uri).token(who).send().await.unwrap();
                    assert_eq!(page.status(), StatusCode::OK, "{}", page.json());
                    let page = page.json();
                    if first {
                        let total = page["pagination"]["total"].clone();
                        seen.push(json!({"total": total}));
                        first = false;
                    }
                    let rows = page["data"].as_array().unwrap().clone();
                    assert!(rows.len() <= 2, "{page}");
                    for row in rows {
                        seen.push(json!(format!("{}|{}", row["kind"], row["title"])));
                    }
                    match page["pagination"]["next_cursor"].as_str() {
                        Some(next) => cursor = Some(next.to_owned()),
                        None => break,
                    }
                }
                seen
            }
        };
        let desc = walk("admin", "desc").await;
        assert_eq!(desc[0], json!({"total": 6}), "{desc:?}");
        let mut rows: Vec<String> =
            desc[1..].iter().map(|row| row.as_str().unwrap().to_owned()).collect();
        assert_eq!(rows.len(), 6, "{rows:?}");
        let asc = walk("admin", "asc").await;
        let mut reversed: Vec<String> =
            asc[1..].iter().map(|row| row.as_str().unwrap().to_owned()).collect();
        reversed.reverse();
        assert_eq!(rows, reversed, "asc là desc đảo ngược");
        rows.sort();
        rows.dedup();
        assert_eq!(rows.len(), 6, "không trùng: {rows:?}");
        assert_eq!(
            rows.iter().filter(|row| row.starts_with("\"menus.menu\"")).count(),
            3,
            "{rows:?}"
        );
        assert!(rows.iter().any(|row| row.ends_with("\"footer (vi) · Ba\"")), "{rows:?}");
        // Không đọc được menu: chỉ nguồn của `shop`.
        let seller = walk("seller", "desc").await;
        assert_eq!(seller[0], json!({"total": 3}), "{seller:?}");
        assert_eq!(seller.len(), 4, "{seller:?}");
    })
    .await
    .unwrap();
}

/// Ngôn ngữ theo khách (`menus.locales`, `menus.default_locale`): khách `b` dùng `fr` làm mặc định;
/// khách `a` vẫn theo biến chung (`en,vi`, mặc định `en`); `GET /kernel/client-config` theo khách.
#[tokio::test]
async fn ngon_ngu_menu_theo_khach() {
    TestDatabase::run(async |test| {
        let media: Arc<dyn Media> = Arc::new(fake_media());
        let all = ["menus.read", "menus.manage"];
        let app = TestApp::builder(test)
            .tenant("a")
            .tenant("b")
            .user_in("a", "admin-a", 1, &all)
            .user_in("b", "admin-b", 2, &["menus.read", "menus.manage", "kernel.settings.manage"])
            .provide::<dyn Media>(media)
            .module(Shop)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        for (name, value) in [("menus.locales", "fr, vi"), ("menus.default_locale", "fr")] {
            let set = app
                .put(&format!("/api/v1/kernel/settings/{name}"))
                .token("admin-b")
                .json(json!({"value": value}))
                .send()
                .await
                .unwrap();
            assert_eq!(set.status(), StatusCode::OK, "{}", set.text());
        }
        let body = json!({"key": "main", "items": [{"label": "Accueil", "url": "/"}]});
        let b = send(&app, "POST", "", Some("admin-b"), &[], Some(body.clone())).await;
        assert_eq!(b.status(), StatusCode::CREATED, "{}", b.json());
        assert_eq!(b.json()["data"]["locale"], "fr");
        let a = send(&app, "POST", "", Some("admin-a"), &[], Some(body)).await;
        assert_eq!(a.status(), StatusCode::CREATED, "{}", a.json());
        assert_eq!(a.json()["data"]["locale"], "en");
        let french = json!({"key": "other", "locale": "fr", "items": []});
        let refused = send(&app, "POST", "", Some("admin-a"), &[], Some(french)).await;
        assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let config = app
            .get("/api/v1/kernel/client-config")
            .header("autoxide-tenant", "b")
            .send()
            .await
            .unwrap()
            .json();
        assert_eq!(config["data"]["menus"]["default_locale"], "fr", "{config}");
    })
    .await
    .unwrap();
}
