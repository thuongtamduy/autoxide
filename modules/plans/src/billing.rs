//! Tính tiền thuê bao SaaS (bước 35 mục 4, D-345): mỗi kỳ thanh toán thành một hoá đơn do khách vận
//! hành (nền tảng) lập cho khách thuê qua `dyn Invoices`, thu qua cổng thanh toán của khách vận hành
//! (`dyn Ledger::create_payment`), đánh dấu đã trả khi `ledger` báo đã thu.
//!
//! - Kỳ mở → job `plans.invoice_period` lập + phát hành hoá đơn (số không nhảy, thuế qua `TaxEngine`,
//!   ghi sổ — đều là việc của `invoices`). Khách vận hành chưa có ký hiệu hoá đơn thì bỏ qua (ghi log);
//!   kỳ vẫn trả được.
//! - `POST /billing-periods/{id}/pay` → lượt thu ở cổng của khách vận hành, trả `checkout_url`.
//! - Event `ledger.payment_succeeded` (`source_module = plans`, `source_id` = ID kỳ) → `mark_paid`.
//!   Event đến lại vô hại (kỳ đã trả thì bỏ qua). Số thực thu (`amount`) nhỏ hơn số của kỳ (hay
//!   khác loại tiền) thì kỳ vẫn mở.
//!
//! Giá của gói là số khách trả (đã gồm thuế). `plans` không tự đánh số, không tự tính thuế, không tự
//! ghi sổ.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_invoices::api::{Buyer, NewDocument, NewDocumentLine};
use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Delivered, EventHandler};
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::money::Money;
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{TenantContext, TenantId, root_of};
use autoxide_ledger::api::NewPayment;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ConnectionTrait, EntityTrait};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::{billing_period, plan};
use crate::permissions::BILLING_MANAGE;
use crate::service::{PlansService, now};

pub(crate) const INVOICE_PERIOD: &str = "plans.invoice_period";

/// Job lập hoá đơn cho một kỳ.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct InvoicePeriod {
    pub period_id: Uuid,
}

impl Job for InvoicePeriod {
    const KIND: &'static str = INVOICE_PERIOD;
}

/// A payment started for a billing period.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PayResponse {
    pub period_id: Uuid,
    pub payment_id: Uuid,
    /// `pending` until the partner reports the payment.
    pub status: String,
    pub amount: Money,
    /// The partner's page where you pay.
    pub checkout_url: Option<String>,
    /// QR payload, when the partner gives one.
    pub qr_payload: Option<String>,
    pub expires_at: Timestamp,
}

pub(crate) fn billing_profile_required() -> Error {
    Error::conflict(
        "plans.billing_profile_required",
        "Fill in the billing profile (name, email) before paying.",
    )
}

pub(crate) fn payments_unavailable() -> Error {
    Error::unavailable(
        "plans.payments_unavailable",
        "Online payment is not set up on this server. Pay by bank transfer as agreed with the \
         operator.",
    )
}

impl PlansService {
    /// Khách vận hành (bên bán thuê bao).
    fn operator(&self) -> TenantContext {
        self.databases.default_tenant()
    }

    /// Xếp job xử lý hoá đơn của kỳ vừa huỷ (trong transaction huỷ kỳ): job `plans.invoice_period` thấy kỳ
    /// `void` có hoá đơn thì huỷ / điều chỉnh giảm nó; lỗi giữa chừng thì job thử lại. Khoá `void-invoice:<kỳ>`: một
    /// job.
    pub(crate) async fn enqueue_void_invoice<C: ConnectionTrait>(
        &self,
        connection: &C,
        period_id: Uuid,
    ) -> Result<(), Error> {
        if self.invoices.is_none() {
            return Ok(());
        }
        self.jobs
            .enqueue_database(
                connection,
                &InvoicePeriod { period_id },
                EnqueueOptions::default().unique_key(format!("void-invoice:{period_id}")),
            )
            .await
            .map(|_| ())
    }

