//! Bộ giao diện của website (bước 67): token theo nút, kế thừa (gốc → cửa hàng), `If-Match`, mọi
//! lỗi một lần, logo phải là ảnh công khai, cảnh báo tương phản trên token đã ghép, quyền theo nút.
//! Gate chạy trên cả SQLite và Postgres.

use super::*;
use autoxide_kernel::access::Scope;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree};

#[tokio::test]
async fn theme_ke_thua_theo_nut_va_kiem_token() {
    TestDatabase::run(async |test| {
        let (chain, shop, other) = (
            OrgId::from_uuid(Uuid::now_v7()),
            OrgId::from_uuid(Uuid::now_v7()),
            OrgId::from_uuid(Uuid::now_v7()),
        );
        let fake = FakeMedia::default();
        let (logo, private) = (fake.add(true), fake.add(false));
        let media: Arc<dyn Media> = Arc::new(fake);
        let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
        let app = TestApp::builder(test)
            .user("owner", 1, &["content.themes.manage"])
            .member("manager", 2, shop, &[("content.themes.manage", Scope::Unit)])
            .user("reader", 3, &["content.read"])
            .provide::<dyn Media>(media)
            .provide::<dyn TextModel>(text)
            .provide::<dyn Embedder>(embedder)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let write = app.database().begin_write().await.unwrap();
        let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, chain, root, 8).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, shop, chain, 8).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, other, root, 8).await.unwrap();
        write.commit().await.unwrap();
        let uri = |org: OrgId| format!("/themes/{}", org.as_uuid());

        // Chưa đặt gì: token mặc định, không cảnh báo.
        let fresh = send(&app, "GET", &uri(shop), Some("owner"), &[], None).await;
        assert_eq!(fresh.status(), StatusCode::OK, "{}", fresh.json());
        assert_eq!(fresh.json()["data"]["tokens"], json!({}));
        assert_eq!(fresh.json()["data"]["effective"]["radius"], "medium");
        assert_eq!(fresh.json()["data"]["warnings"], json!([]));

        // Chuỗi đặt màu và font; cửa hàng chỉ đổi màu chính.
        let etag = send(&app, "GET", &uri(chain), Some("owner"), &[], None).await.header(header::ETAG).to_owned();
        let body = json!({"tokens": {"color.primary": "#B91C1C", "font.heading": "playfair display", "logo_media_id": logo}});
        let missing = send(&app, "PUT", &uri(chain), Some("owner"), &[], Some(body.clone())).await;
        assert_eq!(missing.status(), StatusCode::PRECONDITION_REQUIRED);
        let saved = send(&app, "PUT", &uri(chain), Some("owner"), &[("if-match", &etag)], Some(body.clone())).await;
        assert_eq!(saved.status(), StatusCode::OK, "{}", saved.json());
        assert_eq!(saved.json()["data"]["tokens"]["color.primary"], "#b91c1c");
        assert_eq!(saved.json()["data"]["tokens"]["font.heading"], "Playfair Display");
        let stale = send(&app, "PUT", &uri(chain), Some("owner"), &[("if-match", &etag)], Some(body)).await;
        assert_eq!(stale.status(), StatusCode::PRECONDITION_FAILED);

        let shop_put = json!({"tokens": {"color.primary": "#065f46"}});
        let saved = send(&app, "PUT", &uri(shop), Some("manager"), &[("if-match", "*")], Some(shop_put)).await;
        assert_eq!(saved.status(), StatusCode::OK, "{}", saved.json());
        let effective = &saved.json()["data"]["effective"];
        assert_eq!(effective["color.primary"], "#065f46", "the shop overrides the chain");
        assert_eq!(effective["font.heading"], "Playfair Display", "inherited from the chain");
        assert_eq!(effective["logo_media_id"], logo.to_string());
        // Website khác của khách không thấy token của chuỗi.
        let elsewhere = send(&app, "GET", &uri(other), Some("owner"), &[], None).await.json();
        assert_eq!(elsewhere["data"]["effective"]["color.primary"], "#1d4ed8");

        // Mọi lỗi một lần; logo không công khai bị từ chối.
        let bad = json!({"tokens": {"color.text": "red", "font.body": "Comic Sans", "spacing": "huge", "glow": 1}});
        let refused = send(&app, "PUT", &uri(shop), Some("owner"), &[("if-match", "*")], Some(bad)).await;
        assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY, "{}", refused.json());
        assert_eq!(refused.json()["errors"].as_array().unwrap().len(), 4, "{}", refused.json());
        let hidden = json!({"tokens": {"logo_media_id": private}});
        let refused = send(&app, "PUT", &uri(shop), Some("owner"), &[("if-match", "*")], Some(hidden)).await;
        assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(refused.json()["errors"][0]["field"], "tokens.logo_media_id");

        // Tương phản kém chỉ cảnh báo, kiểm trên token đã ghép (chữ của cửa hàng trên nền của chuỗi).
        let low = json!({"tokens": {"color.background": "#eeeeee"}});
        send(&app, "PUT", &uri(chain), Some("owner"), &[("if-match", "*")], Some(low)).await;
        let grey = json!({"tokens": {"color.primary": "#065f46", "color.text": "#aaaaaa"}});
        let saved = send(&app, "PUT", &uri(shop), Some("manager"), &[("if-match", "*")], Some(grey)).await;
        assert_eq!(saved.status(), StatusCode::OK, "{}", saved.json());
        let warnings = saved.json()["data"]["warnings"].clone();
        assert_eq!(warnings[0]["foreground"], "color.text", "{warnings}");
        assert_eq!(warnings[0]["background"], "color.background");

        // Quyền theo nút: người của cửa hàng không đổi được chuỗi; người chỉ đọc bài không vào được.
        let out = send(&app, "PUT", &uri(chain), Some("manager"), &[("if-match", "*")], Some(json!({"tokens": {}}))).await;
        assert_eq!(out.status(), StatusCode::FORBIDDEN);
        assert_eq!(send(&app, "GET", &uri(shop), Some("reader"), &[], None).await.status(), StatusCode::FORBIDDEN);
        let none = send(&app, "GET", &format!("/themes/{}", Uuid::now_v7()), Some("owner"), &[], None).await;
        assert_eq!(none.status(), StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}
