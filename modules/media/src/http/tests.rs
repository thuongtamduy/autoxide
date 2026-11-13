//! Test qua HTTP (router thật của kernel) và MCP (client thật): header tải về, công khai,
//! `Range`, 304, `If-Match`, permission của từng endpoint và từng tool. Gate chạy trên cả
//! SQLite và Postgres.

use std::sync::Arc;

use autoxide_kernel::ai::TextModel;
use autoxide_kernel::storage::Storage;
use autoxide_testkit::ai::FakeTextModel;
use autoxide_testkit::{TestApp, TestDatabase, TestResponse, tool};
use axum::http::{Method, StatusCode, header};
use sea_orm::{ConnectionTrait, Statement};
use serde_json::{Value, json};

use crate::sniff::samples;
use crate::testing::MemoryStorage;

struct App {
    http: TestApp,
    storage: Arc<MemoryStorage>,
}

/// Token → permission: `admin` mọi quyền của module; `reader` chỉ đọc; `writer` đọc + ghi;
/// `nobody` không quyền nào. Nơi lưu trong bộ nhớ thay cho module `files`; module `ai` giả (đã
/// tắt): `media` cần `dyn TextModel` cho gợi ý alt text.
async fn app(test: &TestDatabase) -> App {
    let storage = Arc::new(MemoryStorage::default());
    let shared: Arc<dyn Storage> = storage.clone();
    let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
    let http = TestApp::builder(test)
        .setting("AUTOXIDE_MEDIA_MAX_BYTES", "200000")
        .setting("AUTOXIDE_MEDIA_IMAGE_WIDTHS", "100")
        .user("admin", 1, &["media.read", "media.write", "media.purge"])
        .user("writer", 1, &["media.read", "media.write"])
        .user("reader", 1, &["media.read"])
        .user("nobody", 1, &[])
        .provide::<dyn Storage>(shared)
        .provide::<dyn TextModel>(text)
        .module(crate::module())
        .build()
        .await
        .unwrap();
    App { http, storage }
}

/// Gửi request; `headers`: các cặp header thêm vào (kể cả `authorization`).
async fn send(
    app: &App,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: Vec<u8>,
) -> TestResponse {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.http.request(method, &format!("/api/v1/media{uri}"));
    for (name, value) in headers {
        request = request.header(name, value);
    }
    request.bytes(body).send().await.unwrap()
}

fn as_user(who: &str) -> String {
    format!("Bearer {who}")
}

async fn upload(app: &App, name: &str, content_type: &str, bytes: &[u8]) -> Value {
    let auth = as_user("admin");
    let headers = [("authorization", auth.as_str()), ("content-type", content_type)];
    let answer = send(app, "POST", &format!("?name={name}"), &headers, bytes.to_vec()).await;
    assert_eq!(answer.status(), StatusCode::CREATED, "{}", answer.json());
    answer.json()["data"].clone()
}

async fn patch(app: &App, id: &str, if_match: Option<&str>, body: Value) -> TestResponse {
    let auth = as_user("admin");
    let mut headers = vec![("authorization", auth.as_str()), ("content-type", "application/json")];
    if let Some(etag) = if_match {
        headers.push(("if-match", etag));
    }
    send(app, "PATCH", &format!("/{id}"), &headers, body.to_string().into_bytes()).await
}