    /// Xếp job lập hoá đơn cho kỳ `period_id` (trong transaction mở kỳ).
    pub(crate) async fn enqueue_invoice<C: ConnectionTrait>(
        &self,
        connection: &C,
        period_id: Uuid,
    ) -> Result<(), Error> {
        if self.invoices.is_none() {
            return Ok(());
        }
        self.jobs
            .enqueue_database(
                connection,
                &InvoicePeriod { period_id },
                EnqueueOptions::default().unique_key(format!("invoice:{period_id}")),
            )
            .await
            .map(|_| ())
    }

    /// Lập và phát hành hoá đơn cho kỳ `period_id` (job). Chạy lại vô hại: `reference` của chứng từ
    /// là `plans:{period_id}`, phát hành lại trả cùng số.
    pub(crate) async fn invoice_period(&self, period_id: Uuid) -> Result<(), Error> {
        let Some(invoices) = &self.invoices else { return Ok(()) };
        let control = self.databases.control();
        let Some(period) =
            billing_period::Entity::find_by_id(period_id).one(control.write()).await?
        else {
            return Ok(());
        };
        if period.status == "void" && period.invoice_id.is_some() {
            // Lượt trước gắn hoá đơn vào kỳ vừa bị huỷ nhưng chưa huỷ xong hoá đơn (lỗi giữa
            // chừng): huỷ lại — `cancel` của `invoices` gọi lại vô hại.
            return self.cancel_invoice(&period).await;
        }
        if period.invoice_id.is_some() || period.status == "void" || period.amount_minor <= 0 {
            return Ok(());
        }
        let found = plan::Entity::find_by_id(period.plan_id).one(control.read()).await?;
        let name = found.as_ref().map_or("Subscription", |found| found.name.as_str());
        let tenant = TenantId::from_uuid(period.tenant_id);
        let buyer = match self.billing_profile(tenant).await? {
            Some(profile) => {
                let mut buyer = match &profile.tax_code {
                    Some(code) => Buyer::business(profile.legal_name.clone(), code.clone()),
                    None => Buyer::consumer(Some(profile.legal_name.clone())),
                };
                buyer.email = Some(profile.email.clone());
                let parts: Vec<&str> = [
                    profile.address_line.as_deref(),
                    profile.address_commune_name.as_deref(),
                    profile.address_province_name.as_deref(),
                ]
                .into_iter()
                .flatten()
                .filter(|part| !part.is_empty())
                .collect();
                buyer.address = (!parts.is_empty()).then(|| parts.join(", "));
                buyer
            }
            None => {
                let name =
                    self.databases.tenant(tenant).await.map(|found| found.name().to_owned()).ok();
                Buyer::consumer(name)
            }
        };
        let price = Money::from_columns(period.amount_minor, &period.amount_currency)?;
        let description = format!(
            "{name} ({} – {})",
            period.period_start.format("%Y-%m-%d"),
            period.period_end.format("%Y-%m-%d")
        );
        let line = NewDocumentLine::new(description, rust_decimal::Decimal::ONE, price)
            .service()
            .unit("period")
            .tax_category(self.tax_category.clone());
        let operator = self.operator();
        let draft = NewDocument::new(
            root_of(operator.id()),
            "vat_invoice",
            price.currency(),
            buyer,
            vec![line],
            "plans",
            format!("plans:{period_id}"),
        )
        .source_id(period_id.to_string())
        .prices_include_tax();
        let document =
            invoices.create_document(&operator, Actor::system(), draft).await?.into_inner();
        let issued = match invoices.issue(&operator, Actor::system(), document.id).await {
            Ok(issued) => issued,
            Err(error) if error.code() == "invoices.series_not_found" => {
                tracing::warn!(
                    period_id = %period_id,
                    "no invoice series for subscription invoices; create one with POST /api/v1/invoices/series"
                );
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let number = match (&issued.symbol, issued.number) {
            (Some(symbol), Some(number)) => Some(format!("{symbol}-{number:07}")),
            _ => None,
        };
        // Hoá đơn phát hành ngoài transaction (có thể gọi nhà cung cấp): kỳ có thể vừa bị huỷ trong
        // lúc đó, và `void_period` đã đọc `invoice_id` trống nên không huỷ hoá đơn nào. Khoá kỳ
        // (cùng khoá với `void_period`, D-181) rồi gắn hoá đơn; kỳ đã huỷ thì huỷ luôn hoá đơn vừa
        // phát hành — kỳ huỷ không bao giờ còn hoá đơn GTGT hiệu lực (cả hai DB).
        let transaction = control.begin_write().await?;
        let Some(current) =
            rows::lock_one(&transaction, billing_period::Entity::find_by_id(period_id)).await?
        else {
            return Ok(());
        };
        let mut row: billing_period::ActiveModel = current.into();
        row.invoice_id = Set(Some(issued.id));
        row.invoice_number = Set(number);
        row.updated_at = Set(now());
        let attached = row.update(&transaction).await?;
        transaction.commit().await?;
        if attached.status == "void" {
            self.cancel_invoice(&attached).await?;
        }
        Ok(())
    }

    /// Khách trả kỳ `id` qua cổng thanh toán của khách vận hành.
    pub(crate) async fn pay(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        method: Option<String>,
        return_url: Option<String>,
    ) -> Result<PayResponse, Error> {
        principal.require(&BILLING_MANAGE)?;
        let period = self.billing_period(Some(tenant.id()), id).await?;
        if period.status != "open" {
            return Err(crate::lifecycle::billing_period_not_payable());
        }
        let Some(ledger) = &self.ledger else { return Err(payments_unavailable()) };
        let profile =
            self.billing_profile(tenant.id()).await?.ok_or_else(billing_profile_required)?;
        if profile.legal_name.trim().is_empty() || profile.email.trim().is_empty() {
            return Err(billing_profile_required());
        }
        let amount = Money::from_columns(period.amount_minor, &period.amount_currency)?;
        let method = method.unwrap_or_else(|| "card".to_owned());
        if method.is_empty() || method.len() > 32 {
            return Err(Error::validation(vec![FieldError::new(
                "method",
                "length",
                "must be 1 to 32 characters",
            )]));
        }
        let operator = self.operator();
        // Lượt thu đã hết hạn / thất bại thì lập lượt mới (hậu tố tăng dần); lượt còn chờ thì trả lại.
        for attempt in 1..=20 {
            let reference = format!("plans:{id}:{attempt}");
            let mut payment = NewPayment::new(
                root_of(operator.id()),
                method.clone(),
                amount,
                "Subscription",
                "plans",
                reference,
            )
            .source_id(id.to_string());
            if let Some(url) = &return_url {
                payment = payment.return_url(url.clone());
            }
            let found =
                match ledger.create_payment(&operator, Actor::from(principal), payment).await {
                    Ok(found) => found.into_inner(),
                    // Gửi lại với nội dung khác (vd phương thức khác) → lượt kế tiếp.
                    Err(error) if error.code() == "reference_reused" => continue,
                    Err(error) => return Err(error),
                };
            if ["failed", "expired"].contains(&found.status.as_str()) {
                continue;
            }
            return Ok(PayResponse {
                period_id: id,
                payment_id: found.id,
                status: found.status,
                amount: found.amount,
                checkout_url: found.checkout_url,
                qr_payload: found.qr_payload,
                expires_at: found.expires_at,
            });
        }
        Err(Error::conflict(
            "plans.too_many_payment_attempts",
            "This billing period has had too many payment attempts. Contact the operator.",
        ))
    }

    /// Huỷ hoá đơn của kỳ vừa huỷ (nếu có).
    pub(crate) async fn cancel_invoice(&self, period: &billing_period::Model) -> Result<(), Error> {
        let (Some(invoices), Some(document)) = (&self.invoices, period.invoice_id) else {
            return Ok(());
        };
        let operator = self.operator();
        match invoices
            .cancel(&operator, Actor::system(), document, "Billing period voided".to_owned())
            .await
        {
            Ok(_) => Ok(()),
            // Hoá đơn điện tử đã gửi cơ quan thuế mà luật không cho huỷ (Việt Nam từ 01/6/2025): lập điều
            // chỉnh giảm toàn bộ. `reference` cố định nên job chạy lại không lập hai lần.
            // Lượt trước đã điều chỉnh xong (gốc thành `adjusted`): đi lại đường điều chỉnh — cùng `reference`
            // nên trả lại đúng điều chỉnh đó, phát hành lại trả cùng số.
            Err(error)
                if error.code() == "invoices.einvoice_not_cancellable"
                    || error.code() == "invoices.not_cancellable" =>
            {
                let Some(original) = invoices.document(&operator, document).await? else {
                    return Ok(());
                };
                if error.code() == "invoices.not_cancellable" && original.status != "adjusted" {
                    return Err(error);
                }
                // Dòng ghi chú (`note`) không có tiền: không điều chỉnh.
                let lines = original
                    .lines
                    .iter()
                    .filter(|line| line.kind != "note")
                    .map(|line| {
                        let quantity = line
                            .quantity
                            .parse::<rust_decimal::Decimal>()
                            .map_err(|_| Error::internal("an invoice line has a bad quantity"))?;
                        let mut back = NewDocumentLine::new(
                            line.description.clone(),
                            quantity,
                            line.unit_price,
                        );
                        if line.kind == "service" {
                            back = back.service();
                        }
                        if let Some(category) = &line.tax_category {
                            back = back.tax_category(category.clone());
                        }
                        if let Some(unit) = &line.unit {
                            back = back.unit(unit.clone());
                        }
                        if let Some(item) = &line.item_ref {
                            back = back.item_ref(item.clone());
                        }
                        if line.discount.is_positive() {
                            back.discount = Some(line.discount);
                        }
                        Ok(back)
                    })
                    .collect::<Result<Vec<_>, Error>>()?;
                let reference = format!("plans:{}:void", period.id);
                let adjustment = invoices
                    .adjust(&operator, Actor::system(), document, "decrease", lines, reference)
                    .await?
                    .into_inner();
                invoices.issue(&operator, Actor::system(), adjustment.id).await.map(|_| ())
            }
            Err(error) => Err(error),
        }
    }
}

/// Job `plans.invoice_period`.
pub(crate) struct InvoicePeriodJob(pub Arc<PlansService>);

#[async_trait::async_trait]
impl JobHandler for InvoicePeriodJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let payload: InvoicePeriod =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        self.0.invoice_period(payload.period_id).await
    }
}

