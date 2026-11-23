//! Website và nội dung dùng được như biên tập viên thật (cả SQLite và Postgres): tác giả, chuyên mục,
//! chuyển hướng, lịch đăng, duyệt bài, di chuyển trang, loại nội dung riêng, biến thể giao diện, vị trí
//! hiển thị và banner, vùng đầu / chân trang, tìm kiếm, trang mẫu bắt buộc, SEO.

use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_testkit::{TestApp, TestDatabase};
use serde_json::{Value, json};

use super::kit::{ADMIN, ANONYMOUS, app, call_tool, expect, get, id};

/// Thay đổi có kiểm phiên bản (`If-Match`): như client không giữ ETag, gửi `*` ("đổi dù phiên bản nào").
async fn write(
    app: &autoxide_testkit::TestApp,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
    expected: u16,
) -> serde_json::Value {
    let method = axum::http::Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method.clone(), path).token(ADMIN).header("if-match", "*");
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let json: serde_json::Value =
        serde_json::from_slice(answer.body()).unwrap_or(serde_json::Value::Null);
    assert_eq!(answer.status().as_u16(), expected, "{method} {path}: {json}");
    json
}

async fn entry(app: &TestApp, kind: &str, title: &str) -> Value {
    let body = json!({"kind": kind, "title": title, "locale": "vi",
        "body": {"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Nội dung"}]}]}});
    expect(app, "POST", "/api/v1/content/entries", ADMIN, Some(body), 201).await["data"].clone()
}

