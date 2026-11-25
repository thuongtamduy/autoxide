//! Theo khách (bước 33): slug, đường dẫn, tác giả, term, chuyển hướng chỉ không trùng trong một
//! khách; cache tìm theo ý nghĩa không dùng chung giữa khách; lịch đăng của khách không mặc định
//! đăng và ghi event ở đúng khách đó. Hai khách thật của `TestApp`: `a` dùng chung database chính,
//! `b` có database riêng.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::ai::{Embedder, TextModel};
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_media::api::Media;
use autoxide_testkit::TestApp;
use autoxide_testkit::ai::{FakeEmbedder, FakeTextModel};

use super::*;
use crate::ai::semantic::{Scope, SearchQuery};
use crate::publishing::redirects::{RedirectInput, Resolved};
use crate::publishing::schedules::{Action, RunSchedule};
use crate::service::ContentService;
use crate::taxonomy::authors::AuthorInput;
use crate::taxonomy::terms::{Taxonomy, TermInput};
use crate::testing::{FakeMedia, service_in};

/// App có module `content` (để migrate database riêng của `b`) và hai khách `a`, `b`; service
/// tra khách qua sổ của app.
async fn two_tenants(
    test: &TestDatabase,
    embedder: Arc<dyn Embedder>,
) -> (TestApp, Arc<ContentService>, TenantContext, TenantContext) {
    let media: Arc<dyn Media> = Arc::new(FakeMedia::new().tagging_locale());
    let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
    let app = TestApp::builder(test)
        .tenant("a")
        .tenant_dedicated("b")
        .provide::<dyn Media>(media)
        .provide::<dyn TextModel>(text.clone())
        .provide::<dyn Embedder>(embedder.clone())
        .module(crate::module())
        .build()
        .await
        .unwrap();
    let (content, _, _) = service_in(test, config(&[]), text, embedder, &app).await;
    let a = app.tenant("a").await.unwrap();
    let b = app.tenant("b").await.unwrap();
    (app, content, a, b)
}

/// Tên các event `content` của khách `tenant` (trong database của khách đó), theo thứ tự.
async fn events_of(tenant: &TenantContext) -> Vec<String> {
    let database = tenant.database();
    let backend = database.read().get_database_backend();
    let sql = "SELECT name, tenant_id FROM content_events ORDER BY id";
    let rows = database.read().query_all_raw(Statement::from_string(backend, sql)).await;
    rows.unwrap()
        .iter()
        .filter(|row| row.try_get::<Uuid>("", "tenant_id").unwrap() == tenant.id().as_uuid())
        .map(|row| row.try_get("", "name").unwrap())
        .collect()
}

