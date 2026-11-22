//! Lời gọi tới SePay eInvoice API: lấy và giữ access token của từng khách, gửi request kèm token,
//! đổi lỗi của SePay thành lỗi của hợp đồng `einvoice`. Không bao giờ ghi client secret hay token
//! vào log.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use autoxide_kernel::connections::ConnectionStatus;
use autoxide_kernel::einvoice::{provider_unavailable, rejected};
use autoxide_kernel::error::Error;
use autoxide_kernel::provider::{
    Environment, ProviderAccounts, ProviderReply, Target, http_client, send,
};
use autoxide_kernel::tenancy::{TenantContext, TenantId};
use serde_json::Value;

use crate::api::PROVIDER;
use crate::config::EInvoiceConfig;
use crate::settings::{CLIENT_ID, CLIENT_SECRET, ENVIRONMENT};

const TIMEOUT: Duration = Duration::from_secs(20);
/// Bỏ token sớm chừng này trước hạn (tài liệu: token sống 86400 giây).
const TOKEN_MARGIN: Duration = Duration::from_secs(300);

struct CachedToken {
    /// Bản băm của (client id, client secret) đã dùng để lấy token: khách đổi tài khoản thì token
    /// cũ không được dùng nữa.
    fingerprint: String,
    token: String,
    expires_at: Instant,
}

pub(crate) struct Client {
    pub accounts: ProviderAccounts,
    status: ConnectionStatus,
    config: EInvoiceConfig,
    http: reqwest::Client,
    /// Token theo khách: mỗi khách một tài khoản SePay, không bao giờ dùng chung.
    tokens: Mutex<HashMap<TenantId, CachedToken>>,
}

/// Một request tới API (sau `/v1`…).
pub(crate) enum Call<'a> {
    Get(&'a str),
    Post(&'a str, &'a Value),
}

impl Client {
    pub(crate) fn new(
        accounts: ProviderAccounts,
        status: ConnectionStatus,
        config: EInvoiceConfig,
    ) -> Result<Self, Error> {
        Ok(Client {
            accounts,
            status,
            config,
            http: http_client(TIMEOUT, Target::Fixed)?,
            tokens: Mutex::default(),
        })
    }

    /// Địa chỉ API cho khách: một trong HAI địa chỉ cố định của người vận hành, theo cấu hình
    /// `environment` của khách. Giá trị đã lưu mà không còn hợp lệ → từ chối, không đoán.
    async fn base_url(&self, tenant: &TenantContext) -> Result<&str, Error> {
        let chosen = self.accounts.get(tenant, &ENVIRONMENT).await?;
        let environment = Environment::parse(chosen.as_ref().map(|value| value.as_str()))
            .ok_or_else(|| autoxide_kernel::provider::not_configured(ENVIRONMENT.name()))?;
        Ok(environment.pick(&self.config.api_url, &self.config.sandbox_api_url))
    }

    fn forget(&self, tenant: TenantId) {
        self.tokens.lock().unwrap_or_else(PoisonError::into_inner).remove(&tenant);
    }

