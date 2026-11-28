//! Tìm nâng cao qua `GET /api/v1/kernel/find` (cú pháp `kind:`, `status:`, `updated>`…) và mở
//! thẳng theo mã.

use super::*;

/// Tìm nâng cao và mở thẳng (bước 51 P9): `is:`, `status:`, phủ định, so ngày; trường lạ vào
/// `ignored`; `GET /kernel/resolve?code=` mở bài theo slug hay đường dẫn, mã lạ là 404.
#[tokio::test]
async fn tim_nang_cao_va_mo_thang_theo_ma() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        for (kind, title) in [("post", "Đà Lạt mùa hoa"), ("page", "Đà Lạt giới thiệu")]
        {
            let created = send(
                &app,
                "POST",
                "/entries",
                Some("admin"),
                &[],
                Some(json!({"kind": kind, "title": title})),
            )
            .await;
            assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        }
        // Trần của nguồn do testkit đặt rộng (`AUTOXIDE_SOURCE_TIMEOUT_MS`): máy bận không đẩy nguồn
        // `content` vào `incomplete`.
        let find = async |q: &str| {
            let uri = format!(
                "/api/v1/kernel/find?q={}",
                q.replace(' ', "%20").replace(':', "%3A").replace('>', "%3E")
            );
            app.get(&uri).token("reader").send().await
        };
        let titles = |body: &Value| -> Vec<String> {
            body["data"]["groups"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|group| {
                    group["items"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|item| item["title"].as_str().unwrap().to_owned())
                })
                .collect()
        };
        let pages = find("da lat kind:page").await.unwrap().json();
        assert_eq!(titles(&pages), ["Đà Lạt giới thiệu"], "{pages}");
        let not_pages = find("is:entry -kind:page status:draft").await.unwrap().json();
        assert_eq!(titles(&not_pages), ["Đà Lạt mùa hoa"], "{not_pages}");
        let future = find("da lat updated>2999-01-01").await.unwrap().json();
        assert!(titles(&future).is_empty(), "{future}");
        let odd = find("da lat colour:red").await.unwrap().json();
        assert_eq!(odd["data"]["ignored"], json!(["colour:red"]));
        assert_eq!(titles(&odd).len(), 2);

        let elsewhere = app
            .get(&format!("/api/v1/kernel/find?q=da%20lat&scope=node&org_id={}", Uuid::now_v7()))
            .token("reader")
            .send()
            .await
            .unwrap()
            .json();
        assert!(titles(&elsewhere).is_empty(), "nút khác không có bài: {elsewhere}");
        let no_unit = app
            .get("/api/v1/kernel/find?q=da%20lat&scope=subtree")
            .token("reader")
            .send()
            .await
            .unwrap();
        assert_eq!(no_unit.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let resolved = app
            .get("/api/v1/kernel/resolve?code=%2Fblog%2Fda-lat-mua-hoa")
            .token("reader")
            .send()
            .await
            .unwrap();
        assert_eq!(resolved.status(), StatusCode::OK, "{}", resolved.json());
        assert_eq!(
            (resolved.json()["data"]["kind"].as_str(), resolved.json()["data"]["title"].as_str()),
            (Some("content.entry"), Some("Đà Lạt mùa hoa"))
        );
        let missing = app
            .get("/api/v1/kernel/resolve?code=nothing-here")
            .token("reader")
            .send()
            .await
            .unwrap();
        assert_eq!(
            (missing.status(), missing.json()["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("kernel.code_not_found"))
        );
    })
    .await
    .unwrap();
}
