//! Khung website `GET /public/site` (bước 67, D-234): đầu trang nháp chưa đăng không hiện, cửa hàng đè
//! đầu trang của chuỗi, thiếu bản dịch dùng ngôn ngữ mặc định, bản chủ đề kèm đầu trang riêng, chân trang
//! pháp lý luôn dưới footer (ngoài bố cục), phần của module khác qua `SiteExtensions`, cache nóng không
//! chạm database, `max-age` bằng số giây tới mốc đổi, 304; đo p99 khi cache nóng / nguội với 50 bản chủ
//! đề. Gate chạy trên cả SQLite và Postgres.

use std::time::{Duration, Instant};

use super::*;
use crate::api::{SiteExtension, SiteExtensions};
use autoxide_kernel::access::Scope;
use autoxide_kernel::blocks::{BlockCategory, BlockData, BlockRef, BlockRequest, BlockType};
use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree};
use autoxide_kernel::time::Timestamp;

/// `compliance` giả: chân trang pháp lý và phần khung website (cấu hình đồng ý cookie).
struct Compliance;

struct Footer;

#[async_trait::async_trait]
impl BlockData for Footer {
    async fn resolve(
        &self,
        _: &BlockRequest<'_>,
        _: &str,
        items: &[BlockRef],
    ) -> Result<Vec<Option<Value>>, Error> {
        Ok(items.iter().map(|_| Some(json!({"owner": {"legal_name": "Công ty A"}}))).collect())
    }
}

struct Cookies;

#[async_trait::async_trait]
impl SiteExtension for Cookies {
    async fn site(&self, request: &BlockRequest<'_>) -> Result<Option<Value>, Error> {
        Ok(Some(json!({"cookie_banner": true, "locale": request.locale})))
    }
}

impl Module for Compliance {
    fn name(&self) -> &'static str {
        "compliance"
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", "{}")]
    }

    fn blocks(&self) -> Vec<BlockType> {
        // Nhãn mượn câu đã dịch của `content.section` (`cargo xtask check` đòi bản dịch).
        let footer = BlockType::new(
            "compliance.legal_footer",
            BlockCategory::Legal,
            "Section",
            "Use to group blocks into a band of the page with its own background and spacing.",
            r#"{"type": "object", "additionalProperties": false, "properties": {}}"#,
        );
        vec![footer.with_data()]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.block_data("compliance.legal_footer", Arc::new(Footer))?;
        // Đăng ký SAU `content`: tự cắm phần của mình vào khung website.
        context.require::<dyn SiteExtensions>()?.register("compliance", Arc::new(Cookies))
    }
}

fn layout(text: &str) -> Value {
    json!({"layout": {"type": "layout", "schema": 1, "sections": [
        {"id": "title", "block": "content.heading", "props": {"text": text}}
    ]}})
}

async fn site_app(test: &TestDatabase, shop: OrgId) -> TestApp {
    let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
    let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
    TestApp::builder(test)
        .user("owner", 1, &["content.themes.manage"])
        .member("shop", 2, shop, &[("content.themes.manage", Scope::Unit)])
        .provide::<dyn Media>(media)
        .provide::<dyn TextModel>(text)
        .provide::<dyn Embedder>(embedder)
        .module(crate::module())
        .module(Compliance)
        .build()
        .await
        .unwrap()
}

async fn put_region(app: &TestApp, who: &str, query: &str, body: Value) -> Value {
    let saved = send(
        app,
        "PUT",
        &format!("/site-regions?{query}"),
        Some(who),
        &[("if-match", "*")],
        Some(body),
    )
    .await;
    assert_eq!(saved.status(), StatusCode::OK, "{}", saved.json());
    saved.json()["data"].clone()
}

async fn publish(app: &TestApp, who: &str, id: &str) {
    let done = send(
        app,
        "POST",
        &format!("/site-regions/{id}/publish"),
        Some(who),
        &[("if-match", "*")],
        None,
    )
    .await;
    assert_eq!(done.status(), StatusCode::OK, "{}", done.json());
}

async fn frame(app: &TestApp, query: &str) -> (Value, autoxide_testkit::TestResponse) {
    let answer = send(app, "GET", &format!("/public/site{query}"), None, &[], None).await;
    assert_eq!(answer.status(), StatusCode::OK, "{}", answer.text());
    (answer.json()["data"].clone(), answer)
}

