//! Bản tin, biểu mẫu, bình luận, menu và thư viện ảnh dùng được như người quản trị website thật (cả
//! SQLite và Postgres): danh sách, người đăng ký, chặn địa chỉ, chiến dịch (thử, hẹn giờ, huỷ, tạm dừng,
//! chạy lại, xoá); xoá bài gửi biểu mẫu; đọc bình luận công khai và trả lời; menu bằng tool; thư mục
//! ảnh, thùng rác.

use autoxide_testkit::TestDatabase;
use serde_json::json;

use super::kit::{ADMIN, ANONYMOUS, app, app_with, call_tool, drain, expect, get, id};

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

#[tokio::test]
async fn ban_tin_danh_sach_nguoi_dang_ky_va_chien_dich() {
    TestDatabase::run(async |test| {
        // Thư có link huỷ đăng ký cần địa chỉ công khai của API, như bản cài thật.
        let app = app_with(test, |builder| builder
                .setting("AUTOXIDE_HTTP_PUBLIC_URL", "https://api.example.com")
                .setting("AUTOXIDE_NEWSLETTER_PREFERENCES_URL", "https://www.example.com/newsletter?token={token}")).await;
        let list = json!({"key": "tin-moi", "name": {"vi": "Tin mới", "en": "News"}, "double_opt_in": false});
        let list = id(&expect(&app, "POST", "/api/v1/newsletter/lists", ADMIN, Some(list), 201).await);
        let list_path = format!("/api/v1/newsletter/lists/{list}");
        get(&app, &list_path).await;
        write(&app, "PATCH", &list_path, Some(json!({"public": true})), 200).await;

        let import = json!({"list_ids": [list], "consent_source": "import", "subscribers": [
            {"email": "an@example.com", "name": "An"}, {"email": "binh@example.com", "name": "Bình"}]});
        expect(&app, "POST", "/api/v1/newsletter/subscribers/import", ADMIN, Some(import), 200).await;
        let subscribers = get(&app, "/api/v1/newsletter/subscribers").await;
        let subscriber = subscribers["data"][0]["id"].as_str().unwrap_or_else(|| panic!("{subscribers}")).to_owned();
        let path = format!("/api/v1/newsletter/subscribers/{subscriber}");
        get(&app, &path).await;
        write(&app, "PATCH", &path, Some(json!({"name": "An Trần"})), 200).await;
        write(&app, "DELETE", &path, None, 204).await;

        let suppressed = json!({"email": "spam@example.com", "note": "Báo thư rác"});
        let suppressed = id(&expect(&app, "POST", "/api/v1/newsletter/suppressions", ADMIN, Some(suppressed), 201).await);
        write(&app, "DELETE", &format!("/api/v1/newsletter/suppressions/{suppressed}"), None, 204).await;

        let campaign = |subject: &str| {
            json!({"list_id": list, "subject": subject, "body_text": "Chào bạn, có tin mới.", "locale": "vi"})
        };
        // Hẹn giờ bằng tool rồi bằng REST, huỷ.
        let first = expect(&app, "POST", "/api/v1/newsletter/campaigns", ADMIN, Some(campaign("Tháng 12")), 201).await;
        let first_id = id(&first);
        let path = format!("/api/v1/newsletter/campaigns/{first_id}");
        expect(&app, "POST", &format!("{path}/test"), ADMIN, Some(json!({"email": "editor@example.com"})), 202).await;
        let at = (autoxide_kernel::time::Timestamp::now() + jiff::SignedDuration::from_hours(24)).to_rfc3339();
        // Phiên bản để hẹn giờ bằng tool là ETag của chiến dịch.
        let read = app.get(&path).token(ADMIN).send().await.unwrap();
        let version = read.headers()["etag"].to_str().unwrap().to_owned();
        call_tool(&app, ADMIN, "newsletter.schedule_campaign", json!({"id": first_id, "at": at, "version": version})).await;
        write(&app, "POST", &format!("{path}/schedule"), Some(json!({"at": at})), 200).await;
        write(&app, "POST", &format!("{path}/cancel"), None, 200).await;

        // Gửi, tạm dừng giữa chừng, chạy lại.
        let second = id(&expect(&app, "POST", "/api/v1/newsletter/campaigns", ADMIN, Some(campaign("Tháng 1")), 201).await);
        let path = format!("/api/v1/newsletter/campaigns/{second}");
        write(&app, "POST", &format!("{path}/send"), None, 200).await;
        write(&app, "POST", &format!("{path}/pause"), None, 200).await;
        write(&app, "POST", &format!("{path}/resume"), None, 200).await;
        drain(&app).await;

        let third = id(&expect(&app, "POST", "/api/v1/newsletter/campaigns", ADMIN, Some(campaign("Nháp")), 201).await);
        write(&app, "DELETE", &format!("/api/v1/newsletter/campaigns/{third}"), None, 204).await;
        // Danh sách còn chiến dịch chưa xong thì không xoá được; danh sách cũ không dùng nữa thì xoá.
        let old = json!({"key": "cu", "name": {"vi": "Cũ"}});
        let old = id(&expect(&app, "POST", "/api/v1/newsletter/lists", ADMIN, Some(old), 201).await);
        write(&app, "DELETE", &format!("/api/v1/newsletter/lists/{old}"), None, 204).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bieu_mau_binh_luan_menu_va_thu_vien_anh() {
    TestDatabase::run(async |test| {
        let app = app(test).await;

        // Biểu mẫu liên hệ: một bài gửi (người gọi có `forms.submit`, như máy chủ website), rồi xoá.
        let form = json!({"key": "lien-he", "name": "Liên hệ", "status": "open",
            "fields": [{"key": "email", "type": "email", "label": {"vi": "Email"}, "required": true}]});
        expect(&app, "POST", "/api/v1/forms", ADMIN, Some(form), 201).await;
        let submission = json!({"data": {"email": "khach@example.com"}});
        let sent = expect(&app, "POST", "/api/v1/forms/public/lien-he/submissions", ADMIN, Some(submission), 201).await;
        let submission = sent["data"]["id"].as_str().unwrap_or_else(|| panic!("{sent}")).to_owned();
        write(&app, "DELETE", &format!("/api/v1/forms/submissions/{submission}"), None, 204).await;

        // Bình luận và trả lời của người đã đăng nhập (chọn tên hiển thị trước), đọc công khai.
        expect(&app, "PUT", "/api/v1/comments/profile", ADMIN, Some(json!({"display_name": "Ban biên tập"})), 200).await;
        let post = json!({"kind": "post", "title": "Ra mắt", "locale": "vi"});
        let post = id(&expect(&app, "POST", "/api/v1/content/entries", ADMIN, Some(post), 201).await);
        write(&app, "POST", &format!("/api/v1/content/entries/{post}/publish"), None, 200).await;
        let comment = json!({"target_type": "content.entry", "target_id": post, "body": "Bài hay!"});
        let comment = id(&expect(&app, "POST", "/api/v1/comments/public/comments", ADMIN, Some(comment), 201).await);
        let reply = json!({"target_type": "content.entry", "target_id": post, "body": "Cảm ơn bạn", "parent_id": comment});
        expect(&app, "POST", "/api/v1/comments/public/comments", ADMIN, Some(reply), 201).await;
        expect(&app, "GET", &format!("/api/v1/comments/public/comments/{comment}"), ANONYMOUS, None, 200).await;
        expect(&app, "GET", &format!("/api/v1/comments/public/comments/{comment}/replies"), ANONYMOUS, None, 200).await;

        // Menu: tạo bằng REST, đọc và xoá bằng tool; nhập menu cũ của `content` (thử trước).
        let menu = json!({"key": "main", "locale": "vi", "items": [{"label": "Trang chủ", "url": "/"}]});
        let menu = id(&expect(&app, "POST", "/api/v1/menus", ADMIN, Some(menu), 201).await);
        call_tool(&app, ADMIN, "menus.get_menu", json!({"id": menu})).await;
        call_tool(&app, ADMIN, "menus.delete_menu", json!({"id": menu})).await;
        expect(&app, "POST", "/api/v1/menus/import-content", ADMIN, Some(json!({"dry_run": true})), 200).await;

        // Ảnh: thư mục rỗng xoá được; ảnh vào thùng rác rồi khôi phục.
        let folder = id(&expect(&app, "POST", "/api/v1/media/folders", ADMIN, Some(json!({"name": "Tạm"})), 201).await);
        write(&app, "DELETE", &format!("/api/v1/media/folders/{folder}"), None, 204).await;
        let png = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/guides/files/photo.png")).unwrap();
        let uploaded = app
            .post("/api/v1/media?name=photo.png")
            .token(ADMIN)
            .header("content-type", "image/png")
            .bytes(png)
            .send()
            .await
            .unwrap();
        assert_eq!(uploaded.status(), 201, "{}", uploaded.text());
        let item = uploaded.json()["data"]["id"].as_str().unwrap().to_owned();
        call_tool(&app, ADMIN, "media.get", json!({"id": item})).await;
        write(&app, "DELETE", &format!("/api/v1/media/{item}"), None, 200).await;
        expect(&app, "POST", &format!("/api/v1/media/{item}/restore"), ADMIN, None, 200).await;
    })
    .await
    .unwrap();
}

/// Ảnh lớn tải thẳng lên S3 bằng REST và bằng tool. Chỉ chạy khi có S3 (gate dựng máy chủ S3 giả lập).
#[tokio::test]
async fn anh_lon_tai_thang_len_s3_bang_rest_va_tool() {
    use sha2::{Digest as _, Sha256};
    let Some(s3) = autoxide_testkit::s3_settings() else {
        return;
    };
    TestDatabase::run(async |test| {
        let app = app_with(test, |mut builder| {
            for (name, value) in &s3 {
                builder = builder.setting(name, value);
            }
            builder
        })
        .await;
        autoxide_kernel::tls::install_crypto_provider();
        let png = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/guides/files/photo.png")).unwrap();
        let sha256: String = Sha256::digest(&png).iter().map(|b| format!("{b:02x}")).collect();
        for (n, via_tool) in [(1, false), (2, true)] {
            let start = json!({"name": format!("banner-{n}.png"), "content_type": "image/png", "size": png.len(), "sha256": sha256});
            let link = if via_tool {
                call_tool(&app, ADMIN, "media.create_upload", start).await
            } else {
                expect(&app, "POST", "/api/v1/media/uploads", ADMIN, Some(start), 201).await["data"].clone()
            };
            let mut put = reqwest::Client::new().put(link["url"].as_str().unwrap()).body(png.clone());
            for (name, value) in link["headers"].as_object().unwrap() {
                put = put.header(name.as_str(), value.as_str().unwrap());
            }
            assert!(put.send().await.unwrap().status().is_success());
            let upload = link["id"].as_str().unwrap();
            if via_tool {
                call_tool(&app, ADMIN, "media.confirm_upload", json!({"id": upload})).await;
            } else {
                expect(&app, "POST", &format!("/api/v1/media/uploads/{upload}/complete"), ADMIN, None, 201).await;
            }
        }
    })
    .await
    .unwrap();
}