/// Tải lên rồi tải về qua HTTP: đúng byte; header `inline` chỉ cho ảnh raster; luôn có
/// `nosniff` và CSP; `Location`; cache theo phiên bản trong URL.
#[tokio::test]
async fn tai_len_tai_ve_va_header_an_toan() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let png = samples::png(4, 3);
        let auth = as_user("admin");
        let headers = [("authorization", auth.as_str()), ("content-type", "image/png")];
        let answer =
            send(&app, "POST", "?name=Logo%20%C4%91%E1%BA%B9p.png", &headers, png.clone()).await;
        assert_eq!(answer.status(), StatusCode::CREATED, "{}", answer.json());
        let item = answer.json()["data"].clone();
        let id = item["id"].as_str().unwrap();
        assert_eq!(answer.header(header::LOCATION), format!("/api/v1/media/{id}"));
        assert_eq!((item["width"].as_i64(), item["height"].as_i64()), (Some(4), Some(3)));
        assert_eq!(item["name"], "Logo đẹp.png");
        assert_eq!(item["duplicates"], json!([]));
        let url = item["url"].as_str().unwrap();
        assert_eq!(
            url,
            format!("/api/v1/media/{id}/content?v={}", item["version_id"].as_str().unwrap())
        );

        let path = url.trim_start_matches("/api/v1/media");
        let reader = as_user("reader");
        let got = send(&app, "GET", path, &[("authorization", reader.as_str())], Vec::new()).await;
        assert_eq!(got.status(), StatusCode::OK);
        assert_eq!(got.body(), png);
        assert_eq!(got.header(header::CONTENT_TYPE), "image/png");
        assert_eq!(got.header(header::CONTENT_LENGTH), png.len().to_string());
        assert!(got.header(header::CONTENT_DISPOSITION).starts_with("inline; "));
        assert_eq!(got.header(header::X_CONTENT_TYPE_OPTIONS), "nosniff");
        assert_eq!(got.header(header::CONTENT_SECURITY_POLICY), "default-src 'none'; sandbox");
        assert_eq!(got.header(header::ETAG), format!("\"{}\"", item["sha256"].as_str().unwrap()));
        assert_eq!(got.header(header::ACCEPT_RANGES), "bytes");
        assert_eq!(got.header(header::CACHE_CONTROL), "private, max-age=31536000, immutable");
        // Không kèm phiên bản: cache ngắn.
        let plain = format!("/{id}/content");
        let got =
            send(&app, "GET", &plain, &[("authorization", reader.as_str())], Vec::new()).await;
        assert_eq!(got.header(header::CACHE_CONTROL), "private, no-cache");

        // SVG, PDF: luôn tải về, không mở trong trang.
        for (name, content_type, bytes) in
            [("a.svg", "image/svg+xml", samples::SVG), ("a.pdf", "application/pdf", samples::PDF)]
        {
            let item = upload(&app, name, content_type, bytes).await;
            let path = format!("/{}/content", item["id"].as_str().unwrap());
            let got =
                send(&app, "GET", &path, &[("authorization", reader.as_str())], Vec::new()).await;
            assert_eq!(got.body(), bytes);
            assert!(got.header(header::CONTENT_DISPOSITION).starts_with("attachment; "), "{name}");
            assert_eq!(got.header(header::CONTENT_SECURITY_POLICY), "default-src 'none'; sandbox");
            assert_eq!(got.header(header::X_CONTENT_TYPE_OPTIONS), "nosniff");
        }

        // Khai sai, không được phép, quá cỡ.
        let html = [("authorization", auth.as_str()), ("content-type", "image/png")];
        let refused = send(&app, "POST", "?name=x.png", &html, samples::HTML.to_vec()).await;
        assert_eq!(
            (refused.status(), refused.json()["code"].clone()),
            (StatusCode::UNSUPPORTED_MEDIA_TYPE, json!("media.content_mismatch"))
        );
        let zip = [("authorization", auth.as_str()), ("content-type", "application/zip")];
        let refused = send(&app, "POST", "?name=x.zip", &zip, b"PK\x03\x04".to_vec()).await;
        assert_eq!(refused.json()["code"], "media.type_not_allowed");
        let big = [samples::png(1, 1), vec![0; 200_000]].concat();
        let refused = send(&app, "POST", "?name=x.png", &html, big).await;
        assert_eq!(refused.status(), StatusCode::PAYLOAD_TOO_LARGE);
    })
    .await
    .unwrap();
}

