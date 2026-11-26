//! Bộ nháp chiến dịch từ một chương trình khuyến mại (bước 67): `promotions` và `newsletter` là module
//! giả khai đúng tên tool thật (`content` gọi chúng qua sổ tool của kernel, không phụ thuộc crate).
//! Kết quả: trang nháp + lượt AI dựng bố cục, banner nháp link tới chương trình, chiến dịch nháp gắn
//! nhãn quảng cáo; số bịa trong chữ bị gọi lại; thiếu quyền / module thì báo rõ; hạn mức theo gói.
//! Gate chạy trên cả SQLite và Postgres.

use std::sync::Mutex;

use autoxide_kernel::access::Permission;
use autoxide_kernel::ai::Prompt;
use autoxide_kernel::error::Error;
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::resource::{Resource, ResourceResolver};
use serde::Deserialize;
use autoxide_spike_macros::Schema as ToSchema;
use validator::Validate;

use super::*;

// Mô tả mượn câu đã dịch của `content.read` / `content.write`: `cargo xtask check` đòi bản dịch cho mọi
// câu trong `src/`, kể cả module thử.
const PROMOTIONS_READ: Permission =
    Permission::new("promotions.read", "Read drafts, revisions and schedules of posts and pages.");
const NEWSLETTER_MANAGE: Permission = Permission::new(
    "newsletter.manage",
    "Create posts and pages, edit drafts, add translations, restore revisions, make preview links.",
);

#[derive(Debug, Deserialize, Validate, ToSchema)]
struct PromotionId {
    id: Uuid,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
struct CampaignInput {
    list_id: Uuid,
    subject: String,
    preheader: Option<String>,
    body_text: String,
    locale: Option<String>,
    advertising: Option<bool>,
}

struct Promotions {
    id: Uuid,
}

/// Sổ đối tượng: chương trình đang chạy, công khai (banner link tới nó).
struct PromotionResolver;

#[async_trait::async_trait]
impl ResourceResolver for PromotionResolver {
    async fn resolve(&self, _: &TenantContext, _: &str) -> Result<Option<Resource>, Error> {
        Ok(Some(Resource::new("Sale cuối tháng").public(true)))
    }
}

impl Module for Promotions {
    fn name(&self) -> &'static str {
        "promotions"
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![PROMOTIONS_READ]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.resource_type("promotions.promotion", Arc::new(PromotionResolver))
    }

    fn tools(&self) -> Vec<Tool> {
        let known = self.id;
        vec![
            Tool::new(
                "get_promotion",
                "Get a promotion.",
                move |_: TenantContext, _: Principal, input: PromotionId| async move {
                    if input.id != known {
                        // Mã của module giả phải mang tiền tố `content.` (`cargo xtask check`).
                        return Err(crate::service::entry_not_found(input.id));
                    }
                    Ok(json!({
                        "id": known, "status": "active",
                        "rule": {"kind": "flash_sale", "value_bp": 2000, "ends_at": "2026-11-30T17:00:00Z", "org_id": "0192"},
                        "texts": [{"locale": "vi", "title": "Sale cuối tháng", "terms": "Áp dụng tới 30/11."}],
                        "usage_count": 7,
                    }))
                },
            )
            .requires(PROMOTIONS_READ)
            .read_only(),
        ]
    }
}

struct Newsletter {
    sent: Arc<Mutex<Vec<Value>>>,
}

impl Module for Newsletter {
    fn name(&self) -> &'static str {
        "newsletter"
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![NEWSLETTER_MANAGE]
    }

    fn tools(&self) -> Vec<Tool> {
        let sent = self.sent.clone();
        vec![
            Tool::new(
                "create_campaign",
                "Write a draft campaign.",
                move |_: TenantContext, _: Principal, input: CampaignInput| {
                    let sent = sent.clone();
                    async move {
                        sent.lock().unwrap().push(json!({
                            "list_id": input.list_id, "subject": input.subject,
                            "preheader": input.preheader, "body_text": input.body_text,
                            "locale": input.locale, "advertising": input.advertising,
                        }));
                        Ok::<_, Error>(
                            json!({"id": Uuid::now_v7(), "status": "draft", "version": "1"}),
                        )
                    }
                },
            )
            .requires(NEWSLETTER_MANAGE),
        ]
    }
}

fn layout() -> Value {
    json!({"layout_json": json!({"type": "layout", "schema": 1, "sections": [
        {"id": "hero", "block": "content.hero", "props": {"heading": "Sale cuối tháng", "text": "Đồ mới mỗi tuần."}}
    ]}).to_string()})
}