    /// Access token của khách: `POST /v1/token` với Basic Authentication (client id / client
    /// secret), thân rỗng; giữ lại tới gần hạn.
    async fn token(&self, tenant: &TenantContext) -> Result<String, Error> {
        let id = self.accounts.require(tenant, &CLIENT_ID).await?;
        let secret = self.accounts.require(tenant, &CLIENT_SECRET).await?;
        let base = self.base_url(tenant).await?;
        // Môi trường nằm trong dấu vân tay: đổi môi trường thì token cũ không được dùng nữa.
        let fingerprint = autoxide_kernel::secret::sha256(&format!(
            "{base}\n{}\n{}",
            id.as_str(),
            secret.as_str()
        ));
        {
            let tokens = self.tokens.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(cached) = tokens.get(&tenant.id())
                && cached.fingerprint == fingerprint
                && cached.expires_at > Instant::now()
            {
                return Ok(cached.token.clone());
            }
        }
        let request = self
            .http
            .post(format!("{base}/v1/token"))
            .basic_auth(id.as_str(), Some(secret.as_str()));
        let reply = send(request, PROVIDER, "token").await.map_err(|_| provider_unavailable())?;
        if reply.status == 400 || reply.status == 401 {
            return Err(rejected("SePay eInvoice refused the client id or client secret"));
        }
        let json = reply.json().filter(|_| reply.is_success()).ok_or_else(provider_unavailable)?;
        let token = json
            .pointer("/data/access_token")
            .and_then(Value::as_str)
            .ok_or_else(provider_unavailable)?
            .to_owned();
        let lifetime = json.pointer("/data/expires_in").and_then(Value::as_u64).unwrap_or(3600);
        let expires_at =
            Instant::now() + Duration::from_secs(lifetime).saturating_sub(TOKEN_MARGIN);
        self.tokens
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(tenant.id(), CachedToken { fingerprint, token: token.clone(), expires_at });
        Ok(token)
    }

    async fn send_once(
        &self,
        tenant: &TenantContext,
        call: &Call<'_>,
        purpose: &'static str,
    ) -> Result<ProviderReply, Error> {
        let token = self.token(tenant).await?;
        let base = self.base_url(tenant).await?;
        let request = match call {
            Call::Get(path) => self.http.get(format!("{base}{path}")),
            Call::Post(path, body) => self
                .http
                .post(format!("{base}{path}"))
                .header("content-type", "application/json")
                .body(serde_json::to_vec(body).map_err(Error::internal)?),
        };
        send(request.bearer_auth(token), PROVIDER, purpose)
            .await
            .map_err(|_| provider_unavailable())
    }

    /// Gửi `call` với token của khách. 401 (token hết hạn / bị thu hồi): lấy token mới và gửi lại
    /// một lần. Trả câu trả lời thô — bên gọi đọc mã HTTP và `error.code` theo tài liệu.
    pub(crate) async fn call(
        &self,
        tenant: &TenantContext,
        call: Call<'_>,
        purpose: &'static str,
    ) -> Result<ProviderReply, Error> {
        let mut outcome = self.send_once(tenant, &call, purpose).await;
        if matches!(&outcome, Ok(reply) if reply.status == 401) {
            self.forget(tenant.id());
            outcome = self.send_once(tenant, &call, purpose).await;
        }
        let problem = match &outcome {
            Ok(reply) if reply.status == 401 => {
                Some("SePay eInvoice refused the access token".to_owned())
            }
            Ok(_) => None,
            Err(error) => Some(error.message()),
        };
        self.status.record(tenant, crate::CONNECTION, problem.as_deref()).await;
        outcome
    }
}

/// `error.code` của một câu trả lời lỗi (`{"success": false, "error": {"code": …}}`).
pub(crate) fn error_code(reply: &ProviderReply) -> Option<String> {
    reply.json()?.pointer("/error/code")?.as_str().map(str::to_owned)
}

/// Đổi một câu trả lời KHÔNG thành công thành lỗi của hợp đồng. Tài liệu: xử lý theo `error.code`,
/// không theo `message`; `500` kèm mã nhận diện được là lỗi trạng thái tài khoản (hết hạn mức,
/// chưa đăng ký xong…) — gọi lại không đổi kết quả; `500` không kèm mã là lỗi hệ thống.
pub(crate) fn failure(reply: &ProviderReply) -> Error {
    match (reply.status, error_code(reply)) {
        (401, _) => rejected("SePay eInvoice refused the access token"),
        (429, _) => provider_unavailable(),
        (status, None) if status >= 500 => provider_unavailable(),
        (_, Some(code)) => rejected(format!("SePay eInvoice answered {code}")),
        (status, None) => rejected(format!("SePay eInvoice answered HTTP {status}")),
    }
}
