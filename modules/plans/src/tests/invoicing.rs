//! Hoá đơn của kỳ thuê bao và huỷ kỳ cùng lúc (rà đồng thời mục 8): hoá đơn phát hành ngoài
//! transaction, kỳ bị huỷ đúng lúc đó thì hoá đơn vừa phát hành phải bị huỷ theo.

use autoxide_kernel::time::Timestamp;
use std::sync::{Arc, Mutex};

use autoxide_invoices::api::{
    Document, Invoices, NewDocument, NewDocumentLine, RenderFormat, Rendered,
};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::Method;
use serde_json::{Value, json};
use tokio::sync::Notify;
use uuid::Uuid;

use super::lifecycle::{ok, priced, sql};

/// `invoices` giả: `issue` dừng giữa chừng (báo `entered`, chờ `release`) để test chen lệnh huỷ kỳ
/// vào đúng khoảng hở giữa lúc job đọc kỳ và lúc ghi `invoice_id`.
#[derive(Default)]
struct SlowInvoices {
    documents: Mutex<Vec<Document>>,
    cancelled: Mutex<Vec<Uuid>>,
    entered: Notify,
    release: Notify,
}

fn document(draft: &NewDocument) -> Document {
    let at = Timestamp::now();
    let zero = Money::new(0, Currency::VND);
    Document {
        id: Uuid::now_v7(),
        org_id: draft.org.as_uuid(),
        on_behalf_of_org_id: None,
        kind: draft.kind.clone(),
        relation: None,
        related_document_id: None,
        direction: "increase".to_owned(),
        series_id: None,
        symbol: None,
        number: None,
        status: "draft".to_owned(),
        issue_date: None,
        prices_include_tax: draft.prices_include_tax,
        currency: "VND".to_owned(),
        subtotal: zero,
        tax: zero,
        total: zero,
        buyer: draft.buyer.clone(),
        seller: json!({}),
        lines: Vec::new(),
        einvoice_provider: None,
        provider_ref: None,
        provider_number: None,
        authority_code: None,
        lookup_code: None,
        rejection_reason: None,
        retention_until: None,
        exchange_rate: None,
        exchange_rate_id: None,
        base_total: None,
        source_module: draft.source_module.clone(),
        source_id: draft.source_id.clone(),
        reference: draft.reference.clone(),
        issued_at: None,
        created_at: at,
        updated_at: at,
    }
}

#[async_trait::async_trait]
impl Invoices for SlowInvoices {
    async fn create_document(
        &self,
        _: &TenantContext,
        _: Actor,
        draft: NewDocument,
    ) -> Result<Submitted<Document>, Error> {
        let mut documents = self.documents.lock().unwrap();
        if let Some(found) = documents.iter().find(|found| found.reference == draft.reference) {
            return Ok(Submitted::Replayed(found.clone()));
        }
        let made = document(&draft);
        documents.push(made.clone());
        Ok(Submitted::Created(made))
    }

    async fn issue(&self, _: &TenantContext, _: Actor, id: Uuid) -> Result<Document, Error> {
        self.entered.notify_one();
        self.release.notified().await;
        let mut documents = self.documents.lock().unwrap();
        let found = documents.iter_mut().find(|found| found.id == id).unwrap();
        found.number = Some(1);
        found.symbol = Some("1C26TAA".to_owned());
        found.status = "issued".to_owned();
        Ok(found.clone())
    }

    async fn document(&self, _: &TenantContext, id: Uuid) -> Result<Option<Document>, Error> {
        Ok(self.documents.lock().unwrap().iter().find(|found| found.id == id).cloned())
    }

    async fn adjust(
        &self,
        _: &TenantContext,
        _: Actor,
        _: Uuid,
        _: &str,
        _: Vec<NewDocumentLine>,
        _: String,
    ) -> Result<Submitted<Document>, Error> {
        Err(Error::internal("not used by plans"))
    }

    async fn cancel(
        &self,
        _: &TenantContext,
        _: Actor,
        id: Uuid,
        _: String,
    ) -> Result<Document, Error> {
        self.cancelled.lock().unwrap().push(id);
        let mut documents = self.documents.lock().unwrap();
        let found = documents.iter_mut().find(|found| found.id == id).unwrap();
        found.status = "cancelled".to_owned();
        Ok(found.clone())
    }