/// Không đăng nhập: item công khai tải được; riêng tư hay trong thùng rác → 404; token sai
/// vẫn là 401.
#[tokio::test]
async fn noi_dung_cong_khai() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let item = upload(&app, "a.png", "image/png", &samples::png(1, 1)).await;
        let id = item["id"].as_str().unwrap().to_owned();
        let path = format!("/{id}/content");

        let hidden = send(&app, "GET", &path, &[], Vec::new()).await;
        assert_eq!(
            (hidden.status(), hidden.json()["code"].clone()),
            (StatusCode::NOT_FOUND, json!("media.item_not_found"))
        );
        let stranger = as_user("nobody");
        let hidden =
            send(&app, "GET", &path, &[("authorization", stranger.as_str())], Vec::new()).await;
        assert_eq!(hidden.status(), StatusCode::NOT_FOUND);

        let changed = patch(&app, &id, Some("*"), json!({"visibility": "public"})).await;
        assert_eq!(changed.status(), StatusCode::OK, "{}", changed.json());
        let versioned = format!("{path}?v={}", item["version_id"].as_str().unwrap());
        let open = send(&app, "GET", &versioned, &[], Vec::new()).await;
        assert_eq!(open.status(), StatusCode::OK);
        assert_eq!(open.header(header::CACHE_CONTROL), "public, max-age=31536000, immutable");
        let open = send(&app, "GET", &path, &[], Vec::new()).await;
        assert_eq!(open.header(header::CACHE_CONTROL), "public, max-age=60, must-revalidate");
        let bad = send(&app, "GET", &path, &[("authorization", "Bearer forged")], Vec::new()).await;
        assert_eq!(bad.status(), StatusCode::UNAUTHORIZED);

        let auth = as_user("admin");
        let trashed = send(
            &app,
            "DELETE",
            &format!("/{id}"),
            &[("authorization", auth.as_str())],
            Vec::new(),
        )
        .await;
        assert_eq!(trashed.status(), StatusCode::OK);
        assert_eq!(send(&app, "GET", &path, &[], Vec::new()).await.status(), StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}

/// `Range` → 206 / 416, `HEAD` không thân, `If-None-Match` → 304.
#[tokio::test]
async fn range_head_va_304() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let pdf = samples::PDF;
        let item = upload(&app, "a.pdf", "application/pdf", pdf).await;
        let path = format!("/{}/content", item["id"].as_str().unwrap());
        let auth = as_user("reader");
        let size = pdf.len();

        let part = send(
            &app,
            "GET",
            &path,
            &[("authorization", auth.as_str()), ("range", "bytes=2-6")],
            Vec::new(),
        )
        .await;
        assert_eq!(part.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(part.body(), &pdf[2..7]);
        assert_eq!(part.header(header::CONTENT_RANGE), format!("bytes 2-6/{size}"));
        assert_eq!(part.header(header::CONTENT_LENGTH), "5");
        let tail = send(
            &app,
            "GET",
            &path,
            &[("authorization", auth.as_str()), ("range", "bytes=-4")],
            Vec::new(),
        )
        .await;
        assert_eq!(tail.body(), &pdf[size - 4..]);
        let outside = format!("bytes={size}-");
        let wrong = send(
            &app,
            "GET",
            &path,
            &[("authorization", auth.as_str()), ("range", outside.as_str())],
            Vec::new(),
        )
        .await;
        assert_eq!(wrong.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(wrong.json()["code"], "media.range_not_satisfiable");
        assert_eq!(wrong.header(header::CONTENT_RANGE), format!("bytes */{size}"));

        let head = send(&app, "HEAD", &path, &[("authorization", auth.as_str())], Vec::new()).await;
        assert_eq!(head.status(), StatusCode::OK);
        assert!(head.body().is_empty());
        assert_eq!(head.header(header::CONTENT_LENGTH), size.to_string());
        assert_eq!(head.header(header::CONTENT_TYPE), "application/pdf");

        let etag = head.header(header::ETAG).to_owned();
        let cached = send(
            &app,
            "GET",
            &path,
            &[("authorization", auth.as_str()), ("if-none-match", etag.as_str())],
            Vec::new(),
        )
        .await;
        assert_eq!(cached.status(), StatusCode::NOT_MODIFIED);
        assert!(cached.body().is_empty());
        assert_eq!(cached.header(header::ETAG), etag);
    })
    .await
    .unwrap();
}