/// Cùng slug bài, đường dẫn công khai, slug tác giả, slug thẻ và chuyển hướng ở hai khách đều
/// tạo được, mỗi khách thấy bản của mình; trong một khách vẫn bị chặn như trước.
#[tokio::test]
async fn trung_ten_o_hai_khach_duoc_phep() {
    TestDatabase::run(async |test| {
        let (_app, content, a, b) = two_tenants(test, Arc::new(FakeEmbedder::disabled())).await;
        let ada = editor();
        let author = || AuthorInput {
            display_name: Some("Ada".into()),
            slug: Some("ada".into()),
            ..Default::default()
        };
        let tag = || TermInput {
            locale: Some("en".into()),
            name: Some("Travel".into()),
            slug: Some("travel".into()),
            ..Default::default()
        };
        let redirect = || RedirectInput {
            locale: None,
            from_path: "/old".into(),
            to_entry_id: None,
            to_url: Some("https://example.com/".into()),
            status_code: 301,
        };
        let mut published = Vec::new();
        for tenant in [&a, &b] {
            let entry = content
                .create(tenant, &ada.clone().in_tenant(tenant.id()), post("Hello"))
                .await
                .unwrap();
            assert_eq!(entry.slug, "hello");
            content
                .publish(tenant, &ada.clone().in_tenant(tenant.id()), entry.id, None, None, false)
                .await
                .unwrap();
            published.push(entry.id);
            assert_eq!(
                content
                    .create_author(tenant, &ada.clone().in_tenant(tenant.id()), author())
                    .await
                    .unwrap()
                    .slug,
                "ada"
            );
            let term = content
                .create_term(tenant, &ada.clone().in_tenant(tenant.id()), Taxonomy::Tag, tag())
                .await
                .unwrap();
            assert_eq!(term.texts[0].slug, "travel");
            content
                .create_redirect(tenant, &ada.clone().in_tenant(tenant.id()), redirect())
                .await
                .unwrap();
        }
        for (tenant, id) in [(&a, published[0]), (&b, published[1])] {
            match content
                .resolve(
                    tenant,
                    &autoxide_kernel::tenancy::Site::whole(tenant),
                    "en",
                    "/blog/hello",
                )
                .await
                .unwrap()
            {
                Resolved::Entry(row) => assert_eq!(row.entry_id, id),
                other => panic!("expected an entry, got {other:?}"),
            }
            assert_eq!(content.author_by_slug(tenant, "ada").await.unwrap().slug, "ada");
        }
        // Mục của khách này không thấy được từ khách kia.
        assert_eq!(
            content.get(&b, &ada.clone().in_tenant(b.id()), published[0]).await.unwrap_err().code(),
            "content.not_found"
        );

        // Trong một khách: vẫn bị chặn.
        let mut again = post("Hello again");
        again.fields.slug = Some("hello".into());
        assert_eq!(
            content.create(&a, &ada.clone().in_tenant(a.id()), again).await.unwrap_err().code(),
            "content.slug_taken"
        );
        assert_eq!(
            content
                .create_author(&a, &ada.clone().in_tenant(a.id()), author())
                .await
                .unwrap_err()
                .code(),
            "content.slug_taken"
        );
        assert_eq!(
            content
                .create_term(&a, &ada.clone().in_tenant(a.id()), Taxonomy::Tag, tag())
                .await
                .unwrap_err()
                .code(),
            "content.slug_taken"
        );
        assert_eq!(
            content
                .create_redirect(&a, &ada.clone().in_tenant(a.id()), redirect())
                .await
                .unwrap_err()
                .code(),
            "content.redirect_exists"
        );
    })
    .await
    .unwrap();
}

/// Vector trong bộ nhớ giữ theo khách: hai khách cùng (`scope`, `locale`) và cùng số `version`
/// không lấy nhầm vector của nhau; sửa ở khách `b` chỉ làm `b` nạp lại.
#[tokio::test]
async fn cache_tim_theo_y_nghia_khong_dung_chung() {
    TestDatabase::run(async |test| {
        let embedder = Arc::new(FakeEmbedder::new(128));
        let (_app, content, a, b) = two_tenants(test, embedder).await;
        let ada = editor();
        let vi_post = |title: &str, text: &str| {
            let mut input = post(title);
            input.locale = Some("vi".into());
            input.fields.body = Some(paragraph(text));
            input
        };
        let query =
            |q: &str| SearchQuery { q: q.into(), locale: Some("vi".into()), ..Default::default() };
        // `b` lập index và tìm trước: vector của `b` (version 1) vào bộ nhớ.
        let in_b = content
            .create(
                &b,
                &ada.clone().in_tenant(b.id()),
                vi_post("Đà Lạt mùa hoa", "Hoa dã quỳ nở vàng đồi thông."),
            )
            .await
            .unwrap();
        content.index_entry(&b, in_b.id).await.unwrap();
        let hits = content
            .search(&b, Some(&ada.clone().in_tenant(b.id())), &query("hoa đà lạt"), None)
            .await
            .unwrap();
        assert_eq!(hits[0].entry_id, in_b.id);
        assert_eq!(content.search_cache.cached_version(b.id(), Scope::Draft, "vi"), Some(1));

        // `a` cũng ở version 1 cho cùng (`draft`, `vi`): chỉ thấy mục của chính mình.
        let in_a = content
            .create(
                &a,
                &ada.clone().in_tenant(a.id()),
                vi_post("Học Rust", "Ownership và lifetime."),
            )
            .await
            .unwrap();
        content.index_entry(&a, in_a.id).await.unwrap();
        let hits = content
            .search(&a, Some(&ada.clone().in_tenant(a.id())), &query("hoa đà lạt"), None)
            .await
            .unwrap();
        assert!(hits.iter().all(|hit| hit.entry_id == in_a.id), "{hits:?}");
        assert!(hits.iter().all(|hit| hit.entry_id != in_b.id));
        let hits = content
            .search(&a, Some(&ada.clone().in_tenant(a.id())), &query("ownership"), None)
            .await
            .unwrap();
        assert_eq!(hits[0].entry_id, in_a.id);
        assert_eq!(content.search_cache.cached_version(a.id(), Scope::Draft, "vi"), Some(1));

        // Sửa mục của `b`: chỉ `b` nạp lại (version 2), bản của `a` giữ nguyên.
        let changes = crate::entries::EntryFields {
            title: Some("Đà Lạt mùa mưa".into()),
            ..Default::default()
        };
        content
            .update(&b, &ada.clone().in_tenant(b.id()), in_b.id, changes, false, None)
            .await
            .unwrap();
        content.index_entry(&b, in_b.id).await.unwrap();
        let hits = content
            .search(&b, Some(&ada.clone().in_tenant(b.id())), &query("mùa mưa"), None)
            .await
            .unwrap();
        assert_eq!(hits[0].entry_id, in_b.id);
        assert_eq!(content.search_cache.cached_version(b.id(), Scope::Draft, "vi"), Some(2));
        assert_eq!(content.search_cache.cached_version(a.id(), Scope::Draft, "vi"), Some(1));
    })
    .await
    .unwrap();
}

