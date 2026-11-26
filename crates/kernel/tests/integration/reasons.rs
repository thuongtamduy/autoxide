//! Danh sách lý do theo khách (bước 51 P18): mã mặc định tự có, khách thêm mã riêng, mã pháp luật
//! không tắt được, mã đã tắt bị từ chối khi ghi, hai khách không thấy mã của nhau.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use autoxide_kernel::access::Permission;
use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::reasons::{ReasonCodes, ReasonDefault, ReasonList};
use autoxide_kernel::tenancy::{DatabaseRegistry, TenantContext};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::{Value, json};

const SHOP_READ: Permission = Permission::new("shop.read", "See the shop.");

struct Shop(Arc<Mutex<Option<ReasonCodes>>>);

impl Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![SHOP_READ]
    }

    fn reason_lists(&self) -> Vec<ReasonList> {
        vec![ReasonList::new(
            "shop.cancel",
            vec![
                ReasonDefault::new("customer_request", "The customer asked"),
                ReasonDefault::new("out_of_stock", "Out of stock"),
                ReasonDefault::new("fraud", "Suspected fraud").legal(),
            ],
        )]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(context.reason_codes());
        Ok(())
    }
}

fn codes(body: &Value) -> Vec<String> {
    body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|reason| reason["code"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn danh_sach_ly_do_theo_khach() {
    TestDatabase::run(async |test| {
        let handle = Arc::new(Mutex::new(None));
        let app = TestApp::builder(test)
            .tenant("acme")
            .user("manager", 1, &["shop.read", "kernel.reasons.manage"])
            .user("clerk", 2, &["shop.read"])
            .user_in("acme", "other", 3, &["shop.read", "kernel.reasons.manage"])
            .user("nobody", 4, &[])
            .module(Shop(handle.clone()))
            .build()
            .await
            .unwrap();
        let list = "/api/v1/kernel/reasons/shop.cancel";
        let get = async |who: &str, query: &str| {
            let mut request = app.get(&format!("{list}{query}")).token(who);
            if who == "other" {
                request = request.header("autoxide-tenant", "acme");
            }
            request.send().await.unwrap()
        };
        let put = async |who: &str, code: &str, body: Value| {
            let mut request = app.put(&format!("{list}/{code}")).token(who).json(body);
            if who == "other" {
                request = request.header("autoxide-tenant", "acme");
            }
            request.send().await.unwrap()
        };

        // Mã mặc định tự có, đọc được với permission đọc của module.
        let defaults = get("clerk", "").await;
        assert_eq!(defaults.status(), StatusCode::OK, "{}", defaults.json());
        assert_eq!(codes(&defaults.json()), ["customer_request", "out_of_stock", "fraud"]);
        assert_eq!(defaults.json()["data"][2]["legal"], true);
        assert_eq!(get("nobody", "").await.status(), StatusCode::NOT_FOUND);

        // Khách thêm mã riêng, đổi nhãn, tắt một mã; mã pháp luật không tắt được.
        let added = put("manager", "wrong_address", json!({"labels": {"vi": "Sai địa chỉ"}})).await;
        assert_eq!(added.status(), StatusCode::OK, "{}", added.json());
        let renamed = put(
            "manager",
            "customer_request",
            json!({"labels": {"vi": "Khách yêu cầu"}, "active": false}),
        )
        .await;
        assert_eq!(renamed.status(), StatusCode::OK);
        let legal = put("manager", "fraud", json!({"active": false})).await;
        assert_eq!(
            (legal.status(), legal.json()["code"].as_str()),
            (StatusCode::CONFLICT, Some("kernel.reason_legal"))
        );
        let bad = put("manager", "Bad-Code", json!({"labels": {"en": "x"}})).await;
        assert_eq!(
            (bad.status(), bad.json()["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("kernel.reason_invalid"))
        );
        assert_eq!(
            put("clerk", "x", json!({"labels": {"en": "x"}})).await.status(),
            StatusCode::FORBIDDEN
        );

        assert_eq!(
            codes(&get("clerk", "").await.json()),
            ["out_of_stock", "fraud", "wrong_address"]
        );
        let all = get("clerk", "?include_inactive=true").await.json();
        assert_eq!(codes(&all), ["customer_request", "out_of_stock", "fraud", "wrong_address"]);
        let vi = app
            .get(list)
            .token("clerk")
            .header("accept-language", "vi")
            .send()
            .await
            .unwrap()
            .json();
        assert_eq!(vi["data"][2]["label"], "Sai địa chỉ");

        // Service kiểm mã: mã đã tắt bị từ chối, mã riêng của khách được nhận.
        let reasons = handle.lock().unwrap().clone().unwrap();
        let tenant: TenantContext =
            DatabaseRegistry::single(app.database().clone()).default_tenant();
        let refused = reasons.ensure(&tenant, "shop.cancel", "customer_request").await.unwrap_err();
        assert_eq!(refused.code(), "kernel.reason_unknown");
        reasons.ensure(&tenant, "shop.cancel", "wrong_address").await.unwrap();

        // Khách khác không thấy mã riêng hay mã đã tắt của khách này.
        let theirs = get("other", "").await;
        assert_eq!(theirs.status(), StatusCode::OK, "{}", theirs.json());
        assert_eq!(codes(&theirs.json()), ["customer_request", "out_of_stock", "fraud"]);
    })
    .await
    .unwrap();
}

// Danh sách lý do đọc gộp trong bộ nhớ nên có trần ở chỗ ghi: khách thêm tối đa `AUTOXIDE_REASONS_TENANT_MAX` lý do
// riêng mỗi danh sách (409 `kernel.reason_limit_reached`); sửa lý do đã có vẫn được. Trang theo vị trí rồi mã,
// theo `next_cursor` tới hết.
#[tokio::test]
async fn tran_ly_do_rieng_va_phan_trang() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .setting("AUTOXIDE_REASONS_TENANT_MAX", "2")
            .user("manager", 1, &["shop.read", "kernel.reasons.manage"])
            .module(Shop(Arc::new(Mutex::new(None))))
            .build()
            .await
            .unwrap();
        let put = async |code: &str| {
            app.put(&format!("/api/v1/kernel/reasons/shop.cancel/{code}"))
                .token("manager")
                .json(json!({"labels": {"en": code}}))
                .send()
                .await
                .unwrap()
        };
        assert_eq!(put("damaged").await.status(), StatusCode::OK);
        assert_eq!(put("late").await.status(), StatusCode::OK);
        let third = put("wrong_size").await;
        assert_eq!(
            (third.status(), third.json()["code"].as_str()),
            (StatusCode::CONFLICT, Some("kernel.reason_limit_reached"))
        );
        assert_eq!(put("late").await.status(), StatusCode::OK, "sửa lý do đã có không tính thêm");
        assert_eq!(put("out_of_stock").await.status(), StatusCode::OK, "lý do mặc định không tính");

        let mut seen = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut path = "/api/v1/kernel/reasons/shop.cancel?limit=2".to_owned();
            if let Some(cursor) = &cursor {
                path.push_str(&format!("&cursor={cursor}"));
            }
            let page = app.get(&path).token("manager").send().await.unwrap().json();
            seen.extend(codes(&page));
            match page["pagination"]["next_cursor"].as_str() {
                Some(next) => cursor = Some(next.to_owned()),
                None => break,
            }
        }
        assert_eq!(seen, ["customer_request", "out_of_stock", "fraud", "damaged", "late"]);
    })
    .await
    .unwrap();
}
