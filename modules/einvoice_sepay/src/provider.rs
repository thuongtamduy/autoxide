//! `EInvoiceProvider` của SePay eInvoice: gửi hoá đơn (mới, thay thế, điều chỉnh), hỏi trạng thái,
//! tải file. Mọi lời gọi dùng tài khoản của đúng khách.

use autoxide_kernel::connections::ConnectionProbe;
use autoxide_kernel::einvoice::{
    EInvoice, EInvoiceCancel, EInvoiceCapabilities, EInvoiceFormat, EInvoiceKind, EInvoiceProvider,
    EInvoiceStatus, IssuedEInvoice, not_supported, provider_unavailable, rejected,
};
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{TenantContext, TenantId};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;

use crate::api::PROVIDER;
use crate::client::{Call, Client, error_code, failure};
use crate::settings::PROVIDER_ACCOUNT_ID;
use crate::wire;

pub(crate) struct SepayEInvoice {
    client: Client,
}

impl SepayEInvoice {
    pub(crate) fn new(client: Client) -> Self {
        SepayEInvoice { client }
    }

    /// JSON `data` của một câu trả lời thành công; không thành công → lỗi của hợp đồng.
    async fn data(
        &self,
        tenant: &TenantContext,
        call: Call<'_>,
        purpose: &'static str,
    ) -> Result<Value, Error> {
        let reply = self.client.call(tenant, call, purpose).await?;
        if !reply.is_success() {
            return Err(failure(&reply));
        }
        reply
            .json()
            .and_then(|mut json| json.get_mut("data").map(Value::take))
            .ok_or_else(provider_unavailable)
    }

    /// Tài khoản nhà cung cấp hoá đơn để phát hành: khách đã chọn thì dùng nó; chưa chọn thì chỉ tự
    /// lấy khi tài khoản SePay có ĐÚNG MỘT tài khoản đang hoạt động — không đoán giữa nhiều tài khoản.
    async fn provider_account(&self, tenant: &TenantContext) -> Result<String, Error> {
        if let Some(chosen) = self.client.accounts.get(tenant, &PROVIDER_ACCOUNT_ID).await? {
            return Ok(chosen.as_str().trim().to_owned());
        }
        let listed =
            self.data(tenant, Call::Get("/v1/provider-accounts"), "list_provider_accounts").await?;
        let active: Vec<&str> = listed
            .get("items")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter(|item| item.get("active").and_then(Value::as_bool) == Some(true))
                    .filter_map(|item| item.get("id").and_then(Value::as_str))
                    .collect()
            })
            .unwrap_or_default();
        match active.as_slice() {
            [only] => Ok((*only).to_owned()),
            _ => Err(autoxide_kernel::provider::not_configured(PROVIDER_ACCOUNT_ID.name())),
        }
    }

    /// Ký hiệu hoá đơn của mẫu `template` ở tài khoản `account` (API "chi tiết tài khoản").
    async fn series(
        &self,
        tenant: &TenantContext,
        account: &str,
        template: &str,
    ) -> Result<String, Error> {
        let path = format!("/v1/provider-accounts/{account}");
        let detail = self.data(tenant, Call::Get(&path), "get_provider_account").await?;
        detail
            .get("templates")
            .and_then(Value::as_array)
            .and_then(|templates| {
                templates.iter().find(|found| {
                    found.get("template_code").and_then(Value::as_str) == Some(template)
                })
            })
            .and_then(|found| found.get("invoice_series").and_then(Value::as_str))
            .map(str::to_owned)
            .ok_or_else(|| {
                rejected(format!(
                    "the SePay eInvoice provider account has no invoice template {template}"
                ))
            })
    }

    /// Trạng thái theo chi tiết hoá đơn (`GET /v1/invoices/{reference_code}`).
    async fn status_by_reference(
        &self,
        tenant: &TenantContext,
        reference: &str,
    ) -> Result<EInvoiceStatus, Error> {
        let path = format!("/v1/invoices/{reference}");
        let reply = self.client.call(tenant, Call::Get(&path), "get_invoice").await?;
        if reply.status == 404 {
            // Chưa có hoá đơn nào được ghi nhận cho mã này.
            return Ok(EInvoiceStatus::Pending);
        }
        if !reply.is_success() {
            return Err(failure(&reply));
        }
        let detail = reply
            .json()
            .and_then(|mut json| json.get_mut("data").map(Value::take))
            .ok_or_else(provider_unavailable)?;
        let text = |name: &str| detail.get(name).and_then(Value::as_str).map(str::trim);
        let authority_code = text("tax_authority_code").filter(|code| !code.is_empty());
        Ok(match (text("status"), text("provider_status")) {
            (Some("cancelled"), _) => {
                EInvoiceStatus::Rejected { reason: "cancelled at SePay eInvoice".to_owned() }
            }
            // `provider_status`: 2 — lỗi ký, 6 — không đủ điều kiện cấp mã.
            (Some("issued"), Some(code @ ("2" | "6"))) => EInvoiceStatus::Rejected {
                reason: format!("SePay eInvoice provider status {code}"),
            },
            // 3 — chưa gửi cơ quan thuế, 5 — cơ quan thuế chưa phản hồi.
            (Some("issued"), Some("3" | "5")) if authority_code.is_none() => {
                EInvoiceStatus::Pending
            }
            (Some("issued"), _) => EInvoiceStatus::Accepted {
                authority_code: authority_code.map(str::to_owned),
                // `invoice_number` là chuỗi số (vd `"589"`); `"0"` là nháp.
                number: text("invoice_number")
                    .and_then(|number| number.parse::<i64>().ok())
                    .filter(|number| *number > 0),
            },
            // Còn là nháp ở SePay (một lần gửi trước dừng giữa chừng): bộ nối không tự phát hành
            // nháp. Báo rõ để người dùng phát hành hoặc xoá nháp ở SePay rồi gửi lại.
            (Some("draft"), _) => EInvoiceStatus::Rejected {
                reason: "an unissued draft of this invoice exists at SePay eInvoice; issue or \
                         delete it there, then send the invoice again"
                    .to_owned(),
            },
            _ => EInvoiceStatus::Pending,
        })
    }
}