/// `If-Match` bắt buộc khi sửa và khi thay nội dung: thiếu → 428, cũ → 412. Thay nội
/// dung giữ ID; `ETag` và `url` đổi; nội dung cũ bị xoá.
#[tokio::test]
async fn if_match_bat_buoc_va_thay_noi_dung() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let item = upload(&app, "a.png", "image/png", &samples::png(1, 1)).await;
        let id = item["id"].as_str().unwrap().to_owned();
        let auth = as_user("admin");
        let got =
            send(&app, "GET", &format!("/{id}"), &[("authorization", auth.as_str())], Vec::new())
                .await;
        let etag = got.header(header::ETAG).to_owned();

        let missing = patch(&app, &id, None, json!({"alt": "x"})).await;
        assert_eq!(
            (missing.status(), missing.json()["code"].clone()),
            (StatusCode::PRECONDITION_REQUIRED, json!("precondition_required"))
        );
        let changed =
            patch(&app, &id, Some(&etag), json!({"alt": "A logo", "folder_id": null})).await;
        assert_eq!(changed.status(), StatusCode::OK, "{}", changed.json());
        let new_etag = changed.header(header::ETAG).to_owned();
        assert_ne!(new_etag, etag);
        let stale = patch(&app, &id, Some(&etag), json!({"alt": "y"})).await;
        assert_eq!(stale.status(), StatusCode::PRECONDITION_FAILED);
        let invalid = patch(&app, &id, Some(&new_etag), json!({"visibility": "everyone"})).await;
        assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let path = format!("/{id}/content");
        let webp = samples::webp(10, 20);
        let no_match = [("authorization", auth.as_str()), ("content-type", "image/webp")];
        let refused = send(&app, "PUT", &path, &no_match, webp.clone()).await;
        assert_eq!(refused.status(), StatusCode::PRECONDITION_REQUIRED);
        let old_match = [
            ("authorization", auth.as_str()),
            ("content-type", "image/webp"),
            ("if-match", etag.as_str()),
        ];
        assert_eq!(
            send(&app, "PUT", &path, &old_match, webp.clone()).await.status(),
            StatusCode::PRECONDITION_FAILED
        );
        let ok = [
            ("authorization", auth.as_str()),
            ("content-type", "image/webp"),
            ("if-match", new_etag.as_str()),
        ];
        let replaced = send(&app, "PUT", &path, &ok, webp.clone()).await;
        assert_eq!(replaced.status(), StatusCode::OK, "{}", replaced.json());
        let after = replaced.json()["data"].clone();
        assert_eq!(after["id"], item["id"]);
        assert_ne!(after["url"], item["url"]);
        assert_eq!(
            (after["width"].as_i64(), after["content_type"].as_str()),
            (Some(10), Some("image/webp"))
        );
        let keys = app.storage.keys().await.unwrap();
        assert_eq!(keys, [format!("media/{id}/{}", after["version_id"].as_str().unwrap())]);
        let got = send(&app, "GET", &path, &[("authorization", auth.as_str())], Vec::new()).await;
        assert_eq!(got.body(), webp);
    })
    .await
    .unwrap();
}