/// Mô hình giả: trả bố cục cho lượt dựng trang; chữ chiến dịch theo kịch bản (lần đầu có số bịa).
fn model(copies: Vec<Value>, prompts: Arc<Mutex<Vec<String>>>) -> FakeTextModel {
    let copies = Mutex::new(copies);
    FakeTextModel::new(move |prompt: &Prompt| {
        let text: Vec<String> = prompt.input.iter().map(|part| format!("{part:?}")).collect();
        prompts.lock().unwrap().push(format!("{} {}", prompt.purpose, text.join(" ")));
        if prompt.purpose == "content.ai_page" {
            return Ok(layout());
        }
        let mut list = copies.lock().unwrap();
        Ok(if list.len() > 1 { list.remove(0) } else { list[0].clone() })
    })
}

fn copy(title: &str) -> Value {
    json!({
        "banner": {"title": title, "subtitle": "Tới 30/11", "cta_label": "Mua ngay"},
        "newsletter": {"subject": "Sale cuối tháng", "preheader": "Giảm 20%", "body_text": "Giảm 20% tới 30/11."},
    })
}

async fn campaign_app(
    test: &TestDatabase,
    model: FakeTextModel,
    promotion: Uuid,
    sent: &Arc<Mutex<Vec<Value>>>,
    with_newsletter: bool,
) -> TestApp {
    let media: Arc<dyn Media> = Arc::new(FakeMedia::default());
    let text: Arc<dyn TextModel> = Arc::new(model);
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::disabled());
    let mut builder = TestApp::builder(test)
        .user(
            "admin",
            1,
            &[
                "content.read",
                "content.write",
                "content.publish",
                "content.banners",
                "content.manage",
                "promotions.read",
                "newsletter.manage",
            ],
        )
        .user("writer", 2, &["content.read", "content.write", "promotions.read"])
        .user("stranger", 3, &["content.read", "content.write_own", "promotions.read"])
        .provide::<dyn Media>(media)
        .provide::<dyn TextModel>(text)
        .provide::<dyn Embedder>(embedder)
        .module(crate::module())
        .module(Promotions { id: promotion });
    if with_newsletter {
        builder = builder.module(Newsletter { sent: sent.clone() });
    }
    let app = builder.build().await.unwrap();
    presets(&app).await;
    app
}

/// Nạp các vị trí banner mẫu (có `top_strip`, vị trí chữ).
async fn presets(app: &TestApp) {
    let seeded = send(app, "POST", "/placements/presets", Some("admin"), &[], None).await;
    assert!(seeded.status().is_success(), "{}", seeded.json());
}

async fn run_jobs(app: &TestApp) {
    let worker = app.modules().worker(2).unwrap();
    while worker.run_once().await.unwrap() > 0 {}
}