/// Lịch đăng của khách `b` (database riêng) chạy bằng job của khách đó: đăng ở `b`, event ghi ở
/// database của `b`; khách `a` không có gì. Job định kỳ dọn revision và lập index bù chạy theo
/// từng database.
#[tokio::test]
async fn lich_dang_chay_dung_khach() {
    TestDatabase::run(async |test| {
        let (_app, content, a, b) = two_tenants(test, Arc::new(FakeEmbedder::disabled())).await;
        let ada = editor();
        let entry =
            content.create(&b, &ada.clone().in_tenant(b.id()), post("Later")).await.unwrap();
        let at = Timestamp::now() + SignedDuration::from_hours(1);
        let schedule = content
            .create_schedule(
                &b,
                &ada.clone().in_tenant(b.id()),
                entry.id,
                Action::Publish,
                at,
                None,
            )
            .await
            .unwrap();
        let payload = serde_json::to_value(RunSchedule { schedule_id: schedule.id }).unwrap();
        let job = JobContext::for_tenant(b.clone(), crate::publishing::schedules::RUN, payload, 1);
        crate::publishing::schedules::Runner(content.clone()).run(&job).await.unwrap();

        assert_eq!(
            content.get(&b, &ada.clone().in_tenant(b.id()), entry.id).await.unwrap().status,
            "published"
        );
        assert!(matches!(
            content
                .resolve(&b, &autoxide_kernel::tenancy::Site::whole(&b), "en", "/blog/later")
                .await
                .unwrap(),
            Resolved::Entry(_)
        ));
        assert!(
            content
                .resolve(&a, &autoxide_kernel::tenancy::Site::whole(&a), "en", "/blog/later")
                .await
                .is_err()
        );
        let events = events_of(&b).await;
        assert!(events.contains(&"content.entry_published".to_owned()), "{events:?}");
        assert!(events_of(&a).await.is_empty());

        // Job của cả database: chỉ chạm khách có dữ liệu trên database đó.
        assert_eq!(content.prune_revisions(a.database()).await.unwrap(), 0);
        assert_eq!(content.prune_revisions(b.database()).await.unwrap(), 0);
    })
    .await
    .unwrap();
}

