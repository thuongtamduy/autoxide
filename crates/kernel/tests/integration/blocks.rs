//! Sổ khối (bước 67): module khai khối, `GET /kernel/blocks` liệt kê kèm schema và bản dịch, dữ liệu
//! khối đọc theo lô — mỗi loại một lần gọi, khối lỗi / hết giờ thành `null` —, khai sai thì không
//! khởi động.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use autoxide_kernel::blocks::{
    BlockCategory, BlockData, BlockRef, BlockRequest, BlockType, Blocks,
};
use autoxide_kernel::entitlement::Feature;
use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Module, Modules, StartContext};
use autoxide_kernel::tenancy::{DatabaseRegistry, Site, TenantContext};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::{Value, json};

const GRID_SCHEMA: &str = r#"{
    "type": "object",
    "additionalProperties": false,
    "required": ["limit"],
    "properties": {
        "limit": {"type": "integer", "minimum": 1, "maximum": 24},
        "fail": {"type": "boolean"}
    }
}"#;
const TEXT_SCHEMA: &str = r#"{"type": "object", "additionalProperties": false,
    "properties": {"text": {"type": "string", "maxLength": 100}}}"#;
const VIP: Feature = Feature::new("shop.vip", "VIP blocks.");

/// Trả `{"n": limit}` cho mỗi khối; `fail` làm cả lô lỗi; `limit = 13` là tham chiếu không còn
/// công khai (`None`).
struct Grid(Arc<AtomicUsize>);

#[async_trait::async_trait]
impl BlockData for Grid {
    async fn resolve(
        &self,
        _request: &BlockRequest<'_>,
        block: &str,
        items: &[BlockRef],
    ) -> Result<Vec<Option<Value>>, Error> {
        self.0.fetch_add(1, Ordering::SeqCst);
        if items.iter().any(|item| item.props["fail"] == true) {
            return Err(Error::internal("boom"));
        }
        Ok(items
            .iter()
            .map(|item| {
                (item.props["limit"] != 13)
                    .then(|| json!({"block": block, "n": item.props["limit"]}))
            })
            .collect())
    }
}

struct Shop {
    calls: Arc<AtomicUsize>,
    blocks: Arc<Mutex<Option<Blocks>>>,
}

impl Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn features(&self) -> Vec<Feature> {
        vec![VIP]
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", r#"{"text:Product grid": "Lưới sản phẩm"}"#)]
    }

    fn blocks(&self) -> Vec<BlockType> {
        vec![
            BlockType::new(
                "shop.grid",
                BlockCategory::Commerce,
                "Product grid",
                "Use to show several products.",
                GRID_SCHEMA,
            )
            .with_data(),
            BlockType::new(
                "shop.text",
                BlockCategory::Text,
                "Text",
                "Use for a paragraph.",
                TEXT_SCHEMA,
            ),
            BlockType::new(
                "shop.vip",
                BlockCategory::Text,
                "VIP",
                "Use for VIP text.",
                TEXT_SCHEMA,
            )
            .requires(VIP),
        ]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.block_data("shop.grid", Arc::new(Grid(self.calls.clone())))?;
        *self.blocks.lock().unwrap() = Some(context.blocks());
        Ok(())
    }
}