#[tokio::test]
async fn tac_gia_chuyen_muc_chuyen_huong_lich_dang_va_duyet_bai() {
    TestDatabase::run(async |test| {
        let app = app(test).await;

        let author =
            json!({"display_name": "Lan Nguyễn", "slug": "lan-nguyen", "bio": "Biên tập viên"});
        let author =
            id(&expect(&app, "POST", "/api/v1/content/authors", ADMIN, Some(author), 201).await);
        let path = format!("/api/v1/content/authors/{author}");
        get(&app, &path).await;
        write(&app, "PATCH", &path, Some(json!({"bio": "Biên tập viên chính"})), 200).await;
        expect(&app, "GET", "/api/v1/content/public/authors/lan-nguyen", ANONYMOUS, None, 200)
            .await;
        write(&app, "DELETE", &path, None, 204).await;

        let term =
            json!({"taxonomy": "category", "name": "Tin tức", "slug": "tin-tuc", "locale": "vi"});
        let term = id(&expect(&app, "POST", "/api/v1/content/terms", ADMIN, Some(term), 201).await);
        let path = format!("/api/v1/content/terms/{term}");
        get(&app, &path).await;
        write(&app, "PATCH", &path, Some(json!({"description": "Tin mới", "locale": "vi"})), 200)
            .await;
        expect(
            &app,
            "GET",
            "/api/v1/content/public/terms/category/tin-tuc?locale=vi",
            ANONYMOUS,
            None,
            200,
        )
        .await;
        write(&app, "DELETE", &path, None, 204).await;

        let redirect = json!({"from_path": "/cu", "to_url": "/moi", "status_code": 301});
        let redirect =
            id(&expect(&app, "POST", "/api/v1/content/redirects", ADMIN, Some(redirect), 201)
                .await);
        let path = format!("/api/v1/content/redirects/{redirect}");
        get(&app, &path).await;
        let changed = json!({"from_path": "/cu", "to_url": "/moi-hon", "status_code": 302});
        write(&app, "PATCH", &path, Some(changed), 200).await;
        write(&app, "DELETE", &path, None, 204).await;

        // Lịch đăng: đặt rồi huỷ.
        let post = entry(&app, "post", "Ra mắt").await;
        let post_id = post["id"].as_str().unwrap().to_owned();
        let at = (autoxide_kernel::time::Timestamp::now() + jiff::SignedDuration::from_hours(48))
            .to_rfc3339();
        let schedule = json!({"action": "publish", "run_at": at});
        let schedule = write(
            &app,
            "POST",
            &format!("/api/v1/content/entries/{post_id}/schedules"),
            Some(schedule),
            201,
        )
        .await;
        get(&app, &format!("/api/v1/content/entries/{post_id}/schedules")).await;
        expect(
            &app,
            "DELETE",
            &format!("/api/v1/content/schedules/{}", id(&schedule)),
            ADMIN,
            None,
            200,
        )
        .await;

        // Xin duyệt bằng tool, rút lại bằng REST.
        let current = get(&app, &format!("/api/v1/content/entries/{post_id}")).await;
        let version = current["data"]["version"].clone();
        call_tool(
            &app,
            ADMIN,
            "content.request_review",
            json!({"id": post_id, "version": version}),
        )
        .await;
        write(
            &app,
            "DELETE",
            &format!("/api/v1/content/entries/{post_id}/request-review"),
            None,
            200,
        )
        .await;
        call_tool(&app, ADMIN, "content.seo_check", json!({"id": post_id})).await;

        // Trang con chuyển sang trang cha khác.
        let parent = entry(&app, "page", "Giới thiệu").await;
        let child = entry(&app, "page", "Đội ngũ").await;
        let moved = json!({"parent_id": parent["id"]});
        write(
            &app,
            "POST",
            &format!("/api/v1/content/entries/{}/move", child["id"].as_str().unwrap()),
            Some(moved),
            200,
        )
        .await;

        get(&app, "/api/v1/content/search?q=ra%20m%E1%BA%AFt").await;
        call_tool(&app, ADMIN, "content.search", json!({"q": "ra mắt"})).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn loai_noi_dung_giao_dien_vi_tri_banner_va_trang_mau() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let root = json!(root_of(DEFAULT_TENANT));
        let kind = json!({"key": "du_an", "labels": {"one": {"vi": "Dự án"}, "many": {"vi": "Dự án"}},
                          "routable": true, "path_prefix": "du-an"});
        expect(&app, "POST", "/api/v1/content/types", ADMIN, Some(kind), 201).await;
        get(&app, "/api/v1/content/types/du_an").await;

        let variant = json!({"preset_key": "tet", "year": 2027});
        let variant = id(&expect(&app, "POST", "/api/v1/content/theme-variants", ADMIN, Some(variant), 201).await);
        let path = format!("/api/v1/content/theme-variants/{variant}");
        get(&app, &path).await;
        // Thay toàn bộ (mẫu chỉ dùng lúc tạo): gửi lại lịch và giá trị đã đọc, đổi độ ưu tiên.
        let current = get(&app, &path).await["data"].clone();
        let replaced = json!({"name": current["name"], "schedule": current["schedule"], "overrides": current["overrides"],
                              "effect": current["effect"], "effect_intensity": current["effect_intensity"], "priority": 5});
        write(&app, "PUT", &path, Some(replaced), 200).await;

        let placement = json!({"key": "home_hero", "name": "Ảnh lớn trang chủ", "display": "carousel",
                               "display_settings": {}, "page_kinds": ["home"], "accepts": "text"});
        expect(&app, "POST", "/api/v1/content/placements", ADMIN, Some(placement), 201).await;
        let replaced = json!({"name": "Ảnh lớn trang chủ", "display": "carousel", "display_settings": {},
                              "page_kinds": ["home"], "accepts": "text", "max_items": 5});
        write(&app, "PUT", "/api/v1/content/placements/home_hero", Some(replaced), 200).await;
        let banner = json!({"placement": "home_hero", "kind": "text", "title": "Sale hè", "status": "active", "link": {"url": "/sale"}});
        let banner = id(&expect(&app, "POST", "/api/v1/content/banners", ADMIN, Some(banner), 201).await);
        get(&app, &format!("/api/v1/content/banners/{banner}")).await;
        let strip = json!({"placement": "home_hero", "kind": "text", "title": "Miễn phí giao hàng", "status": "active"});
        let second = call_tool(&app, ADMIN, "content.save_banner", strip).await;
        // Thứ tự banner theo phiên bản của vị trí (ETag lúc đọc).
        let read = app.get("/api/v1/content/placements/home_hero").token(ADMIN).send().await.unwrap();
        assert_eq!(read.status(), 200, "{}", read.text());
        let version = read.headers()["etag"].to_str().unwrap().to_owned();
        let order = json!({"key": "home_hero", "version": version, "banner_ids": [second["id"], banner]});
        call_tool(&app, ADMIN, "content.order_placement", order).await;

        let region = format!("/api/v1/content/site-regions?org_id={}&region=header&locale=vi", root.as_str().unwrap());
        get(&app, &region).await;

        let templates = get(&app, "/api/v1/content/templates").await;
        let key = templates["data"][0]["key"].as_str().unwrap_or_else(|| panic!("{templates}")).to_owned();
        let variables = json!({"owner": {"legal_name": "Công ty TNHH An", "address": "12 Tràng Tiền, Hà Nội"},
                               "contact": {"email": "lienhe@example.com", "phone": "0901234567"}});
        call_tool(&app, ADMIN, "content.draft_from_template", json!({"key": key, "locale": "vi", "variables": variables})).await;
    })
    .await
    .unwrap();
}
