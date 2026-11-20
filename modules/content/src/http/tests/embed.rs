//! Nhúng đối tượng của module khác (module `shop` giả) và tìm xuyên phân hệ.

use super::*;

/// Module giả cung cấp `shop.item`: ID bắt đầu bằng `p` là công khai, `h` là đang ẩn, còn lại
/// không có.
struct Shop;

struct ShopItems;

#[async_trait::async_trait]
impl autoxide_kernel::resource::ResourceResolver for ShopItems {
    async fn resolve(
        &self,
        _: &TenantContext,
        id: &str,
    ) -> Result<Option<autoxide_kernel::resource::Resource>, autoxide_kernel::error::Error> {
        Ok((id.starts_with('p') || id.starts_with('h')).then(|| {
            let public = id.starts_with('p');
            autoxide_kernel::resource::Resource::new(if public {
                format!("Item {id}")
            } else {
                String::new()
            })
            .public(public)
            .summary(format!("About {id}"))
            .url(Some(format!("/items/{id}")))
        }))
    }
}

impl autoxide_kernel::module::Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn start(
        &mut self,
        context: &mut autoxide_kernel::module::StartContext<'_>,
    ) -> Result<(), autoxide_kernel::error::Error> {
        context.resource_type("shop.item", Arc::new(ShopItems))
    }
}

/// Bước 51 P6: node `embed` dạng `{kind, id}` — loại chưa đăng ký hay bản ghi không có bị từ
/// chối khi lưu; đọc công khai chỉ trả bản ghi còn công khai.
#[tokio::test]
async fn embed_kind_chua_dang_ky_bi_tu_choi_va_bo_doi_tuong_khong_con_cong_khai() {
    TestDatabase::run(async |test| {
        let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
        let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
        let app = TestApp::builder(test)
            .user("admin", 1, &["content.read", "content.write", "content.publish"])
            .provide::<dyn Media>(media)
            .provide::<dyn TextModel>(text)
            .provide::<dyn Embedder>(embedder)
            .module(Shop)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let create = |body: Value| json!({"kind": "post", "title": "Nhúng sản phẩm", "body": body});

        let unknown = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(create(embed_body(&[("nope.item", "1")]))),
        )
        .await;
        assert_eq!(unknown.status(), StatusCode::UNPROCESSABLE_ENTITY, "{}", unknown.json());
        assert_eq!(unknown.json()["errors"][0]["code"], "embed_kind_unknown");
        let missing = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(create(embed_body(&[("shop.item", "x1")]))),
        )
        .await;
        assert_eq!(missing.json()["errors"][0]["code"], "embed_not_found", "{}", missing.json());
        let bad = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(create(embed_body(&[("Shop Item", "")]))),
        )
        .await;
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let body = embed_body(&[("shop.item", "p1"), ("shop.item", "h1")]);
        let created = send(&app, "POST", "/entries", Some("admin"), &[], Some(create(body))).await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let read = send(&app, "GET", &format!("/entries/{id}"), Some("admin"), &[], None).await;
        let etag = read.header(header::ETAG).to_owned();
        let published = send(
            &app,
            "POST",
            &format!("/entries/{id}/publish"),
            Some("admin"),
            &[("if-match", &etag)],
            None,
        )
        .await;
        assert_eq!(published.status(), StatusCode::OK, "{}", published.json());

        let public = send(
            &app,
            "GET",
            "/public/resolve?locale=en&path=/blog/nhung-san-pham",
            None,
            &[],
            None,
        )
        .await;
        assert_eq!(public.status(), StatusCode::OK, "{}", public.json());
        let embeds = public.json()["data"]["entry"]["embeds"].clone();
        assert_eq!(embeds.as_object().unwrap().len(), 1, "chỉ bản ghi còn công khai: {embeds}");
        assert_eq!(embeds["shop.item:p1"]["title"], "Item p1");
        assert_eq!(embeds["shop.item:p1"]["url"], "/items/p1");
    })
    .await
    .unwrap();
}

/// Bước 51 P3: ô tìm Ctrl K (`GET /kernel/find`) tìm bài không dấu, chỉ trong loại người gọi được
/// xem; từ tìm quá ngắn bị từ chối.
#[tokio::test]
async fn tim_xuyen_phan_he_tim_bai_khong_dau() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let created = send(
            &app,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(json!({"kind": "post", "title": "Đường đến Đà Lạt"})),
        )
        .await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();

        let found = app.get("/api/v1/kernel/find?q=da%20lat").token("reader").send().await.unwrap();
        assert_eq!(found.status(), StatusCode::OK, "{}", found.json());
        let groups = found.json()["data"]["groups"].clone();
        assert_eq!(groups[0]["kind"], "content.entry", "{groups}");
        assert_eq!(groups[0]["items"][0]["id"], id.as_str());
        assert_eq!(groups[0]["items"][0]["title"], "Đường đến Đà Lạt");
        assert_eq!(groups[0]["items"][0]["screen"], "content.entries");
        let nothing =
            app.get("/api/v1/kernel/find?q=hanoi").token("reader").send().await.unwrap().json();
        assert_eq!(nothing["data"]["groups"], json!([]));
        let short = app.get("/api/v1/kernel/find?q=d").token("reader").send().await.unwrap();
        assert_eq!(short.status(), StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}
