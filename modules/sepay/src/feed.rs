//! `BankFeed` của SePay: nhận webhook biến động số dư (kiểu đẩy) và đọc lại giao dịch bằng SePay
//! API v2 (kiểu kéo, để đối soát khi webhook thất lạc). Mọi thứ thuộc về khách — secret webhook,
//! API token — đọc từ cấu hình của đúng khách đó qua [`ProviderAccounts`].

use std::time::Duration;

use autoxide_kernel::connections::{ConnectionProbe, ConnectionStatus};
use autoxide_kernel::error::Error;
use autoxide_kernel::payment::{
    BankAccountRef, BankBatch, BankFeed, BankTransaction, provider_rejected, provider_unavailable,
    signature_invalid,
};
use autoxide_kernel::provider::{
    ProviderAccounts, ProviderReply, Target, WebhookAck, constant_time_eq, hex, hmac_sha256,
    http_client, not_configured, send,
};
use autoxide_kernel::tenancy::{TenantContext, TenantId};
use autoxide_kernel::time::Timestamp;
use axum::http::HeaderMap;

use crate::api::PROVIDER;
use crate::config::SepayConfig;
use crate::settings::{API_TOKEN, WEBHOOK_API_KEY, WEBHOOK_SECRET};
use crate::wire::{ApiBankAccounts, ApiPage, WebhookBody};

/// Header chữ ký HMAC: `sha256={hex}` của `"{timestamp}.{thân thô}"`.
const SIGNATURE: &str = "x-sepay-signature";
/// Header thời điểm ký (giây Unix).
const TIMESTAMP: &str = "x-sepay-timestamp";
/// Tài liệu: từ chối request có timestamp cách hiện tại quá 5 phút (chống phát lại).
const MAX_SKEW_SECONDS: i64 = 300;
/// Số giao dịch mỗi trang khi kéo (tài liệu: tối đa 100).
const PER_PAGE: u32 = 100;
/// Lần kéo đầu của một tài khoản (chưa có con trỏ) đọc lùi chừng này ngày: thanh toán đang chờ
/// không sống lâu hơn thế, và không kéo cả lịch sử tài khoản.
const FIRST_PULL_DAYS: i64 = 3;
const TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) struct SepayFeed {
    accounts: ProviderAccounts,
    status: ConnectionStatus,
    config: SepayConfig,
    client: reqwest::Client,
}

/// Con trỏ kéo của một tài khoản, lưu ở `ledger`. Tài liệu của `since_id`: nó chỉ lọc, không đổi
/// thứ tự — phải đọc HẾT các trang của một `since_id` (sắp `transaction_date` tăng dần) rồi mới
/// lấy `id` của giao dịch cuối làm `since_id` kế. Nên con trỏ mang cả mốc lẫn số trang.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Cursor {
    /// Chưa có giao dịch nào làm mốc: đọc từ thời điểm này (`YYYY-MM-DD HH:mm:ss`, giờ Việt Nam).
    From { from: String, page: u32 },
    /// Đọc các giao dịch ghi nhận sau giao dịch `since` (UUID của SePay).
    Since { since: String, page: u32 },
}

impl Cursor {
    fn parse(raw: &str) -> Option<Self> {
        let (kind, rest) = raw.split_once('|')?;
        let (mark, page) = rest.rsplit_once('|')?;
        let page = page.parse().ok().filter(|page| *page >= 1)?;
        match kind {
            "from" => Some(Cursor::From { from: mark.to_owned(), page }),
            "since" => Some(Cursor::Since { since: mark.to_owned(), page }),
            _ => None,
        }
    }

    fn encode(&self) -> String {
        match self {
            Cursor::From { from, page } => format!("from|{from}|{page}"),
            Cursor::Since { since, page } => format!("since|{since}|{page}"),
        }
    }

    fn page(&self) -> u32 {
        match self {
            Cursor::From { page, .. } | Cursor::Since { page, .. } => *page,
        }
    }

    fn next_page(&self) -> Self {
        match self {
            Cursor::From { from, page } => Cursor::From { from: from.clone(), page: page + 1 },
            Cursor::Since { since, page } => Cursor::Since { since: since.clone(), page: page + 1 },
        }
    }

    /// Mốc đọc đầu tiên: `FIRST_PULL_DAYS` ngày trước `now`, viết theo giờ Việt Nam (UTC+7).
    fn first(now: Timestamp) -> Self {
        let shifted = now
            .checked_sub(jiff::SignedDuration::from_hours(FIRST_PULL_DAYS * 24 - 7))
            .unwrap_or(now);
        Cursor::From { from: shifted.format("%Y-%m-%d %H:%M:%S"), page: 1 }
    }
}