/// Mỗi endpoint đòi đúng permission của nó.
#[tokio::test]
async fn permission_cua_tung_endpoint() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let item = upload(&app, "a.png", "image/png", &samples::png(1, 1)).await;
        let id = item["id"].as_str().unwrap();
        let folder = {
            let auth = as_user("admin");
            let headers = [("authorization", auth.as_str()), ("content-type", "application/json")];
            let answer =
                send(&app, "POST", "/folders", &headers, br#"{"name":"F"}"#.to_vec()).await;
            assert_eq!(answer.status(), StatusCode::CREATED);
            answer.json()["data"]["id"].as_str().unwrap().to_owned()
        };
        // (method, đường dẫn, người thiếu quyền)
        let cases = [
            ("GET", String::new(), "nobody"),
            ("POST", "?name=a.png".to_owned(), "reader"),
            ("GET", format!("/{id}"), "nobody"),
            ("PATCH", format!("/{id}"), "reader"),
            ("DELETE", format!("/{id}"), "reader"),
            ("POST", format!("/{id}/restore"), "reader"),
            ("POST", format!("/{id}/purge"), "writer"),
            ("PUT", format!("/{id}/content"), "reader"),
            ("GET", "/folders".to_owned(), "nobody"),
            ("POST", "/folders".to_owned(), "reader"),
            ("GET", format!("/folders/{folder}"), "nobody"),
            ("PATCH", format!("/folders/{folder}"), "reader"),
            ("DELETE", format!("/folders/{folder}"), "reader"),
        ];
        for (method, path, who) in cases {
            let auth = as_user(who);
            let headers = [
                ("authorization", auth.as_str()),
                ("content-type", "application/json"),
                ("if-match", "*"),
            ];
            let body = if method == "POST" && path.starts_with('?') || method == "PUT" {
                samples::png(1, 1)
            } else if method == "PATCH" || path == "/folders" {
                br#"{"name":"x"}"#.to_vec()
            } else {
                Vec::new()
            };
            let answer = send(&app, method, &path, &headers, body).await;
            assert_eq!(
                answer.status(),
                StatusCode::FORBIDDEN,
                "{method} {path}: {}",
                answer.json()
            );
            assert_eq!(answer.json()["code"], "permission_denied");
        }
        // Thiếu đăng nhập hẳn: 401.
        assert_eq!(send(&app, "GET", "", &[], Vec::new()).await.status(), StatusCode::UNAUTHORIZED);
    })
    .await
    .unwrap();
}

/// Tham số danh sách sai: mọi lỗi (bộ lọc lẫn phân trang) báo một lần.
#[tokio::test]
async fn tham_so_danh_sach_sai_bao_mot_lan() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let auth = as_user("reader");
        let uri = "?type=png&visibility=all&trashed=maybe&sort=color&folder_id=x&limit=0";
        let answer = send(&app, "GET", uri, &[("authorization", auth.as_str())], Vec::new()).await;
        assert_eq!(answer.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let fields: Vec<String> = answer.json()["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|error| error["field"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(fields, ["type", "visibility", "trashed", "folder_id", "sort", "limit"]);
        let ok = send(
            &app,
            "GET",
            "?folder_id=root&type=image/*&sort=name&order=asc",
            &[("authorization", auth.as_str())],
            Vec::new(),
        )
        .await;
        assert_eq!(ok.status(), StatusCode::OK, "{}", ok.json());
    })
    .await
    .unwrap();
}

/// Tool MCP: người thiếu quyền không thấy tool; tool gọi cùng service với REST.
#[tokio::test]
async fn tool_mcp_theo_permission() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let item = upload(&app, "a.png", "image/png", &samples::png(2, 2)).await;
        let client = async |who: &str| app.http.mcp(who).await.unwrap();
        let names = async |who: &str| {
            let client = client(who).await;
            let mut names: Vec<String> = client
                .list_all_tools()
                .await
                .unwrap()
                .into_iter()
                .map(|tool| tool.name.to_string())
                .collect();
            names.sort();
            client.cancel().await.unwrap();
            names
        };
        assert_eq!(
            names("writer").await,
            [
                "media.get",
                "media.list",
                "media.list_tags",
                "media.suggest_alt",
                "media.trash",
                "media.update",
                "media.upload"
            ]
        );
        assert_eq!(names("reader").await, ["media.get", "media.list", "media.list_tags"]);
        assert!(names("nobody").await.is_empty());

        let writer = client("writer").await;
        let call = |name: &str, arguments: Value| tool(name, arguments);
        let listed = writer
            .call_tool(call("media.list", json!({"type": "image/*", "sort": "name"})))
            .await
            .unwrap();
        let listed = listed.structured_content.unwrap();
        assert_eq!(listed["data"][0]["id"], item["id"]);
        let update =
            json!({"id": item["id"], "updated_at": item["updated_at"], "alt": "Two by two"});
        let updated = writer.call_tool(call("media.update", update.clone())).await.unwrap();
        assert_eq!(updated.structured_content.unwrap()["data"]["alt"], "Two by two");
        // Lần hai với `updated_at` cũ: từ chối.
        let stale = writer.call_tool(call("media.update", update)).await.unwrap();
        assert_eq!(stale.is_error, Some(true));
        let trashed =
            writer.call_tool(call("media.trash", json!({"id": item["id"]}))).await.unwrap();
        assert!(trashed.structured_content.unwrap()["data"]["trashed_at"].is_string());
        writer.cancel().await.unwrap();
    })
    .await
    .unwrap();
}