#[tokio::test]
async fn khung_website_dau_chan_trang_chu_de_va_cache() {
    TestDatabase::run(async |test| {
        let (chain, shop) = (OrgId::from_uuid(Uuid::now_v7()), OrgId::from_uuid(Uuid::now_v7()));
        let app = site_app(test, shop).await;
        let write = app.database().begin_write().await.unwrap();
        let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, chain, root, 8).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, shop, chain, 8).await.unwrap();
        write.commit().await.unwrap();
        let (chain_id, shop_id) = (chain.as_uuid(), shop.as_uuid());

        // Chưa có gì: token mặc định, không đầu / chân trang; chân trang pháp lý và phần của
        // `compliance` vẫn có.
        let (empty, _) = frame(&app, "").await;
        assert_eq!(empty["header"], Value::Null);
        assert_eq!(empty["tokens"]["radius"], "medium");
        assert_eq!(empty["legal_footer"]["owner"]["legal_name"], "Công ty A");
        assert_eq!(empty["extensions"]["compliance"]["cookie_banner"], true);

        // Đầu trang của chuỗi, bản nháp: chưa hiện.
        let draft = put_region(&app, "owner", &format!("org_id={chain_id}&region=header&locale=en"), layout("Chain")).await;
        let (seen, _) = frame(&app, &format!("?site={shop_id}")).await;
        assert_eq!(seen["header"], Value::Null, "a draft never shows");
        publish(&app, "owner", draft["id"].as_str().unwrap()).await;
        let (seen, _) = frame(&app, &format!("?site={shop_id}&locale=vi")).await;
        assert_eq!(seen["header"]["layout"]["sections"][0]["props"]["text"], "Chain", "inherited, and English when Vietnamese is missing");

        // Cửa hàng đè đầu trang của chuỗi (bản tiếng Việt).
        let own = put_region(&app, "shop", &format!("org_id={shop_id}&region=header&locale=vi"), layout("Shop")).await;
        publish(&app, "shop", own["id"].as_str().unwrap()).await;
        let (seen, _) = frame(&app, &format!("?site={shop_id}&locale=vi")).await;
        assert_eq!(seen["header"]["layout"]["sections"][0]["props"]["text"], "Shop");
        let (en, _) = frame(&app, &format!("?site={shop_id}&locale=en")).await;
        assert_eq!(en["header"]["layout"]["sections"][0]["props"]["text"], "Chain", "the shop has no English header");
        let (chain_view, _) = frame(&app, &format!("?site={chain_id}&locale=vi")).await;
        assert_eq!(chain_view["header"]["layout"]["sections"][0]["props"]["text"], "Chain");
        // Người của cửa hàng không sửa được chuỗi.
        let out = send(&app, "PUT", &format!("/site-regions?org_id={chain_id}&region=footer&locale=en"), Some("shop"), &[("if-match", "*")], Some(layout("x"))).await;
        assert_eq!(out.status(), StatusCode::FORBIDDEN);
        // Bản chủ đề đang chạy kèm đầu trang riêng; còn 100 giây nữa thì hết.
        let now = Timestamp::now();
        let ends = now.checked_add(autoxide_kernel::time::SignedDuration::from_secs(100)).unwrap();
        let variant = json!({"org_id": chain_id, "name": "Flash", "overrides": {"color.primary": "#b91c1c"}, "effect": "confetti",
            "schedule": {"kind": "once", "starts_at": now.checked_sub(autoxide_kernel::time::SignedDuration::from_secs(60)).unwrap(), "ends_at": ends}});
        let created = send(&app, "POST", "/theme-variants", Some("owner"), &[], Some(variant)).await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let variant_id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let special = put_region(&app, "owner", &format!("org_id={chain_id}&region=header&locale=vi&variant_id={variant_id}"), layout("Flash header")).await;
        publish(&app, "owner", special["id"].as_str().unwrap()).await;
        let on = send(&app, "POST", &format!("/theme-variants/{variant_id}/activate"), Some("owner"), &[("if-match", "*")], None).await;
        assert_eq!(on.status(), StatusCode::OK, "{}", on.json());
        let (seen, answer) = frame(&app, &format!("?site={shop_id}&locale=vi")).await;
        assert_eq!(seen["variant"]["id"], variant_id.as_str());
        assert_eq!(seen["effect"], "confetti");
        assert_eq!(seen["tokens"]["color.primary"], "#b91c1c");
        assert_eq!(seen["header"]["layout"]["sections"][0]["props"]["text"], "Flash header", "the variant's own header wins");
        let max_age: i64 = answer.header(header::CACHE_CONTROL).trim_start_matches("public, max-age=").parse().unwrap();
        assert!((90..=100).contains(&max_age), "max-age ends when the variant ends: {max_age}");
        assert_eq!(seen["next_change_at"], json!(ends));

        // Cache nóng: không một câu truy vấn; 304 với `If-None-Match`.
        let etag = answer.header(header::ETAG).to_owned();
        let query = format!("/public/site?site={shop_id}&locale=vi");
        let (again, statements) = app.statements(send(&app, "GET", &query, None, &[], None)).await;
        assert_eq!(again.status(), StatusCode::OK);
        // `site` tra cây tổ chức (extractor của kernel) trước khi tới `content`; phần của `content` không query.
        assert!(statements.iter().all(|sql| !sql.contains("content_")), "{statements:#?}");
        let not_modified = send(&app, "GET", &query, None, &[("if-none-match", &etag)], None).await;
        assert_eq!(not_modified.status(), StatusCode::NOT_MODIFIED);

        // Sửa theme trên máy này: khung đổi ngay, không chờ 30 giây.
        let theme = json!({"tokens": {"radius": "large"}});
        let saved = send(&app, "PUT", &format!("/themes/{chain_id}"), Some("owner"), &[("if-match", "*")], Some(theme)).await;
        assert_eq!(saved.status(), StatusCode::OK, "{}", saved.json());
        let (seen, answer) = frame(&app, &format!("?site={shop_id}&locale=vi")).await;
        assert_eq!(seen["tokens"]["radius"], "large");
        assert_ne!(answer.header(header::ETAG), etag);

        let unsupported = send(&app, "GET", "/public/site?locale=fr", None, &[], None).await;
        assert_eq!(unsupported.status(), StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

/// Đo p99 của `GET /public/site` khi cache nóng và nguội, website có 50 bản chủ đề (ghi vào
/// D-20261122-public-site; chỉ chặn p99 dưới 2 giây, không chặn theo máy).
#[tokio::test]
async fn public_site_p99_nong_va_nguoi() {
    TestDatabase::run(async |test| {
        let shop = OrgId::from_uuid(Uuid::now_v7());
        let app = site_app(test, shop).await;
        for index in 0..50 {
            let body = json!({"name": format!("V{index}"), "overrides": {"radius": "small"}, "priority": index,
                "schedule": {"kind": "daily", "start": format!("{:02}:00", index % 24), "end": format!("{:02}:30", index % 24)}});
            let created = send(&app, "POST", "/theme-variants", Some("owner"), &[], Some(body)).await;
            assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
            let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
            send(&app, "POST", &format!("/theme-variants/{id}/activate"), Some("owner"), &[("if-match", "*")], None).await;
        }
        let root = autoxide_kernel::tenancy::root_of(DEFAULT_TENANT).as_uuid();
        for region in ["header", "footer"] {
            let row = put_region(&app, "owner", &format!("org_id={root}&region={region}&locale=en"), layout(region)).await;
            publish(&app, "owner", row["id"].as_str().unwrap()).await;
        }
        let mut cold = Vec::new();
        let mut warm = Vec::new();
        for round in 0..100 {
            // Nguội: sửa theme (không đo) xoá cache, rồi đo lần đọc đầu.
            let theme = json!({"tokens": {"font.size_base": 14 + round % 4}});
            send(&app, "PUT", &format!("/themes/{root}"), Some("owner"), &[("if-match", "*")], Some(theme)).await;
            let started = Instant::now();
            assert_eq!(send(&app, "GET", "/public/site", None, &[], None).await.status(), StatusCode::OK);
            cold.push(started.elapsed());
            let started = Instant::now();
            assert_eq!(send(&app, "GET", "/public/site", None, &[], None).await.status(), StatusCode::OK);
            warm.push(started.elapsed());
        }
        cold.sort();
        warm.sort();
        let (cold99, warm99): (Duration, Duration) = (cold[98], warm[98]);
        eprintln!("content public site, 50 variants: cold p50 = {:?}, p99 = {cold99:?}; warm p50 = {:?}, p99 = {warm99:?}", cold[50], warm[50]);
        assert!(cold99 < Duration::from_secs(2) && warm99 < Duration::from_secs(2));
    })
    .await
    .unwrap();
}