/// Cấu hình theo khách (đối chiếu thiết kế 2, A-b7): ngôn ngữ của khách `b` thay
/// `AUTOXIDE_CONTENT_LOCALES` / `AUTOXIDE_CONTENT_DEFAULT_LOCALE` cho riêng `b` — kiểm dữ liệu
/// vào, ngôn ngữ của bài mới, route công khai và `GET /kernel/client-config`; khách `a` chưa đặt
/// dùng biến môi trường; giá trị sai bị từ chối lúc đặt.
#[tokio::test]
async fn cau_hinh_module_theo_khach_ghi_de_bien_moi_truong() {
    TestDatabase::run(async |test| {
        let media: Arc<dyn Media> = Arc::new(FakeMedia::new().tagging_locale());
        let text: Arc<dyn TextModel> = Arc::new(FakeTextModel::disabled());
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
        let app = TestApp::builder(test)
            .tenant("a")
            .tenant_dedicated("b")
            .user("platform", 1, &["kernel.tenants.manage", "kernel.tenants.read"])
            .provide::<dyn Media>(media)
            .provide::<dyn TextModel>(text.clone())
            .provide::<dyn Embedder>(embedder.clone())
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let (content, _, _) = service_in(test, config(&[]), text, embedder, &app).await;
        let (a, b) = (app.tenant("a").await.unwrap(), app.tenant("b").await.unwrap());
        let base = "/api/v1/kernel/tenants/b/settings";
        let set = |name: &'static str, value: &'static str| {
            app.put(&format!("{base}/{name}")).token("platform").json(json!({ "value": value }))
        };
        let refused = set("content.locales", "vi, english").send().await.unwrap();
        assert_eq!(refused.status(), 422, "{}", refused.text());
        assert_eq!(refused.json()["errors"][0]["field"], "value");
        for (name, value) in [("content.locales", "vi, FR"), ("content.default_locale", "fr")] {
            let response = set(name, value).send().await.unwrap();
            assert_eq!(response.status(), 200, "{}", response.text());
        }
        let listed = app.get(base).token("platform").send().await.unwrap().json();
        let locales = listed["data"].as_array().unwrap();
        let locales = locales.iter().find(|s| s["name"] == "content.locales").unwrap();
        assert_eq!(locales["value"], "vi, FR");

        // Bài mới không nêu ngôn ngữ: mặc định của khách; `fr` chỉ hợp lệ ở `b`, `en` chỉ ở `a`.
        let ada = editor();
        assert_eq!(
            content
                .create(&b, &ada.clone().in_tenant(b.id()), post("Bonjour"))
                .await
                .unwrap()
                .locale,
            "fr"
        );
        assert_eq!(
            content.create(&a, &ada.clone().in_tenant(a.id()), post("Hello")).await.unwrap().locale,
            "en"
        );
        let in_language = |locale: &str| {
            let mut entry = post("Xin chào");
            entry.locale = Some(locale.to_owned());
            entry
        };
        let error = content
            .create(&a, &ada.clone().in_tenant(a.id()), in_language("fr"))
            .await
            .unwrap_err();
        assert_eq!(error.fields()[0].code, "locale_not_supported");
        let error = content
            .create(&b, &ada.clone().in_tenant(b.id()), in_language("en"))
            .await
            .unwrap_err();
        assert_eq!(error.fields()[0].code, "locale_not_supported");
        content.create(&b, &ada.clone().in_tenant(b.id()), in_language("vi")).await.unwrap();

        // Route công khai và cấu hình cho giao diện theo khách của request.
        let public = |slug: &'static str, locale: &'static str| {
            app.get(&format!("/api/v1/content/public/posts?locale={locale}"))
                .header("autoxide-tenant", slug)
                .send()
        };
        assert_eq!(public("b", "fr").await.unwrap().status(), 200);
        assert_eq!(public("a", "fr").await.unwrap().status(), 422);
        let client = |slug: Option<&'static str>| {
            let request = app.get("/api/v1/kernel/client-config");
            let request = match slug {
                Some(slug) => request.header("autoxide-tenant", slug),
                None => request,
            };
            async move { request.send().await.unwrap().json()["data"]["content"].clone() }
        };
        let theirs = client(Some("b")).await;
        assert_eq!(
            (theirs["locales"].clone(), theirs["default_locale"].clone()),
            (json!(["vi", "fr"]), json!("fr"))
        );
        let ours = client(Some("a")).await;
        assert_eq!(
            (ours["locales"].clone(), ours["default_locale"].clone()),
            (json!(["en", "vi"]), json!("en"))
        );
        // Request không nêu khách: cấu hình của máy chủ.
        assert_eq!(client(None).await["locales"], json!(["en", "vi"]));

        // Bỏ cấu hình: về biến môi trường.
        let removed = app.delete(&format!("{base}/content.locales")).token("platform").send();
        assert_eq!(removed.await.unwrap().status(), 204);
        assert_eq!(
            content.create(&b, &ada.clone().in_tenant(b.id()), post("Hi")).await.unwrap().locale,
            "en"
        );
    })
    .await
    .unwrap();
}