#[tokio::test]
async fn so_khoi_liet_ke_va_doc_du_lieu_theo_lo() {
    TestDatabase::run(async |test| {
        let calls = Arc::new(AtomicUsize::new(0));
        let handle = Arc::new(Mutex::new(None));
        let app = TestApp::builder(test)
            .user("editor", 1, &[])
            .module(Shop { calls: calls.clone(), blocks: handle.clone() })
            .build()
            .await
            .unwrap();

        // Liệt kê: cần đăng nhập; có schema, nhãn dịch theo `Accept-Language`.
        assert_eq!(
            app.get("/api/v1/kernel/blocks").send().await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        let listed = app
            .get("/api/v1/kernel/blocks")
            .token("editor")
            .header("accept-language", "vi")
            .send()
            .await
            .unwrap();
        assert_eq!(listed.status(), StatusCode::OK, "{}", listed.json());
        let body = listed.json();
        let names: Vec<&str> =
            body["data"].as_array().unwrap().iter().map(|b| b["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["shop.grid", "shop.text", "shop.vip"]);
        let grid = &body["data"][0];
        assert_eq!(grid["label"], "Lưới sản phẩm");
        assert_eq!(grid["category"], "commerce");
        assert_eq!(grid["has_data"], true);
        assert_eq!(grid["props_schema"]["properties"]["limit"]["maximum"], 24);
        assert_eq!(body["data"][2]["feature"], "shop.vip");

        // Kiểm props theo schema.
        let blocks = handle.lock().unwrap().clone().unwrap();
        assert!(blocks.check_props("shop.grid", &json!({"limit": 4}), "p").is_empty());
        let wrong: Vec<String> = blocks
            .check_props("shop.grid", &json!({"limit": 99, "x": 1}), "p")
            .into_iter()
            .map(|error| format!("{} {}", error.field, error.code))
            .collect();
        assert_eq!(wrong, ["p.limit range", "p.x unknown"]);
        assert_eq!(blocks.check_props("nope.block", &json!({}), "p")[0].code, "unknown");

        // Dữ liệu theo lô: ba khối lưới, một lần gọi; khối chữ không cần dữ liệu.
        let tenant: TenantContext =
            DatabaseRegistry::single(app.database().clone()).default_tenant();
        let site = Site::whole(&tenant);
        let request = BlockRequest::new(&tenant, &site, "vi");
        let items = [
            ("shop.grid", BlockRef::new("a", json!({"limit": 4}))),
            ("shop.text", BlockRef::new("b", json!({"text": "x"}))),
            ("shop.grid", BlockRef::new("c", json!({"limit": 13}))),
            ("shop.grid", BlockRef::new("d", json!({"limit": 8}))),
        ];
        let resolved = blocks.resolve(&request, &items).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(resolved["a"], json!({"block": "shop.grid", "n": 4}));
        assert_eq!(resolved["c"], Value::Null);
        assert_eq!(resolved["d"]["n"], 8);
        assert!(!resolved.contains_key("b"));

        // Lô lỗi: các khối của loại đó thành `null`, không hỏng cả kết quả.
        let failing = [("shop.grid", BlockRef::new("e", json!({"limit": 1, "fail": true})))];
        let resolved = blocks.resolve(&request, &failing).await;
        assert_eq!(resolved["e"], Value::Null);
    })
    .await
    .unwrap();
}

struct Broken(Vec<BlockType>, bool);

impl Module for Broken {
    fn name(&self) -> &'static str {
        "broken"
    }

    fn blocks(&self) -> Vec<BlockType> {
        self.0.clone()
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        if self.1 {
            context.block_data("broken.text", Arc::new(Grid(Arc::default())))?;
        }
        Ok(())
    }
}

#[test]
fn khai_khoi_sai_thi_khong_khoi_dong() {
    let open = r#"{"type": "object", "properties": {}}"#;
    let cases = [
        BlockType::new("other.text", BlockCategory::Text, "T", "Hint.", TEXT_SCHEMA),
        BlockType::new("broken.open", BlockCategory::Text, "T", "Hint.", open),
        BlockType::new("broken.json", BlockCategory::Text, "T", "Hint.", "{"),
        BlockType::new("broken.vip", BlockCategory::Text, "T", "Hint.", TEXT_SCHEMA).requires(VIP),
    ];
    for case in cases {
        let name = case.name;
        assert!(Modules::new(vec![Box::new(Broken(vec![case], false))]).is_err(), "{name}");
    }
    let twice = BlockType::new("broken.text", BlockCategory::Text, "T", "Hint.", TEXT_SCHEMA);
    assert!(Modules::new(vec![Box::new(Broken(vec![twice, twice], false))]).is_err());
}

#[tokio::test]
async fn khoi_can_du_lieu_phai_gan_block_data() {
    TestDatabase::run(async |test| {
        let declared =
            BlockType::new("broken.text", BlockCategory::Text, "T", "Hint.", TEXT_SCHEMA);
        // Gắn dữ liệu cho khối không khai `with_data`: lỗi lúc khởi động.
        let wrong = TestApp::builder(test).module(Broken(vec![declared], true)).build().await;
        assert!(wrong.is_err());
        // Khai `with_data` mà không gắn: lỗi lúc khởi động.
        let missing =
            TestApp::builder(test).module(Broken(vec![declared.with_data()], false)).build().await;
        let error = missing.err().unwrap();
        assert!(format!("{error:?}").contains("broken.text"), "{error:?}");
    })
    .await
    .unwrap();
}