    async fn render(
        &self,
        _: &TenantContext,
        _: Uuid,
        _: RenderFormat,
        _: &str,
    ) -> Result<Rendered, Error> {
        Err(Error::internal("not used by plans"))
    }

    async fn archive(&self, _: &TenantContext, _: Actor) -> Result<Uuid, Error> {
        Err(Error::internal("not used by plans"))
    }
}

const OPS: [&str; 4] = [
    "plans.plans.manage",
    "plans.subscriptions.manage",
    "plans.subscriptions.read",
    "kernel.tenants.read",
];

async fn app(test: &TestDatabase, invoices: &Arc<SlowInvoices>) -> TestApp {
    let provided: Arc<dyn Invoices> = invoices.clone();
    TestApp::builder(test)
        .setting("AUTOXIDE_PLANS_CACHE_SECONDS", "0")
        .provide::<dyn Invoices>(provided)
        .module(crate::module())
        .tenant("acme")
        .user("ops", 1, &OPS)
        .user_in("acme", "owner", 2, &["plans.billing.read", "plans.billing.manage"])
        .build()
        .await
        .unwrap()
}

#[tokio::test]
async fn huy_ky_dung_luc_phat_hanh_hoa_don_thi_huy_ca_hoa_don() {
    TestDatabase::run(async |test| {
        let invoices = Arc::new(SlowInvoices::default());
        let app = app(test, &invoices).await;
        let pro = priced(&app, "pro", 199_000, 0).await;
        let acme = app.tenant("acme").await.unwrap().id();
        let body = json!({"plan_id": pro.0, "price_id": pro.1, "trial": false});
        ok(&app, Method::PUT, &format!("/plans/tenants/{acme}/subscription"), "ops", Some(body))
            .await;
        // Hết kỳ: job gia hạn mở kỳ mới và xếp job lập hoá đơn; `issue` dừng giữa chừng.
        sql(&app, "UPDATE plans_subscription SET current_period_end = '2000-01-01T00:00:00Z'")
            .await;
        sql(&app, "DELETE FROM plans_jobs").await;
        let worker = app.modules().worker(4).unwrap();
        let run = async {
            for _ in 0..5 {
                if worker.run_once().await.unwrap() == 0 {
                    break;
                }
            }
        };
        let void = async {
            invoices.entered.notified().await;
            // Job đã đọc kỳ (mở, chưa có hoá đơn) và đang phát hành: nhà vận hành huỷ kỳ.
            let periods = ok(&app, Method::GET, "/plans/billing-periods", "owner", None).await;
            let period: Value = periods["data"]
                .as_array()
                .unwrap()
                .iter()
                .find(|period| period["status"] == "open")
                .cloned()
                .unwrap();
            let id = period["id"].as_str().unwrap().to_owned();
            ok(&app, Method::POST, &format!("/plans/billing-periods/{id}/void"), "ops", None).await;
            invoices.release.notify_one();
            id
        };
        let ((), id) = tokio::join!(run, void);
        // Kỳ đã huỷ vẫn ghi hoá đơn đã phát hành (để tra), nhưng hoá đơn đó phải bị huỷ.
        let period =
            ok(&app, Method::GET, &format!("/plans/billing-periods/{id}"), "owner", None).await;
        assert_eq!(period["data"]["status"], "void", "{period}");
        let invoice: Uuid = period["data"]["invoice_id"].as_str().unwrap().parse().unwrap();
        assert_eq!(*invoices.cancelled.lock().unwrap(), vec![invoice]);
    })
    .await
    .unwrap();
}

/// `invoices` giả của nơi luật cấm huỷ hoá đơn điện tử đã gửi: `cancel` từ chối, `adjust` chống trùng theo
/// `reference`; lần phát hành điều chỉnh đầu tiên xong việc rồi mới báo lỗi (như lỗi sau commit).
#[derive(Default)]
struct NoCancelInvoices {
    documents: Mutex<Vec<Document>>,
    /// (`reference`, dòng) của mỗi điều chỉnh đã lập.
    adjustments: Mutex<Vec<(String, Vec<NewDocumentLine>)>>,
    fail_after_issue: Mutex<usize>,
}

