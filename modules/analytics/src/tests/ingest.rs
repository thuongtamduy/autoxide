//! Nhận sự kiện: không lưu IP / user agent, khoá phiên theo ngày, sự kiện lạ bị bỏ, bot bị loại,
//! trang ngoài website bị loại, sự kiện phía máy chủ.

use autoxide_kernel::event::Delivered;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use serde_json::json;

use super::{CHROME, HOST, SAFARI, at, browser, harness, page, user_id};
use crate::ingest::Ingested;

#[tokio::test]
async fn khong_luu_ip_va_user_agent_day_du() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        let client = browser("203.0.113.77", CHROME);
        let ingested = h.send(None, &client, None, vec![page("/")], at(10, 9)).await;
        assert_eq!(ingested, Ingested { accepted: 1, dropped: 0 });
        let rows = h.raw().await;
        assert_eq!(rows.len(), 1);
        // Không cột nào giữ IP hay user agent: tìm cả dạng thô lẫn khối mạng trong mọi giá trị.
        let dump = format!("{:?}", rows[0]);
        for secret in ["203.0.113.77", "203.0.113.0", "203.0.113", "Android 14", "Chrome/130"] {
            assert!(!dump.contains(secret), "`{secret}` is stored: {dump}");
        }
        assert_eq!(rows[0].session_key.len(), 22);
        assert_eq!(rows[0].user_id, None);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khoa_phien_khong_noi_duoc_hai_ngay() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        let client = browser("198.51.100.4", SAFARI);
        h.send(None, &client, None, vec![page("/a")], at(10, 9)).await;
        h.send(None, &client, None, vec![page("/b")], at(10, 20)).await;
        h.send(None, &client, None, vec![page("/a")], at(11, 9)).await;
        let rows = h.raw().await;
        // Cùng ngày: một phiên; ngày sau: phiên khác.
        assert_eq!(rows[0].session_key, rows[1].session_key);
        assert_ne!(rows[0].session_key, rows[2].session_key);
        // Khoá của ngày đã qua bị xoá: không ai tính lại được khoá phiên của ngày đó.
        h.service.cleanup(h.app.database(), at(11, 10)).await.unwrap();
        use sea_orm::EntityTrait;
        let keys = crate::entity::day_key::Entity::find().all(h.app.database().read()).await;
        let days: Vec<String> = keys.unwrap().into_iter().map(|key| key.day).collect();
        assert_eq!(days, ["2026-11-11"]);
        // Khối mạng khác cùng ngày: phiên khác; cùng khối (IPv4 /24): cùng phiên.
        let same_block = browser("198.51.100.200", SAFARI);
        let other = browser("192.0.2.1", SAFARI);
        h.send(None, &same_block, None, vec![page("/c")], at(11, 9)).await;
        h.send(None, &other, None, vec![page("/c")], at(11, 9)).await;
        let rows = h.raw().await;
        assert_eq!(rows[3].session_key, rows[2].session_key);
        assert_ne!(rows[4].session_key, rows[2].session_key);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn su_kien_la_bi_bo_khong_loi() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        let events = json!({"events": [
            {"type": "analytics.page_view", "path": "/san-pham?email=a@b.example", "referrer": "https://www.google.com/search?q=x", "utm": {"source": "Facebook", "campaign": "Tet_Sale"}},
            {"type": "shop.product_viewed", "path": "/p/1", "object": "shop.product:p-1", "props": {"variant": "red"}},
            {"type": "shop.product_viewed", "path": "/p/1", "object": "shop.product:p-1", "props": {"variant": 3}},
            {"type": "shop.product_viewed", "path": "/p/1", "object": "shop.product:p-1", "props": {"color": "red"}},
            {"type": "shop.product_viewed", "path": "/p/1"},
            {"type": "shop.purchased", "path": "/thanks", "object": "shop.product:p-1"},
            {"type": "nobody.declared", "path": "/"},
            {"type": 7},
            "not an object"
        ]});
        let answer = h
            .app
            .post("/api/v1/analytics/public/events")
            .host(HOST)
            .header("origin", &format!("https://{HOST}"))
            .header("user-agent", CHROME)
            .json(events)
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), StatusCode::ACCEPTED, "{}", answer.text());
        assert_eq!(answer.json()["data"], json!({"accepted": 2, "dropped": 7}));
        let rows = h.raw().await;
        assert_eq!(rows.len(), 2);
        let page = &rows[0];
        // Query bị bỏ (có thể chứa dữ liệu cá nhân); nguồn là UTM; giới thiệu chỉ giữ tên miền.
        assert_eq!(page.path, "/san-pham");
        assert_eq!(page.source, "facebook");
        assert_eq!(page.utm_campaign.as_deref(), Some("tet_sale"));
        assert_eq!(page.referrer_host.as_deref(), Some("google.com"));
        assert!(page.landing);
        assert_eq!(rows[1].props.as_deref(), Some(r#"{"variant":"red"}"#));

        // Lô rỗng hay quá 20 sự kiện: lỗi của người gọi (422), không phải sự kiện lạ.
        let many: Vec<_> = (0..21).map(|_| super::page("/")).collect();
        let answer = h
            .app
            .post("/api/v1/analytics/public/events")
            .host(HOST)
            .json(json!({"events": many}))
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bot_bi_loai() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        let send = |agent: &'static str, origin: Option<&'static str>| {
            let mut request = h
                .app
                .post("/api/v1/analytics/public/events")
                .host(HOST)
                .header("user-agent", agent)
                .json(json!({"events": [{"type": "analytics.page_view", "path": "/"}]}));
            if let Some(origin) = origin {
                request = request.header("origin", origin);
            }
            async move { request.send().await.unwrap().json()["data"]["accepted"].clone() }
        };
        let googlebot = "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)";
        assert_eq!(send(googlebot, Some("https://shop.example")).await, 0);
        assert_eq!(send("curl/8.5.0", Some("https://shop.example")).await, 0);
        // Trang không thuộc website (không phải tên miền của request, không khai, không xác minh).
        assert_eq!(send(CHROME, Some("https://evil.example")).await, 0);
        // Không có `Origin` lẫn `Referer`: không biết trang nào gửi.
        assert_eq!(send(CHROME, None).await, 0);
        assert_eq!(send(CHROME, Some("https://shop.example")).await, 1);
        assert_eq!(h.raw().await.len(), 1);

        // Giới hạn theo khối mạng mỗi phút: phần thừa bị bỏ.
        let limited = crate::service::AnalyticsService {
            config: crate::config::AnalyticsConfig {
                events_per_minute: 3,
                ..crate::config::AnalyticsConfig::default()
            },
            catalog: h.service.catalog.clone(),
            secret: h.service.secret.clone(),
            consents: None,
            registry: h.service.registry.clone(),
        };
        let client = super::browser("192.0.2.9", SAFARI);
        let site = h.site(None).await;
        let visitor = || crate::ingest::Visitor { client: &client, host: Some(HOST), user_id: None };
        let events = || (0..2).map(|_| page("/")).collect::<Vec<_>>();
        let first = limited.ingest(&h.tenant, &site, visitor(), events(), at(10, 9)).await;
        let second = limited.ingest(&h.tenant, &site, visitor(), events(), at(10, 9)).await;
        assert_eq!(first.unwrap(), Ingested { accepted: 2, dropped: 0 });
        assert_eq!(second.unwrap(), Ingested { accepted: 1, dropped: 1 });
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ten_mien_da_xac_minh_va_cau_hinh_duoc_nhan() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        let mut other_page = browser("192.0.2.1", CHROME);
        other_page.origin = Some("https://www.partner.example".to_owned());
        // Trang của tên miền khác: bỏ. Có trong `AUTOXIDE_ANALYTICS_ALLOWED_HOSTS`: nhận.
        assert_eq!(h.send(None, &other_page, None, vec![page("/")], at(10, 9)).await.accepted, 0);
        let allowed = crate::service::AnalyticsService {
            config: crate::config::AnalyticsConfig {
                allowed_hosts: vec!["www.partner.example".to_owned()],
                ..crate::config::AnalyticsConfig::default()
            },
            catalog: h.service.catalog.clone(),
            secret: h.service.secret.clone(),
            consents: None,
            registry: h.service.registry.clone(),
        };
        let site = h.site(None).await;
        let visitor = crate::ingest::Visitor { client: &other_page, host: None, user_id: None };
        let taken = allowed.ingest(&h.tenant, &site, visitor, vec![page("/")], at(10, 9)).await;
        assert_eq!(taken.unwrap().accepted, 1);
        // `Referer` thay cho `Origin`; cùng tên miền với request.
        let mut by_referer = browser("192.0.2.1", CHROME);
        by_referer.origin = None;
        by_referer.referer = Some(format!("https://{HOST}/a"));
        let visitor =
            crate::ingest::Visitor { client: &by_referer, host: Some(HOST), user_id: None };
        let taken = h.service.ingest(&h.tenant, &site, visitor, vec![page("/")], at(10, 9)).await;
        assert_eq!(taken.unwrap().accepted, 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn su_kien_phia_may_chu_tu_event_cua_module() {
    TestDatabase::run(async |test| {
        let h = harness(test).await;
        let declared = *h.service.catalog.event("shop.purchased").unwrap();
        let buyer = user_id(9);
        let mut delivered = Delivered::new(
            h.tenant.clone(),
            "shop",
            "shop.order_paid",
            json!({"org_id": h.tree.a.as_uuid(), "product_id": "p-1", "user_id": buyer}),
        );
        delivered.occurred_at = at(10, 12);
        let hit = super::paid(&delivered).unwrap().unwrap();
        h.service.record_server_hit(&declared, &delivered, hit.clone()).await.unwrap();
        // Nhận lại cùng event: không ghi thêm.
        h.service.record_server_hit(&declared, &delivered, hit).await.unwrap();
        let rows = h.raw().await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, "shop.purchased");
        assert_eq!(rows[0].site_org_id, h.tree.a.as_uuid());
        assert_eq!(rows[0].object.as_deref(), Some("shop.product:p-1"));
        assert!(rows[0].session_key.starts_with('~'));
        assert_eq!(rows[0].day, "2026-11-10");
        // Người mua chưa đồng ý theo dõi theo người: không gắn.
        assert_eq!(rows[0].user_id, None);
    })
    .await
    .unwrap();
}