/// Ảnh thu nhỏ qua đường thật: tải lên → worker chạy job → item có `variants` → `?w=` trả
/// bản nhỏ (JPEG, mở trong trang), `&format=webp` trả bản WebP; cỡ chưa có thì trả bản gốc với
/// cache ngắn, bản WebP chưa có thì bản JPEG với cache ngắn.
#[tokio::test]
async fn anh_thu_nho_qua_worker_va_tham_so_w() {
    use image::ImageFormat;

    TestDatabase::run(async |test| {
        let app = app(test).await;
        let png = crate::variants::samples::encoded(240, 120, ImageFormat::Png, false);
        let item = upload(&app, "wide.png", "image/png", &png).await;
        assert_eq!(item["variants"], json!([]), "made in the background");

        // Worker chạy mọi job đang chờ (cả hai lịch định kỳ của module lẫn job làm ảnh).
        let worker = app.http.modules().worker(app.http.database(), 4).unwrap();
        while worker.run_once().await.unwrap() > 0 {}
        let id = item["id"].as_str().unwrap();
        let auth = as_user("reader");
        let got =
            send(&app, "GET", &format!("/{id}"), &[("authorization", auth.as_str())], Vec::new())
                .await;
        let variants = got.json()["data"]["variants"].clone();
        assert_eq!(variants.as_array().unwrap().len(), 1, "{variants}");
        assert_eq!(
            (variants[0]["width"].as_i64(), variants[0]["height"].as_i64()),
            (Some(100), Some(50))
        );
        let url =
            variants[0]["url"].as_str().unwrap().trim_start_matches("/api/v1/media").to_owned();
        assert!(url.ends_with("&w=100"), "{url}");

        let small = send(&app, "GET", &url, &[("authorization", auth.as_str())], Vec::new()).await;
        assert_eq!(small.status(), StatusCode::OK);
        assert_eq!(small.header(header::CONTENT_TYPE), "image/jpeg");
        assert_eq!(small.header(header::CACHE_CONTROL), "private, max-age=31536000, immutable");
        assert!(small.header(header::CONTENT_DISPOSITION).starts_with("inline; "));
        let decoded = image::load_from_memory(small.body()).unwrap();
        assert_eq!(decoded.width(), 100);
        let etag = small.header(header::ETAG).to_owned();
        let again = send(
            &app,
            "GET",
            &url,
            &[("authorization", auth.as_str()), ("if-none-match", etag.as_str())],
            Vec::new(),
        )
        .await;
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED);

        // Cỡ không có: bản gốc.
        let other = url.replace("w=100", "w=640");
        let original =
            send(&app, "GET", &other, &[("authorization", auth.as_str())], Vec::new()).await;
        assert_eq!((original.status(), original.body().len()), (StatusCode::OK, png.len()));
        assert_eq!(original.header(header::CONTENT_TYPE), "image/png");

        // Bản WebP cùng cỡ (D-186): địa chỉ riêng, ETag riêng, cache lâu như bản JPEG.
        let webp_url = variants[0]["webp_url"].as_str().unwrap();
        assert_eq!(webp_url, format!("{}&format=webp", variants[0]["url"].as_str().unwrap()));
        assert!(variants[0]["webp_size"].as_u64().unwrap() > 0, "{variants}");
        let webp_url = webp_url.trim_start_matches("/api/v1/media").to_owned();
        let webp =
            send(&app, "GET", &webp_url, &[("authorization", auth.as_str())], Vec::new()).await;
        assert_eq!(webp.status(), StatusCode::OK);
        assert_eq!(webp.header(header::CONTENT_TYPE), "image/webp");
        assert_eq!(webp.header(header::CACHE_CONTROL), "private, max-age=31536000, immutable");
        assert_ne!(webp.header(header::ETAG), etag);
        assert_eq!(image::guess_format(webp.body()).unwrap(), ImageFormat::WebP);
        assert_eq!(image::load_from_memory(webp.body()).unwrap().width(), 100);

        // `format` lạ: báo rõ thay vì lặng lẽ trả JPEG.
        let wrong = webp_url.replace("format=webp", "format=avif");
        let wrong =
            send(&app, "GET", &wrong, &[("authorization", auth.as_str())], Vec::new()).await;
        assert_eq!(wrong.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(wrong.json()["errors"][0]["field"], "format");

        // Bản WebP chưa có (ảnh làm trước khi có WebP): gửi bản JPEG cùng cỡ, cache ngắn — địa
        // chỉ sẽ ra WebP khi bản đó được làm.
        let database = app.http.database().write();
        let backend = database.get_database_backend();
        database
            .execute_raw(Statement::from_string(
                backend,
                "DELETE FROM media_variant WHERE content_type = 'image/webp'",
            ))
            .await
            .unwrap();
        let fallback =
            send(&app, "GET", &webp_url, &[("authorization", auth.as_str())], Vec::new()).await;
        assert_eq!(fallback.status(), StatusCode::OK);
        assert_eq!(fallback.header(header::CONTENT_TYPE), "image/jpeg");
        assert_eq!(fallback.header(header::CACHE_CONTROL), "private, no-cache");
        let got =
            send(&app, "GET", &format!("/{id}"), &[("authorization", auth.as_str())], Vec::new())
                .await;
        assert_eq!(got.json()["data"]["variants"][0]["webp_url"], Value::Null);
    })
    .await
    .unwrap();
}

