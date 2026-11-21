//! Hoá đơn của kỳ thuê bao và huỷ kỳ cùng lúc (rà đồng thời mục 8): hoá đơn phát hành ngoài
//! transaction, kỳ bị huỷ đúng lúc đó thì hoá đơn vừa phát hành phải bị huỷ theo.

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
    let at = chrono::Utc::now();
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