#[autoxide_kernel::async_trait]
impl EInvoiceProvider for SepayEInvoice {
    fn provider(&self) -> &'static str {
        PROVIDER
    }

    fn capabilities(&self) -> EInvoiceCapabilities {
        // Có điều chỉnh và thay thế; tài liệu không có API huỷ hoá đơn đã phát hành.
        let mut capabilities = EInvoiceCapabilities::new(&[EInvoiceKind::Vat, EInvoiceKind::Sales])
            .assigning_numbers();
        capabilities.adjust = true;
        capabilities.replace = true;
        capabilities
    }

    async fn issue(&self, tenant: TenantId, invoice: &EInvoice) -> Result<IssuedEInvoice, Error> {
        let tenant = self.client.accounts.tenant(tenant).await?;
        // Kiểm những gì bộ nối không gửi được TRƯỚC khi gọi ra ngoài.
        if invoice.currency != autoxide_kernel::money::Currency::VND {
            return Err(not_supported(PROVIDER));
        }
        let template = wire::template_code(invoice.kind)?;
        let (path, extra) = wire::endpoint(invoice)?;
        wire::request_body(invoice, "", "", serde_json::Map::new())?;
        let account = self.provider_account(&tenant).await?;
        // Hoá đơn thay thế / điều chỉnh dùng mẫu và ký hiệu của hoá đơn gốc: không cần hỏi.
        let series = if invoice.related.is_none() {
            self.series(&tenant, &account, template).await?
        } else {
            String::new()
        };
        let body = wire::request_body(invoice, &account, &series, extra)?;
        let reference = wire::reference_code(invoice);
        let reply = self.client.call(&tenant, Call::Post(path, &body), "issue").await?;
        // Mã tham chiếu đã có hoá đơn (lần gửi trước đã tới nơi, câu trả lời bị mất): không gửi
        // thêm; trạng thái hỏi theo mã tham chiếu.
        if reply.status == 409 && error_code(&reply).as_deref() == Some("EINVOICE_DOCUMENT_EXISTED")
        {
            return Ok(IssuedEInvoice::new(wire::provider_ref(&reference, "")));
        }
        if !reply.is_success() {
            return Err(failure(&reply));
        }
        let tracking = reply
            .json()
            .and_then(|json| {
                json.pointer("/data/tracking_code").and_then(Value::as_str).map(str::to_owned)
            })
            .ok_or_else(provider_unavailable)?;
        Ok(IssuedEInvoice::new(wire::provider_ref(&reference, &tracking)))
    }

    async fn status(&self, tenant: TenantId, provider_ref: &str) -> Result<EInvoiceStatus, Error> {
        let tenant = self.client.accounts.tenant(tenant).await?;
        let (reference, tracking) = wire::split_provider_ref(provider_ref);
        if let Some(tracking) = tracking {
            let path = format!("/v1/invoices/create/check/{tracking}");
            let check = self.data(&tenant, Call::Get(&path), "check_invoice").await?;
            match check.get("status").and_then(Value::as_str) {
                Some("Success") => {}
                Some("Failed") => {
                    // Câu trả lời bất đồng bộ không có `error.code`: lý do nằm ở `message`.
                    let reason = check
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("SePay eInvoice could not issue the invoice")
                        .chars()
                        .take(300)
                        .collect();
                    return Ok(EInvoiceStatus::Rejected { reason });
                }
                // `Pending` (SePay tự thử lại theo lịch của nó): không gửi lại, hỏi lại sau.
                _ => return Ok(EInvoiceStatus::Pending),
            }
        }
        self.status_by_reference(&tenant, reference).await
    }

    async fn cancel(&self, _tenant: TenantId, _request: &EInvoiceCancel) -> Result<(), Error> {
        Err(not_supported(PROVIDER))
    }

    async fn download(
        &self,
        tenant: TenantId,
        provider_ref: &str,
        format: EInvoiceFormat,
    ) -> Result<bytes::Bytes, Error> {
        let tenant = self.client.accounts.tenant(tenant).await?;
        let (_, tracking) = wire::split_provider_ref(provider_ref);
        let tracking = tracking.ok_or_else(|| {
            rejected("no tracking code is known for this invoice at SePay eInvoice")
        })?;
        let kind = match format {
            EInvoiceFormat::Pdf => "pdf",
            EInvoiceFormat::Xml => "xml",
        };
        let path = format!("/v1/invoices/{tracking}/download?type={kind}");
        let file = self.data(&tenant, Call::Get(&path), "download_invoice").await?;
        // Tài liệu: nội dung file ở `data.content`, mã hoá base64.
        let content =
            file.get("content").and_then(Value::as_str).ok_or_else(provider_unavailable)?;
        STANDARD.decode(content.trim()).map(bytes::Bytes::from).map_err(|_| provider_unavailable())
    }
}

#[autoxide_kernel::async_trait]
impl ConnectionProbe for SepayEInvoice {
    fn connection(&self) -> &'static str {
        crate::CONNECTION
    }

    /// Lấy token rồi đọc danh sách tài khoản nhà cung cấp.
    async fn test(&self, tenant: &TenantContext) -> Result<(), Error> {
        self.data(tenant, Call::Get("/v1/provider-accounts"), "test").await.map(|_| ())
    }
}
