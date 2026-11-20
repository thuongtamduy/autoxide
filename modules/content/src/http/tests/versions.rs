//! Phiên bản kèm tên người sửa (module `auth` giả) và tìm chuyển hướng theo `q`.

use super::*;

/// Module `auth` giả: chỉ đăng ký loại `auth.user` vào sổ đối tượng; người số 1 tên "Ada", người
/// khác không có (như tài khoản đã xoá).
struct FakeAuth;

struct Names;

#[async_trait::async_trait]
impl autoxide_kernel::resource::ResourceResolver for Names {
    async fn resolve(
        &self,
        _: &TenantContext,
        id: &str,
    ) -> Result<Option<autoxide_kernel::resource::Resource>, autoxide_kernel::error::Error> {
        let ada = Uuid::from_u128(1).to_string();
        Ok((id == ada).then(|| autoxide_kernel::resource::Resource::new("Ada")))
    }
}

impl autoxide_kernel::module::Module for FakeAuth {
    fn name(&self) -> &'static str {
        "auth"
    }

    fn start(
        &mut self,
        context: &mut autoxide_kernel::module::StartContext<'_>,
    ) -> Result<(), autoxide_kernel::error::Error> {
        context.resource_type("auth.user", Arc::new(Names))
    }
}

/// Đề bài giao diện (lịch sử phiên bản): mỗi phiên bản kèm tên người sửa, tra theo lô qua sổ đối
/// tượng; chương trình không có module tài khoản thì tên là `null`.
#[tokio::test]
async fn phien_ban_kem_ten_nguoi_sua() {
    TestDatabase::run(async |test| {
        let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
        let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
        let named = TestApp::builder(test)
            .user("admin", 1, &["content.read", "content.write"])
            .provide::<dyn Media>(media)
            .provide::<dyn TextModel>(text)
            .provide::<dyn Embedder>(embedder)
            .module(FakeAuth)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let body = json!({"kind": "post", "title": "Phiên bản"});
        let created = send(&named, "POST", "/entries", Some("admin"), &[], Some(body)).await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let list =
            send(&named, "GET", &format!("/entries/{id}/revisions"), Some("admin"), &[], None)
                .await;
        assert_eq!(list.status(), StatusCode::OK, "{}", list.json());
        let first = &list.json()["data"][0];
        assert_eq!(first["created_by_name"], "Ada", "{first}");
        let revision = first["id"].as_str().unwrap().to_owned();
        let one = send(
            &named,
            "GET",
            &format!("/entries/{id}/revisions/{revision}"),
            Some("admin"),
            &[],
            None,
        )
        .await;
        assert_eq!(one.json()["data"]["created_by_name"], "Ada");

        // Không có module tài khoản: tên trống, không lỗi.
        let plain = app(test).await;
        let body = json!({"kind": "post", "title": "Không tên"});
        let created = send(&plain, "POST", "/entries", Some("admin"), &[], Some(body)).await;
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let list =
            send(&plain, "GET", &format!("/entries/{id}/revisions"), Some("admin"), &[], None)
                .await;
        assert_eq!(list.json()["data"][0]["created_by_name"], Value::Null);
    })
    .await
    .unwrap();
}

/// Đề bài giao diện W-059: `q` của danh sách chuyển hướng tìm trong đường dẫn cũ và địa chỉ đích,
/// không phân biệt hoa thường, có dấu hay không.
#[tokio::test]
async fn tim_chuyen_huong_theo_q() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let body = json!({"locale": "en", "from_path": "/khuyến-mãi-cũ", "to_url": "https://Shop.example.com/Sale", "status_code": 301});
        let created = send(&app, "POST", "/redirects", Some("admin"), &[], Some(body)).await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let body = json!({"locale": "en", "from_path": "/about-us", "to_url": "/about", "status_code": 301});
        let created = send(&app, "POST", "/redirects", Some("admin"), &[], Some(body)).await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());

        let found = async |q: &str| {
            let list = send(&app, "GET", &format!("/redirects?q={q}"), Some("admin"), &[], None).await;
            assert_eq!(list.status(), StatusCode::OK, "{}", list.json());
            list.json()["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["from_path"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(found("khuyen%20mai").await.len(), 0, "khoảng trắng không phải dấu gạch");
        assert_eq!(found("khuyen-mai").await, ["/khuyến-mãi-cũ"]);
        assert_eq!(found("SHOP.EXAMPLE").await, ["/khuyến-mãi-cũ"], "tìm cả địa chỉ đích");
        assert_eq!(found("about").await, ["/about-us"]);
        assert_eq!(found("50%25").await.len(), 0, "`%` là chữ thường");
        assert_eq!(found("").await.len(), 2);
    })
    .await
    .unwrap();
}