#[tokio::test]
async fn bo_nhap_chien_dich_tao_ba_ban_nhap_va_khong_bia_so() {
    TestDatabase::run(async |test| {
        let promotion = Uuid::now_v7();
        let prompts = Arc::new(Mutex::new(Vec::new()));
        // Lần đầu: banner ghi "Giảm 50%" (không có trong chương trình) → bị gọi lại kèm lỗi.
        let fake = model(vec![copy("Giảm 50% hôm nay"), copy("Giảm 20% cuối tháng")], prompts.clone());
        let sent = Arc::new(Mutex::new(Vec::new()));
        let app = campaign_app(test, fake, promotion, &sent, true).await;
        let list = Uuid::now_v7();
        let body = json!({
            "promotion_id": promotion, "locale": "vi",
            "channels": ["page", "banner", "newsletter"],
            "banner_placement": "top_strip", "newsletter_list_id": list,
        });

        // Thiếu `content.banners` / `newsletter.manage`: không được xin kênh đó.
        let refused = send(&app, "POST", "/campaign-drafts", Some("writer"), &[], Some(body.clone())).await;
        assert_eq!(refused.status(), StatusCode::FORBIDDEN, "{}", refused.json());
        // Kênh lạ, thiếu vị trí banner và danh sách: báo một lần.
        let wrong = send(&app, "POST", "/campaign-drafts", Some("admin"), &[],
            Some(json!({"promotion_id": promotion, "channels": ["page", "sms", "banner", "newsletter"]}))).await;
        assert_eq!(wrong.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let fields: Vec<String> = wrong.json()["errors"].as_array().unwrap().iter()
            .map(|error| error["field"].as_str().unwrap().to_owned()).collect();
        assert_eq!(fields, ["channels[1]", "banner_placement", "newsletter_list_id"]);
        // Chương trình không có: lỗi của `promotions`, trước khi xếp hàng.
        let missing = send(&app, "POST", "/campaign-drafts", Some("admin"), &[],
            Some(json!({"promotion_id": Uuid::now_v7(), "channels": ["page"]}))).await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(missing.json()["code"], "content.not_found");

        let queued = send(&app, "POST", "/campaign-drafts", Some("admin"), &[], Some(body)).await;
        assert_eq!(queued.status(), StatusCode::ACCEPTED, "{}", queued.json());
        assert_eq!(queued.json()["data"]["status"], "queued");
        let id = queued.json()["data"]["id"].as_str().unwrap().to_owned();
        run_jobs(&app).await;

        let done = send(&app, "GET", &format!("/campaign-drafts/{id}"), Some("admin"), &[], None).await;
        assert_eq!(done.status(), StatusCode::OK, "{}", done.json());
        let data = done.json()["data"].clone();
        assert_eq!(data["status"], "succeeded", "{data}");
        assert_eq!(data["tries"], 2);
        let results = data["results"].as_array().unwrap();
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|result| result["status"] == "created"), "{data}");

        // Trang: bản nháp có bố cục do AI dựng, không tự đăng.
        let page = &results[0];
        assert_eq!(page["kind"], "content.entry");
        let entry = send(&app, "GET", &format!("/entries/{}", page["id"].as_str().unwrap()), Some("admin"), &[], None).await;
        assert_eq!(entry.json()["data"]["status"], "draft");
        assert_eq!(entry.json()["data"]["ai_assisted"], true);
        assert_eq!(entry.json()["data"]["title"], "Sale cuối tháng");
        let turn = send(&app, "GET", &format!("/entries/{}/ai/thread", page["id"].as_str().unwrap()), Some("admin"), &[], None).await;
        assert_eq!(turn.json()["data"][0]["status"], "succeeded", "{}", turn.json());
        let entry = send(&app, "GET", &format!("/entries/{}", page["id"].as_str().unwrap()), Some("admin"), &[], None).await;
        assert_eq!(entry.json()["data"]["body"]["type"], "layout");

        // Banner: nháp, chữ không bịa số, link tới chương trình.
        let banner = send(&app, "GET", &format!("/banners/{}", results[1]["id"].as_str().unwrap()), Some("admin"), &[], None).await;
        assert_eq!(banner.status(), StatusCode::OK, "{}", banner.json());
        let banner = banner.json()["data"].clone();
        assert_eq!(banner["status"], "draft");
        assert_eq!(banner["title"], "Giảm 20% cuối tháng");
        assert_eq!(banner["ai_assisted"], true);
        assert_eq!(banner["link"]["kind"], "promotions.promotion", "{banner}");

        // Chiến dịch: nháp gắn nhãn quảng cáo, đúng danh sách.
        let mail = sent.lock().unwrap()[0].clone();
        assert_eq!(mail["list_id"], json!(list));
        assert_eq!(mail["advertising"], true);
        assert_eq!(mail["locale"], "vi");

        // Mô hình chỉ thấy dữ liệu của chương trình, không thấy ID nội bộ hay bộ đếm.
        let seen = prompts.lock().unwrap().join("\n");
        assert!(seen.contains("unknown_number") || seen.contains("not in the promotion facts"), "{seen}");
        assert!(!seen.contains("usage_count"));
        assert!(!seen.contains("\"0192\""));

        // Người khác chỉ có `content.write_own`: không thấy yêu cầu của admin.
        let hidden = send(&app, "GET", &format!("/campaign-drafts/{id}"), Some("stranger"), &[], None).await;
        assert_eq!(hidden.status(), StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bo_nhap_chien_dich_bao_ro_khi_thieu_module_hay_chu_hong() {
    TestDatabase::run(async |test| {
        let promotion = Uuid::now_v7();
        let prompts = Arc::new(Mutex::new(Vec::new()));
        // Chữ luôn có số bịa: hết lượt thử → kênh banner thất bại, trang vẫn tạo được.
        let fake = model(vec![json!({"banner": copy("Giảm 70%")["banner"]})], prompts);
        let sent = Arc::new(Mutex::new(Vec::new()));
        let app = campaign_app(test, fake, promotion, &sent, false).await;
        let no_newsletter = send(&app, "POST", "/campaign-drafts", Some("admin"), &[],
            Some(json!({"promotion_id": promotion, "channels": ["newsletter"], "newsletter_list_id": Uuid::now_v7()}))).await;
        assert_eq!(no_newsletter.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(no_newsletter.json()["code"], "content.campaign_module_unavailable");

        let queued = send(&app, "POST", "/campaign-drafts", Some("admin"), &[],
            Some(json!({"promotion_id": promotion, "channels": ["page", "banner"], "banner_placement": "top_strip"}))).await;
        assert_eq!(queued.status(), StatusCode::ACCEPTED, "{}", queued.json());
        let id = queued.json()["data"]["id"].as_str().unwrap().to_owned();
        run_jobs(&app).await;
        let data = send(&app, "GET", &format!("/campaign-drafts/{id}"), Some("admin"), &[], None).await.json()["data"].clone();
        assert_eq!(data["status"], "partial", "{data}");
        assert_eq!(data["tries"], 3);
        assert_eq!(data["results"][0]["status"], "created");
        assert_eq!(data["results"][1]["status"], "failed");
        assert_eq!(data["results"][1]["error_code"], "content.campaign_copy_invalid");
    })
    .await
    .unwrap();
}