impl SepayFeed {
    pub(crate) fn new(
        accounts: ProviderAccounts,
        status: ConnectionStatus,
        config: SepayConfig,
    ) -> Result<Self, Error> {
        // Địa chỉ SePay do người vận hành đặt, không phải khách chọn: `Target::Fixed`.
        Ok(SepayFeed { accounts, status, config, client: http_client(TIMEOUT, Target::Fixed)? })
    }

    /// Kiểm xác thực của webhook bằng secret của khách `tenant`. Khách đặt Secret Key thì BẮT BUỘC
    /// chữ ký HMAC; chỉ đặt API Key thì kiểm header `Authorization: Apikey …`; chưa đặt gì thì từ
    /// chối — không bao giờ nhận webhook không xác thực.
    async fn authenticate(
        &self,
        tenant: &TenantContext,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<(), Error> {
        let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
        if let Some(secret) = self.accounts.get(tenant, &WEBHOOK_SECRET).await? {
            let (Some(signature), Some(timestamp)) = (header(SIGNATURE), header(TIMESTAMP)) else {
                return Err(signature_invalid());
            };
            let signed_at: i64 = timestamp.trim().parse().map_err(|_| signature_invalid())?;
            if (Timestamp::now().as_unix_seconds() - signed_at).abs() > MAX_SKEW_SECONDS {
                return Err(signature_invalid());
            }
            // Ký trên byte gốc của thân: `"{timestamp}.{raw_body}"`.
            let mut signed = Vec::with_capacity(timestamp.len() + 1 + body.len());
            signed.extend_from_slice(timestamp.as_bytes());
            signed.push(b'.');
            signed.extend_from_slice(body);
            let expected =
                format!("sha256={}", hex(&hmac_sha256(secret.as_str().as_bytes(), &signed)));
            return if constant_time_eq(expected.as_bytes(), signature.trim().as_bytes()) {
                Ok(())
            } else {
                Err(signature_invalid())
            };
        }
        if let Some(key) = self.accounts.get(tenant, &WEBHOOK_API_KEY).await? {
            let given = header("authorization").and_then(|value| value.strip_prefix("Apikey "));
            return match given {
                Some(given) if constant_time_eq(key.as_str().as_bytes(), given.as_bytes()) => {
                    Ok(())
                }
                _ => Err(signature_invalid()),
            };
        }
        Err(not_configured(WEBHOOK_SECRET.name()))
    }

    /// `GET {api}{path}` với API token của khách; đổi lỗi của SePay thành lỗi của hợp đồng.
    async fn get(
        &self,
        tenant: &TenantContext,
        path: &str,
        query: &[(&str, String)],
        purpose: &'static str,
    ) -> Result<ProviderReply, Error> {
        let token = self.accounts.require(tenant, &API_TOKEN).await?;
        // Ghép tham số bằng `Url` (mã hoá phần trăm đúng chuẩn); `reqwest` ở đây không bật `query`.
        let mut url = reqwest::Url::parse(&format!("{}{path}", self.config.api_url))
            .map_err(|error| Error::internal(format!("SePay API address: {error}")))?;
        url.query_pairs_mut()
            .extend_pairs(query.iter().map(|(name, value)| (*name, value.as_str())));
        let request = self.client.get(url).bearer_auth(token.as_str());
        let outcome = send(request, PROVIDER, purpose).await;
        let result = match outcome {
            Err(_) => Err(provider_unavailable()),
            Ok(reply) if reply.is_success() => Ok(reply),
            // Tài liệu: 401 `unauthorized` (thiếu / sai / bị vô hiệu), 429 `rate_limited`.
            Ok(reply) if reply.status == 401 => {
                Err(provider_rejected("SePay refused the API token"))
            }
            Ok(reply) if reply.status == 429 || reply.status >= 500 => Err(provider_unavailable()),
            Ok(reply) => Err(provider_rejected(format!("SePay answered HTTP {}", reply.status))),
        };
        let problem = result.as_ref().err().map(Error::message);
        self.status.record(tenant, crate::CONNECTION, problem.as_deref()).await;
        result
    }

    /// UUID của tài khoản ngân hàng `account_number` ở SePay (để lọc giao dịch theo tài khoản).
    async fn bank_account_id(
        &self,
        tenant: &TenantContext,
        account_number: &str,
    ) -> Result<String, Error> {
        let query = [("q", account_number.to_owned()), ("per_page", PER_PAGE.to_string())];
        let reply = self.get(tenant, "/bank-accounts", &query, "find_bank_account").await?;
        let listed: ApiBankAccounts =
            serde_json::from_slice(&reply.body).map_err(|_| provider_unavailable())?;
        listed
            .data
            .into_iter()
            .find(|found| found.account_number == account_number)
            .map(|found| found.id)
            .ok_or_else(|| {
                provider_rejected("this bank account is not linked to the SePay company")
            })
    }
}

#[async_trait::async_trait]
impl BankFeed for SepayFeed {
    fn provider(&self) -> &'static str {
        PROVIDER
    }

