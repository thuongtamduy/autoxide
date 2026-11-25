//! Link trong thư theo từng khách (D-20261125-tenant-link-templates): khách đặt trang riêng có đường dẫn tuỳ
//! ý, chỉ trên host khách sở hữu; khách A đặt không ảnh hưởng khách B; khách B không mượn được tên miền của A.

use autoxide_kernel::module::Module;
use autoxide_kernel::tenancy::{SiteLinks, TenantDomains, TenantSetting};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::{Value, json};

const PAGE: TenantSetting = TenantSetting::link(
    "linker.link.page",
    "Page of this tenant's site, with `{token}`. When unset, the server's AUTOXIDE_LINKER_URL is used.",
    &["{token}"],
);

/// Mẫu link của máy chủ (vai biến `AUTOXIDE_…_URL`).
const SERVER: &str = "https://www.example.com/confirm?token={token}";

struct Linker;

impl Module for Linker {
    fn name(&self) -> &'static str {
        "linker"
    }

    fn tenant_settings(&self) -> Vec<TenantSetting> {
        vec![PAGE]
    }
}

async fn app(test: &TestDatabase) -> TestApp {
    TestApp::builder(test)
        .tenant("a")
        .tenant("b")
        .domain("a", "shop-a.test")
        .user_in("a", "owner-a", 1, &["kernel.settings.manage"])
        .user_in("b", "owner-b", 2, &["kernel.settings.manage"])
        .module(Linker)
        .build()
        .await
        .unwrap()
}

/// Đặt `name` = `value` cho khách của `token`; trả mã HTTP và thân.
async fn put(app: &TestApp, token: &str, name: &str, value: &str) -> (StatusCode, Value) {
    let path = format!("/api/v1/kernel/settings/{name}");
    let response = app.put(&path).token(token).json(json!({"value": value})).send().await.unwrap();
    (response.status(), response.json())
}

fn codes(body: &Value) -> Vec<String> {
    body["errors"]
        .as_array()
        .map(|errors| errors.iter().map(|e| e["code"].as_str().unwrap_or("").to_owned()).collect())
        .unwrap_or_default()
}

#[tokio::test]
async fn link_rieng_chi_tren_host_cua_khach() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let name = PAGE.name();
        for (value, code) in [
            ("https://evil.test/reset?t={token}", "kernel.link_host_not_allowed"),
            ("https://shop-a.test/reset", "kernel.link_placeholder_missing"),
            ("http://shop-a.test/reset?t={token}", "kernel.link_template_invalid"),
            ("https://shop-a.test@evil.test/{token}", "kernel.link_template_invalid"),
            ("https://{token}.shop-a.test/", "kernel.link_template_invalid"),
            ("https://shop-a.test.evil.test/{token}", "kernel.link_host_not_allowed"),
        ] {
            let (status, body) = put(&app, "owner-a", name, value).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{value}: {body}");
            assert_eq!(codes(&body), [code], "{value}: {body}");
        }
        // Tên miền đã xác minh của chính khách: nhận, đường dẫn tuỳ ý.
        let own = "https://shop-a.test/dat-lai-mat-khau?t={token}";
        let (status, body) = put(&app, "owner-a", name, own).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        // Khách B không mượn được tên miền của A (mạo danh / lấy token của người dùng B qua trang của A).
        let (status, body) = put(&app, "owner-b", name, own).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(codes(&body), ["kernel.link_host_not_allowed"]);

        let links = app.api::<SiteLinks>().unwrap();
        let a = app.tenant("a").await.unwrap();
        let b = app.tenant("b").await.unwrap();
        assert_eq!(links.link(&a, &PAGE, Some(SERVER)).await.unwrap().as_deref(), Some(own));
        // B chưa đặt gì: link của máy chủ như cũ.
        assert_eq!(links.link(&b, &PAGE, Some(SERVER)).await.unwrap().as_deref(), Some(SERVER));
        // Máy chủ không đặt biến: A vẫn có link của mình, B không có link (tính năng tắt với B).
        assert_eq!(links.link(&a, &PAGE, None).await.unwrap().as_deref(), Some(own));
        assert_eq!(links.link(&b, &PAGE, None).await.unwrap(), None);

        // B đặt `kernel.public_url` rồi trỏ link về host đó: nhận; link máy chủ đổi host theo B.
        let (status, body) = put(&app, "owner-b", "kernel.public_url", "https://b.example").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            links.link(&b, &PAGE, Some(SERVER)).await.unwrap().as_deref(),
            Some("https://b.example/confirm?token={token}")
        );
        let page_b = "https://b.example/xac-nhan/{token}";
        let (status, body) = put(&app, "owner-b", name, page_b).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(links.link(&b, &PAGE, Some(SERVER)).await.unwrap().as_deref(), Some(page_b));
        assert_eq!(links.link(&a, &PAGE, Some(SERVER)).await.unwrap().as_deref(), Some(own));

        let (status, body) = put(&app, "owner-b", "kernel.public_url", "https://b2.example").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        // Giá trị cũ của B giờ trỏ host B không còn khai: không dùng nữa, quay về link máy chủ (đổi host).
        assert_eq!(
            links.link(&b, &PAGE, Some(SERVER)).await.unwrap().as_deref(),
            Some("https://b2.example/confirm?token={token}")
        );
    })
    .await
    .unwrap();
}