/// Lỗi của module `invoices` (không phải mã của `plans`): dựng như `invoices` trả về.
fn invoices_error(code: &str, status: &str) -> Error {
    Error::conflict(code.to_owned(), "The document cannot be changed.").with_param("status", status)
}

#[async_trait::async_trait]
impl Invoices for NoCancelInvoices {
    async fn create_document(
        &self,
        _: &TenantContext,
        _: Actor,
        draft: NewDocument,
    ) -> Result<Submitted<Document>, Error> {
        let mut documents = self.documents.lock().unwrap();
        if let Some(found) = documents.iter().find(|found| found.reference == draft.reference) {
            return Ok(Submitted::Replayed(found.clone()));
        }
        let mut made = document(&draft);
        let line =
            |kind: &str, unit: Option<&str>, price: i64| autoxide_invoices::api::DocumentLine {
                line_no: 1,
                kind: kind.to_owned(),
                item_ref: None,
                description: format!("{kind} line"),
                unit: unit.map(str::to_owned),
                quantity: "1".to_owned(),
                unit_price: Money::new(price, Currency::VND),
                discount: Money::new(0, Currency::VND),
                net: Money::new(price, Currency::VND),
                tax_category: Some("vat.standard".to_owned()),
                rate_percent: None,
                tax: Money::new(0, Currency::VND),
                total: Money::new(price, Currency::VND),
            };
        made.lines = vec![line("service", Some("period"), 199_000), line("note", None, 0)];
        documents.push(made.clone());
        Ok(Submitted::Created(made))
    }

    async fn issue(&self, _: &TenantContext, _: Actor, id: Uuid) -> Result<Document, Error> {
        let mut documents = self.documents.lock().unwrap();
        let found = documents.iter_mut().find(|found| found.id == id).unwrap();
        let first = found.status == "draft";
        found.status = "issued".to_owned();
        found.number = Some(1);
        let issued = found.clone();
        if let Some(original) = issued.related_document_id {
            let original = documents.iter_mut().find(|found| found.id == original).unwrap();
            original.status = "adjusted".to_owned();
            let mut fail = self.fail_after_issue.lock().unwrap();
            if first && *fail > 0 {
                *fail -= 1;
                return Err(Error::internal("simulated failure after the adjustment was issued"));
            }
        }
        Ok(issued)
    }

    async fn document(&self, _: &TenantContext, id: Uuid) -> Result<Option<Document>, Error> {
        Ok(self.documents.lock().unwrap().iter().find(|found| found.id == id).cloned())
    }

    async fn adjust(
        &self,
        _: &TenantContext,
        _: Actor,
        original: Uuid,
        direction: &str,
        lines: Vec<NewDocumentLine>,
        reference: String,
    ) -> Result<Submitted<Document>, Error> {
        let mut documents = self.documents.lock().unwrap();
        if let Some(found) = documents.iter().find(|found| found.reference == reference) {
            return Ok(Submitted::Replayed(found.clone()));
        }
        let status = documents.iter().find(|found| found.id == original).unwrap().status.clone();
        if status != "issued" {
            return Err(invoices_error("invoices.not_correctable", &status));
        }
        let mut made = documents.iter().find(|found| found.id == original).unwrap().clone();
        made.id = Uuid::now_v7();
        made.reference = reference.clone();
        made.status = "draft".to_owned();
        made.lines = Vec::new();
        made.related_document_id = Some(original);
        made.direction = direction.to_owned();
        documents.push(made.clone());
        self.adjustments.lock().unwrap().push((reference, lines));
        Ok(Submitted::Created(made))
    }

    async fn cancel(
        &self,
        _: &TenantContext,
        _: Actor,
        id: Uuid,
        _: String,
    ) -> Result<Document, Error> {
        let documents = self.documents.lock().unwrap();
        let status = documents.iter().find(|found| found.id == id).unwrap().status.clone();
        let code = if status == "issued" {
            "invoices.einvoice_not_cancellable"
        } else {
            "invoices.not_cancellable"
        };
        Err(invoices_error(code, &status))
    }