/// Nhãn qua REST: gắn khi tải lên (`tags=a,b`), sửa bằng `PATCH`, lọc `tag=`, liệt kê, đổi
/// tên (cần `If-Match`), xoá; permission của từng endpoint nhãn.
#[tokio::test]
async fn nhan_qua_rest() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let auth = as_user("admin");
        let headers = [("authorization", auth.as_str()), ("content-type", "image/png")];
        let answer =
            send(&app, "POST", "?name=a.png&tags=Brand,%20logo", &headers, samples::png(1, 1))
                .await;
        assert_eq!(answer.status(), StatusCode::CREATED, "{}", answer.json());
        let item = answer.json()["data"].clone();
        assert_eq!(item["tags"], json!(["Brand", "logo"]));
        let id = item["id"].as_str().unwrap();

        let changed = patch(&app, id, Some("*"), json!({"tags": ["logo", "Summer"]})).await;
        assert_eq!(changed.json()["data"]["tags"], json!(["logo", "Summer"]));
        let listed =
            send(&app, "GET", "?tag=summer", &[("authorization", auth.as_str())], Vec::new()).await;
        assert_eq!(listed.json()["data"].as_array().unwrap().len(), 1);

        let tags =
            send(&app, "GET", "/tags?order=asc", &[("authorization", auth.as_str())], Vec::new())
                .await;
        let names: Vec<(String, u64)> = tags.json()["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tag| {
                (tag["name"].as_str().unwrap().to_owned(), tag["item_count"].as_u64().unwrap())
            })
            .collect();
        assert_eq!(
            names,
            [("Brand".to_owned(), 0), ("logo".to_owned(), 1), ("Summer".to_owned(), 1)]
        );
        let summer = tags.json()["data"][2]["id"].as_str().unwrap().to_owned();

        let rename = |if_match: Option<&'static str>| {
            let mut headers =
                vec![("authorization", auth.as_str()), ("content-type", "application/json")];
            if let Some(value) = if_match {
                headers.push(("if-match", value));
            }
            headers
        };
        let path = format!("/tags/{summer}");
        let missing =
            send(&app, "PATCH", &path, &rename(None), r#"{"name":"Hè"}"#.as_bytes().to_vec()).await;
        assert_eq!(missing.status(), StatusCode::PRECONDITION_REQUIRED);
        let renamed =
            send(&app, "PATCH", &path, &rename(Some("*")), r#"{"name":"Hè"}"#.as_bytes().to_vec())
                .await;
        assert_eq!(
            (renamed.status(), renamed.json()["data"]["name"].clone()),
            (StatusCode::OK, json!("Hè"))
        );
        let deleted =
            send(&app, "DELETE", &path, &[("authorization", auth.as_str())], Vec::new()).await;
        assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
        let gone = send(&app, "GET", &path, &[("authorization", auth.as_str())], Vec::new()).await;
        assert_eq!(gone.json()["code"], "media.tag_not_found");

        let tag_id = tags.json()["data"][1]["id"].as_str().unwrap().to_owned();
        for (method, path, who) in [
            ("GET", "/tags".to_owned(), "nobody"),
            ("GET", format!("/tags/{tag_id}"), "nobody"),
            ("PATCH", format!("/tags/{tag_id}"), "reader"),
            ("DELETE", format!("/tags/{tag_id}"), "reader"),
        ] {
            let auth = as_user(who);
            let headers = [
                ("authorization", auth.as_str()),
                ("content-type", "application/json"),
                ("if-match", "*"),
            ];
            let answer = send(&app, method, &path, &headers, br#"{"name":"x"}"#.to_vec()).await;
            assert_eq!(answer.status(), StatusCode::FORBIDDEN, "{method} {path}");
        }
    })
    .await
    .unwrap();
}