/// Nghe `ledger.payment_succeeded`: kỳ của `plans` đã được thu.
pub(crate) struct PaymentListener(pub Arc<PlansService>);

#[async_trait::async_trait]
impl EventHandler for PaymentListener {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        if event.payload["source_module"] != "plans" {
            return Ok(());
        }
        let Some(period) =
            event.payload["source_id"].as_str().and_then(|id| id.parse::<Uuid>().ok())
        else {
            return Ok(());
        };
        let reference = event.payload["provider_ref"]
            .as_str()
            .or_else(|| event.payload["payment_id"].as_str())
            .unwrap_or("payment")
            .to_owned();
        // Số thực thu (`ledger` ghi cả khoản thiếu khi đối soát `accept_received`). Event không
        // mang số (bản cũ) thì coi như thu đủ — như trước.
        let received = match event.payload.get("amount") {
            Some(amount) if !amount.is_null() => {
                Some(serde_json::from_value::<Money>(amount.clone()).map_err(Error::internal)?)
            }
            _ => None,
        };
        match self.0.mark_paid(Actor::system(), period, reference, None, received).await {
            Ok(_) => Ok(()),
            // Thu thiếu: kỳ vẫn mở (khách trả nốt hay nhà vận hành thu tay); ghi lại để đối soát.
            Err(error) if error.code() == "plans.billing_period_underpaid" => {
                tracing::warn!(billing_period_id = %period, "payment does not cover the billing period");
                Ok(())
            }
            // Đã trả / đã huỷ (event đến lại): không làm gì.
            Err(error) if error.code() == "plans.billing_period_not_payable" => Ok(()),
            Err(error) if error.code() == "plans.billing_period_not_found" => Ok(()),
            Err(error) => Err(error),
        }
    }
}