    async fn render(
        &self,
        _: &TenantContext,
        _: Uuid,
        _: RenderFormat,
        _: &str,
    ) -> Result<Rendered, Error> {
        Err(Error::internal("not used by plans"))
    }

    async fn archive(&self, _: &TenantContext, _: Actor) -> Result<Uuid, Error> {
        Err(Error::internal("not used by plans"))
    }
}

/// Chạy job tới khi hết; job lỗi được dời về quá khứ để chạy lại ngay.
async fn run_jobs(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap();
    for _ in 0..10 {
        sql(
            app,
            "UPDATE plans_jobs SET run_at = '2000-01-01T00:00:00Z' WHERE completed_at IS NULL",
        )
        .await;
        if worker.run_once().await.unwrap() == 0 {
            break;
        }
    }
}

/// Huỷ kỳ có hoá đơn điện tử không huỷ được: lập đúng MỘT điều chỉnh giảm, kể cả khi lần đầu hỏng sau lúc đã
/// phát hành (job chạy lại thấy gốc `adjusted`, đi lại đường điều chỉnh và được trả lại đúng chứng từ đó).
/// Dòng điều chỉnh giữ đơn vị `period`, bỏ dòng ghi chú.
#[tokio::test]
async fn huy_ky_dieu_chinh_giam_mot_lan_du_chay_lai() {
    TestDatabase::run(async |test| {
        let invoices = Arc::new(NoCancelInvoices::default());
        let provided: Arc<dyn Invoices> = invoices.clone();
        let app = TestApp::builder(test)
            .setting("AUTOXIDE_PLANS_CACHE_SECONDS", "0")
            .provide::<dyn Invoices>(provided)
            .module(crate::module())
            .tenant("acme")
            .user("ops", 1, &OPS)
            .user_in("acme", "owner", 2, &["plans.billing.read", "plans.billing.manage"])
            .build()
            .await
            .unwrap();
        let pro = priced(&app, "pro", 199_000, 0).await;
        let acme = app.tenant("acme").await.unwrap().id();
        let body = json!({"plan_id": pro.0, "price_id": pro.1, "trial": false});
        ok(&app, Method::PUT, &format!("/plans/tenants/{acme}/subscription"), "ops", Some(body))
            .await;
        // Hết kỳ: job gia hạn mở kỳ mới và lập hoá đơn.
        sql(&app, "UPDATE plans_subscription SET current_period_end = '2000-01-01T00:00:00Z'")
            .await;
        run_jobs(&app).await;
        let periods = ok(&app, Method::GET, "/plans/billing-periods", "owner", None).await;
        let period = periods["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|period| !period["invoice_id"].is_null())
            .cloned()
            .unwrap_or_else(|| panic!("a period with an invoice: {periods}"));
        let id = period["id"].as_str().unwrap();
        *invoices.fail_after_issue.lock().unwrap() = 1;
        ok(&app, Method::POST, &format!("/plans/billing-periods/{id}/void"), "ops", None).await;
        run_jobs(&app).await;

        let adjustments = invoices.adjustments.lock().unwrap().clone();
        assert_eq!(adjustments.len(), 1, "{adjustments:?}");
        assert_eq!(adjustments[0].0, format!("plans:{id}:void"));
        let lines = &adjustments[0].1;
        assert_eq!(lines.len(), 1, "dòng ghi chú bị bỏ: {lines:?}");
        assert_eq!(lines[0].unit.as_deref(), Some("period"));
        assert_eq!(lines[0].kind, "service");
        assert_eq!(*invoices.fail_after_issue.lock().unwrap(), 0, "lần hỏng đã xảy ra");
        let open = invoices
            .documents
            .lock()
            .unwrap()
            .iter()
            .filter(|found| found.related_document_id.is_some() && found.status != "issued")
            .count();
        assert_eq!(open, 0, "điều chỉnh đã phát hành");
    })
    .await
    .unwrap();
}