/// Tải lên qua MCP: nội dung base64, kiểm loại thật như REST, giới hạn 1 MiB; người chỉ có
/// quyền đọc không thấy tool.
#[tokio::test]
async fn tai_len_qua_mcp() {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;

    TestDatabase::run(async |test| {
        let app = app(test).await;
        let client = app.http.mcp("writer").await.unwrap();
        let call = |arguments: Value| tool("media.upload", arguments);

        let png = samples::png(7, 5);
        let arguments = json!({"name": "dot.png", "content_base64": STANDARD.encode(&png), "tags": ["tiny"]});
        let uploaded = client.call_tool(call(arguments)).await.unwrap();
        assert_eq!(uploaded.is_error, Some(false), "{uploaded:?}");
        let data = uploaded.structured_content.unwrap()["data"].clone();
        assert_eq!((data["content_type"].as_str(), data["width"].as_i64()), (Some("image/png"), Some(7)));
        assert_eq!(data["tags"], json!(["tiny"]));
        assert_eq!(data["duplicates"], json!([]));
        let auth = as_user("reader");
        let path = format!("/{}/content", data["id"].as_str().unwrap());
        let got = send(&app, "GET", &path, &[("authorization", auth.as_str())], Vec::new()).await;
        assert_eq!(got.body(), png);

        // Nội dung giả, base64 hỏng: lỗi rõ.
        let fake = json!({"name": "x.png", "content_type": "image/png", "content_base64": STANDARD.encode(samples::HTML)});
        let refused = client.call_tool(call(fake)).await.unwrap();
        assert_eq!(refused.is_error, Some(true));
        let broken = client.call_tool(call(json!({"name": "x.png", "content_base64": "%%%"}))).await.unwrap();
        assert_eq!(broken.is_error, Some(true));
        client.cancel().await.unwrap();
    })
    .await
    .unwrap();
}