    async fn verify_webhook(
        &self,
        tenant: TenantId,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<Vec<BankTransaction>, Error> {
        let tenant = self.accounts.tenant(tenant).await?;
        self.authenticate(&tenant, headers, body).await?;
        // Chỉ đọc thân sau khi đã xác thực. Thân không đúng dạng JSON của tài liệu (vd webhook đặt
        // định dạng form) bị từ chối rõ ràng để khách sửa cấu hình ở SePay.
        let parsed: WebhookBody = serde_json::from_slice(body).map_err(|_| {
            provider_rejected("the notification is not the JSON body SePay documents; set the webhook's data format to JSON")
        })?;
        match parsed.into_transaction() {
            Some(transaction) => Ok(vec![transaction]),
            None => {
                Err(provider_rejected("the notification has values outside SePay's documentation"))
            }
        }
    }

    fn webhook_ack(&self) -> Option<WebhookAck> {
        // Tài liệu: chỉ HTTP 200 / 201 kèm đúng `{"success": true}` mới được tính là đã nhận.
        Some(WebhookAck::json(200, r#"{"success": true}"#))
    }

    async fn fetch(
        &self,
        tenant: TenantId,
        account: &BankAccountRef,
        cursor: Option<String>,
    ) -> Result<BankBatch, Error> {
        let tenant = self.accounts.tenant(tenant).await?;
        // `ledger` kéo theo lịch mọi tài khoản có nguồn. Khách chỉ dùng webhook (chưa đặt API token)
        // thì không có gì để kéo: trả trang rỗng, giữ nguyên con trỏ — không phải lỗi, và không bao
        // giờ dùng token của ai khác.
        if self.accounts.get(&tenant, &API_TOKEN).await?.is_none() {
            return Ok(BankBatch::new(Vec::new(), cursor));
        }
        let cursor = cursor
            .as_deref()
            .and_then(Cursor::parse)
            .unwrap_or_else(|| Cursor::first(Timestamp::now()));
        let bank_account = self.bank_account_id(&tenant, &account.account_number).await?;
        let mut query = vec![
            ("bank_account_id", bank_account),
            ("transaction_date_sort", "asc".to_owned()),
            ("per_page", PER_PAGE.to_string()),
            ("page", cursor.page().to_string()),
        ];
        match &cursor {
            Cursor::From { from, .. } => query.push(("transaction_date_from", from.clone())),
            Cursor::Since { since, .. } => query.push(("since_id", since.clone())),
        }
        let reply = self.get(&tenant, "/transactions", &query, "list_transactions").await?;
        let page: ApiPage =
            serde_json::from_slice(&reply.body).map_err(|_| provider_unavailable())?;
        let last_id = page.data.last().map(|found| found.id.clone());
        let transactions: Vec<BankTransaction> =
            page.data.into_iter().filter_map(|found| found.into_transaction()).collect();
        if page.meta.pagination.has_more {
            return Ok(BankBatch::new(transactions, Some(cursor.next_page().encode())).more());
        }
        // Hết trang của mốc này: mốc kế là giao dịch cuối vừa đọc; trang rỗng thì giữ mốc cũ.
        let next = match last_id {
            Some(since) => Cursor::Since { since, page: 1 },
            None => match cursor {
                Cursor::From { from, .. } => Cursor::From { from, page: 1 },
                Cursor::Since { since, .. } => Cursor::Since { since, page: 1 },
            },
        };
        Ok(BankBatch::new(transactions, Some(next.encode())))
    }
}

#[async_trait::async_trait]
impl ConnectionProbe for SepayFeed {
    fn connection(&self) -> &'static str {
        crate::CONNECTION
    }

    /// Một lời gọi rẻ có xác thực: danh sách tài khoản ngân hàng, một dòng.
    async fn test(&self, tenant: &TenantContext) -> Result<(), Error> {
        self.get(tenant, "/bank-accounts", &[("per_page", "1".to_owned())], "test")
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn con_tro_ma_hoa_roi_doc_lai_duoc() {
        for cursor in [
            Cursor::From { from: "2026-11-18 09:00:00".to_owned(), page: 3 },
            Cursor::Since { since: "a1b2c3d4-e5f6-7890-abcd-ef1234567890".to_owned(), page: 1 },
        ] {
            assert_eq!(Cursor::parse(&cursor.encode()), Some(cursor));
        }
        for bad in ["", "since|x", "other|x|1", "since|x|0", "since|x|abc"] {
            assert_eq!(Cursor::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn moc_dau_tinh_theo_gio_viet_nam() {
        let now = Timestamp::parse_rfc3339("2026-11-21T20:00:00Z").unwrap();
        // 20:00 UTC ngày 21 = 03:00 ngày 22 giờ Việt Nam; lùi 3 ngày.
        assert_eq!(
            Cursor::first(now),
            Cursor::From { from: "2026-11-19 03:00:00".to_owned(), page: 1 }
        );
    }
}