/// Kịch bản rà soát #516: (a) tên miền của A bị thu hồi rồi khách C xác minh lại → link và `kernel.public_url`
/// của A không còn trỏ tới đó (token của người dùng A không tới tay C); (b) B đặt `kernel.public_url` bằng tên
/// miền đã xác minh của A → không mượn được; (c) host đúng mà port khác → không nhận.
#[tokio::test]
async fn ten_mien_cua_khach_khac_khong_bao_gio_duoc_tin() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .tenant("a")
            .tenant("b")
            .tenant("c")
            .domain("a", "shop-a.test")
            .user_in("a", "owner-a", 1, &["kernel.settings.manage"])
            .user_in("b", "owner-b", 2, &["kernel.settings.manage"])
            .module(Linker)
            .build()
            .await
            .unwrap();
        let links = app.api::<SiteLinks>().unwrap();
        let (a, b, c) = (
            app.tenant("a").await.unwrap(),
            app.tenant("b").await.unwrap(),
            app.tenant("c").await.unwrap(),
        );
        let name = PAGE.name();

        // (b) B trỏ `kernel.public_url` vào tên miền của A: link mẫu trên đó bị từ chối, link máy chủ không đổi host.
        let (status, body) = put(&app, "owner-b", "kernel.public_url", "https://shop-a.test").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(codes(&body), ["kernel.public_url_not_allowed"]);
        let (status, body) =
            put(&app, "owner-a", "kernel.public_url", "https://shop-a.test:8443").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "port: {body}");
        let (status, body) = put(&app, "owner-b", name, "https://shop-a.test/r?t={token}").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(codes(&body), ["kernel.link_host_not_allowed"]);
        assert_eq!(links.link(&b, &PAGE, Some(SERVER)).await.unwrap().as_deref(), Some(SERVER));

        // (c) Port khác trên tên miền của chính A: không nhận.
        let (status, body) =
            put(&app, "owner-a", name, "https://shop-a.test:9090/r?t={token}").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(codes(&body), ["kernel.link_host_not_allowed"]);

        // (a) A đặt `kernel.public_url` + link mẫu trên tên miền của mình; rồi tên miền chuyển sang C.
        let (status, body) = put(&app, "owner-a", "kernel.public_url", "https://shop-a.test").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let own = "https://shop-a.test/dat-lai?t={token}";
        let (status, body) = put(&app, "owner-a", name, own).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(links.link(&a, &PAGE, Some(SERVER)).await.unwrap().as_deref(), Some(own));

        let domains = app.api::<TenantDomains>().unwrap();
        let system = autoxide_kernel::event::Actor::system;
        let page = autoxide_kernel::pagination::PageRequest::default();
        let listed = domains.list(&a, &page).await.unwrap().data;
        domains.revoke(system(), &a, listed[0].id).await.unwrap();
        let added = domains.add(system(), &c, "shop-a.test").await.unwrap();
        app.dns().set_txt(&added.record.name, &[&added.record.value]);
        assert_eq!(domains.verify(system(), &c, added.id).await.unwrap().status, "verified");

        // Thư đặt lại mật khẩu của người dùng A không còn tới `shop-a.test` (của C): về link của máy chủ.
        assert_eq!(links.link(&a, &PAGE, Some(SERVER)).await.unwrap().as_deref(), Some(SERVER));
        assert_eq!(links.rewrite(&a, SERVER).await.unwrap(), SERVER);
    })
    .await
    .unwrap();
}

/// Đặt trạng thái của mọi dòng tên miền `domain` (giả lập DNS hỏng sau lần kiểm lại hằng ngày).
async fn set_domain_status(app: &TestApp, domain: &str, status: &str) {
    use sea_orm::ConnectionTrait;
    let database = app.registry().control();
    let backend = database.write().get_database_backend();
    let sql = match backend {
        sea_orm::DbBackend::Postgres => {
            "UPDATE kernel_tenant_domains SET status = $1 WHERE domain = $2"
        }
        _ => "UPDATE kernel_tenant_domains SET status = ? WHERE domain = ?",
    };
    let statement =
        autoxide_kernel::database::statement(backend, sql, [status.into(), domain.into()]);
    database.write().execute_raw(statement).await.unwrap();
}

/// Rà soát #516 lần hai: tên miền khách A đã mất (thu hồi, hay hỏng DNS — có thể người khác đã mua lại) mà CHƯA
/// ai xác minh lại: link mẫu, `kernel.public_url` và đường đổi host không còn trỏ tới đó; đặt lại bị 422.
#[tokio::test]
async fn ten_mien_da_mat_khong_con_duoc_tin() {
    for lost in ["revoked", "failed"] {
        TestDatabase::run(async |test| {
            let app = TestApp::builder(test)
                .tenant("a")
                .domain("a", "shop-a.test")
                .user_in("a", "owner-a", 1, &["kernel.settings.manage"])
                .module(Linker)
                .build()
                .await
                .unwrap();
            let links = app.api::<SiteLinks>().unwrap();
            let a = app.tenant("a").await.unwrap();
            let (status, body) =
                put(&app, "owner-a", "kernel.public_url", "https://shop-a.test").await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let own = "https://shop-a.test/dat-lai?t={token}";
            let (status, body) = put(&app, "owner-a", PAGE.name(), own).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(links.link(&a, &PAGE, Some(SERVER)).await.unwrap().as_deref(), Some(own));

            set_domain_status(&app, "shop-a.test", lost).await;
            // Kiểm lại DNS thật xoá cache tên miền của máy này; test đổi thẳng dòng nên tự xoá.
            app.registry().invalidate_domain("shop-a.test");
            assert_eq!(
                links.link(&a, &PAGE, Some(SERVER)).await.unwrap().as_deref(),
                Some(SERVER),
                "{lost}"
            );
            assert_eq!(links.rewrite(&a, SERVER).await.unwrap(), SERVER, "{lost}");
            let (status, body) =
                put(&app, "owner-a", "kernel.public_url", "https://shop-a.test").await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{lost}: {body}");
            assert_eq!(codes(&body), ["kernel.public_url_not_allowed"]);
        })
        .await
        .unwrap();
    }
}
