//! Trang mẫu (bước 67): cài bộ mẫu dựng sẵn (chạy lại không trùng), lọc theo ngành, tạo trang nháp
//! từ mẫu bỏ khối của module khách không có, lưu trang thành mẫu, xoá. Gate chạy trên cả SQLite và
//! Postgres.

use super::*;

#[tokio::test]
async fn cai_mau_tao_trang_tu_mau_bo_khoi_module_tat() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let installed =
            send(&app, "POST", "/page-templates/presets", Some("admin"), &[], None).await;
        assert_eq!(installed.status(), StatusCode::OK, "{}", installed.json());
        // 5 ngành × 2 ngôn ngữ mặc định (`en`, `vi`).
        assert_eq!(installed.json()["data"].as_array().unwrap().len(), 10);
        let again = send(&app, "POST", "/page-templates/presets", Some("admin"), &[], None).await;
        assert_eq!(again.json()["data"], json!([]), "installing twice adds nothing");
        assert_eq!(
            send(&app, "POST", "/page-templates/presets", Some("reader"), &[], None).await.status(),
            StatusCode::FORBIDDEN
        );

        let listed = send(
            &app,
            "GET",
            "/page-templates?industry=services&locale=vi",
            Some("reader"),
            &[],
            None,
        )
        .await;
        assert_eq!(listed.status(), StatusCode::OK, "{}", listed.json());
        let templates = listed.json()["data"].as_array().unwrap().clone();
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0]["name"], "Dịch vụ: nhận báo giá");
        let template = templates[0]["id"].as_str().unwrap().to_owned();

        // Mọi mẫu dựng sẵn tạo được trang hợp lệ (khối của `content` qua bộ kiểm thật).
        for (index, row) in installed.json()["data"].as_array().unwrap().iter().enumerate() {
            let body = json!({"template_id": row["id"], "title": format!("Mẫu {index}")});
            let made =
                send(&app, "POST", "/entries/from-template", Some("admin"), &[], Some(body)).await;
            assert_eq!(
                made.status(),
                StatusCode::CREATED,
                "{}: {}",
                row["preset_key"],
                made.json()
            );
        }

        // Module `forms` không có trong app: khối form bị bỏ, trang vẫn là nháp hợp lệ.
        let body = json!({"template_id": template, "title": "Báo giá sửa nhà"});
        let made =
            send(&app, "POST", "/entries/from-template", Some("admin"), &[], Some(body)).await;
        assert_eq!(made.status(), StatusCode::CREATED, "{}", made.json());
        let data = made.json()["data"].clone();
        assert_eq!(data["dropped_blocks"], json!(["forms.embed"]));
        assert_eq!(data["entry"]["status"], "draft");
        assert_eq!(data["entry"]["locale"], "vi");
        assert!(!data["entry"]["body"].to_string().contains("forms.embed"));
        let page = data["entry"]["id"].as_str().unwrap().to_owned();

        // Lưu trang thành mẫu riêng; trang thân `rich` thì không.
        let saved = send(
            &app,
            "POST",
            "/page-templates",
            Some("admin"),
            &[],
            Some(json!({"entry_id": page, "name": "Báo giá của tôi", "industry": "services"})),
        )
        .await;
        assert_eq!(saved.status(), StatusCode::CREATED, "{}", saved.json());
        assert_eq!(saved.json()["data"]["preset_key"], Value::Null);
        let own = saved.json()["data"]["id"].as_str().unwrap().to_owned();
        let post = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(json!({"kind": "post", "title": "Tin"})),
        )
        .await;
        let post = post.json()["data"]["id"].as_str().unwrap().to_owned();
        let refused = send(
            &app,
            "POST",
            "/page-templates",
            Some("admin"),
            &[],
            Some(json!({"entry_id": post, "name": "", "industry": "cars"})),
        )
        .await;
        assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(refused.json()["errors"].as_array().unwrap().len(), 3, "{}", refused.json());

        assert_eq!(
            send(&app, "DELETE", &format!("/page-templates/{own}"), Some("admin"), &[], None)
                .await
                .status(),
            StatusCode::NO_CONTENT
        );
        let gone = send(
            &app,
            "POST",
            "/entries/from-template",
            Some("admin"),
            &[],
            Some(json!({"template_id": own, "title": "x"})),
        )
        .await;
        assert_eq!(
            (gone.status(), gone.json()["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("content.page_template_not_found"))
        );
    })
    .await
    .unwrap();
}
