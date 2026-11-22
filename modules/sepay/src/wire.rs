//! Dạng dữ liệu của SePay, đúng như tài liệu công khai (xem đầu `lib.rs`), và cách đổi sang
//! `BankTransaction` của kernel. Không có I/O ở đây: test được bằng payload mẫu của tài liệu.

use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::payment::{BankDirection, BankTransaction};
use autoxide_kernel::time::Timestamp;
use serde::Deserialize;

/// Thân webhook (JSON). Tài liệu: `id` số nguyên, không đổi qua mọi lần gửi lại; `code` có thể
/// `null`; `subAccount`, `description`, `referenceCode` có thể rỗng; `transferAmount` là số nguyên
/// đồng, luôn dương; `transactionDate` dạng `YYYY-MM-DD HH:mm:ss`, giờ Việt Nam.
#[derive(Debug, Deserialize)]
pub(crate) struct WebhookBody {
    pub id: i64,
    #[serde(rename = "transactionDate")]
    pub transaction_date: String,
    #[serde(rename = "accountNumber")]
    pub account_number: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(rename = "transferType")]
    pub transfer_type: String,
    #[serde(rename = "transferAmount")]
    pub transfer_amount: i64,
    #[serde(rename = "referenceCode", default)]
    pub reference_code: Option<String>,
}

/// Một giao dịch của `GET /transactions` (SePay API v2). `id` là UUID — KHÁC `id` số nguyên của
/// webhook; `reference_number` là mã tham chiếu của ngân hàng, chung cho cả hai nguồn.
#[derive(Debug, Deserialize)]
pub(crate) struct ApiTransaction {
    pub id: String,
    pub transaction_date: String,
    pub account_number: String,
    pub transfer_type: String,
    #[serde(default)]
    pub amount_in: i64,
    #[serde(default)]
    pub amount_out: i64,
    #[serde(default)]
    pub transaction_content: Option<String>,
    #[serde(default)]
    pub reference_number: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApiPage {
    #[serde(default)]
    pub data: Vec<ApiTransaction>,
    #[serde(default)]
    pub meta: ApiMeta,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ApiMeta {
    #[serde(default)]
    pub pagination: ApiPagination,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ApiPagination {
    #[serde(default)]
    pub has_more: bool,
}

/// Một tài khoản của `GET /bank-accounts`.
#[derive(Debug, Deserialize)]
pub(crate) struct ApiBankAccount {
    pub id: String,
    pub account_number: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApiBankAccounts {
    #[serde(default)]
    pub data: Vec<ApiBankAccount>,
}

/// Giờ Việt Nam `YYYY-MM-DD HH:mm:ss` (UTC+7 quanh năm, không đổi giờ) → thời điểm UTC.
pub(crate) fn vietnam_time(raw: &str) -> Option<Timestamp> {
    let (date, time) = raw.trim().split_once(' ')?;
    Timestamp::parse_rfc3339(&format!("{date}T{time}+07:00")).ok()
}

/// `in` → tiền vào, `out` → tiền ra.
fn direction(transfer_type: &str) -> Option<BankDirection> {
    match transfer_type {
        "in" => Some(BankDirection::Credit),
        "out" => Some(BankDirection::Debit),
        _ => None,
    }
}

/// Mã chống trùng của một biến động, GIỐNG NHAU dù nó tới qua webhook hay qua API: hai nguồn mang
/// `id` khác kiểu (số nguyên / UUID), nên khi ngân hàng có mã tham chiếu thì ghép từ tài khoản,
/// ngày, mã tham chiếu và số tiền; không có mã tham chiếu thì dùng `id` của nguồn (`fallback`).
/// Cột của `ledger` dài 100 ký tự: mã dài hơn thì giữ phần đầu và nối bản băm của cả chuỗi.
fn transaction_key(
    account: &str,
    date: &str,
    reference: Option<&str>,
    amount: i64,
    fallback: String,
) -> String {
    let Some(reference) = reference.map(str::trim).filter(|reference| !reference.is_empty()) else {
        return fallback;
    };
    let day = date.trim().split(' ').next().unwrap_or_default();
    let key = format!("ref:{account}:{day}:{reference}:{amount}");
    if key.len() <= 100 {
        return key;
    }
    let digest = autoxide_kernel::secret::sha256(&key);
    let head: String = key.chars().take(100 - digest.len() - 1).collect();
    format!("{head}~{digest}")
}

impl WebhookBody {
    /// `None` khi thân có giá trị ngoài tài liệu (loại giao dịch lạ, số tiền không dương, ngày sai).
    pub(crate) fn into_transaction(self) -> Option<BankTransaction> {
        let direction = direction(&self.transfer_type)?;
        let booked_at = vietnam_time(&self.transaction_date)?;
        if self.transfer_amount <= 0 {
            return None;
        }
        let key = transaction_key(
            &self.account_number,
            &self.transaction_date,
            self.reference_code.as_deref(),
            self.transfer_amount,
            format!("id:{}", self.id),
        );
        Some(BankTransaction::new(
            key,
            self.account_number,
            direction,
            Money::new(self.transfer_amount, Currency::VND),
            self.content.unwrap_or_default(),
            booked_at,
        ))
    }
}

impl ApiTransaction {
    pub(crate) fn into_transaction(self) -> Option<BankTransaction> {
        let direction = direction(&self.transfer_type)?;
        let booked_at = vietnam_time(&self.transaction_date)?;
        let amount = match direction {
            BankDirection::Credit => self.amount_in,
            BankDirection::Debit => self.amount_out,
        };
        if amount <= 0 {
            return None;
        }
        let key = transaction_key(
            &self.account_number,
            &self.transaction_date,
            self.reference_number.as_deref(),
            amount,
            format!("tx:{}", self.id),
        );
        Some(BankTransaction::new(
            key,
            self.account_number,
            direction,
            Money::new(amount, Currency::VND),
            self.transaction_content.unwrap_or_default(),
            booked_at,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Payload mẫu của trang "Tích hợp webhook".
    pub(crate) const SAMPLE: &str = r#"{
  "id": 92704,
  "gateway": "Vietcombank",
  "transactionDate": "2024-07-02 11:08:33",
  "accountNumber": "1017588888",
  "subAccount": "",
  "code": "SEVN63DC8E5C",
  "content": "SEVN63DC8E5C chuyen tien",
  "transferType": "in",
  "description": "NGUYEN VAN A chuyen tien",
  "transferAmount": 5000000,
  "accumulated": 105000000,
  "referenceCode": "FT24012345678"
}"#;

    #[test]
    fn doc_payload_mau_cua_tai_lieu() {
        let body: WebhookBody = serde_json::from_str(SAMPLE).unwrap();
        let txn = body.into_transaction().unwrap();
        assert_eq!(txn.account_number, "1017588888");
        assert_eq!(txn.direction, BankDirection::Credit);
        assert_eq!(txn.amount, Money::new(5_000_000, Currency::VND));
        assert_eq!(txn.memo, "SEVN63DC8E5C chuyen tien");
        // 11:08:33 giờ Việt Nam = 04:08:33 UTC.
        assert_eq!(txn.booked_at, Timestamp::parse_rfc3339("2024-07-02T04:08:33Z").unwrap());
        assert_eq!(txn.provider_txn_id, "ref:1017588888:2024-07-02:FT24012345678:5000000");
    }

    #[test]
    fn webhook_va_api_ra_cung_ma_chong_trung() {
        let api: ApiTransaction = serde_json::from_str(
            r#"{"id":"a1b2c3d4-e5f6-7890-abcd-ef1234567890","transaction_date":"2024-07-02 11:08:33",
                "account_number":"1017588888","va":null,"transfer_type":"in","amount_in":5000000,
                "amount_out":0,"accumulated":105000000,
                "transaction_content":"SEVN63DC8E5C chuyen tien","reference_number":"FT24012345678",
                "code":"SEVN63DC8E5C","bank_brand_name":"Vietcombank","webhook_success":1}"#,
        )
        .unwrap();
        let pushed: WebhookBody = serde_json::from_str(SAMPLE).unwrap();
        assert_eq!(
            api.into_transaction().unwrap().provider_txn_id,
            pushed.into_transaction().unwrap().provider_txn_id
        );
    }

    #[test]
    fn khong_co_ma_tham_chieu_thi_dung_id_cua_nguon() {
        let body: WebhookBody = serde_json::from_str(
            r#"{"id":7,"transactionDate":"2026-01-05 00:10:00","accountNumber":"1","content":null,
                "transferType":"out","transferAmount":1000,"referenceCode":""}"#,
        )
        .unwrap();
        let txn = body.into_transaction().unwrap();
        assert_eq!((txn.provider_txn_id.as_str(), txn.direction), ("id:7", BankDirection::Debit));
        // 00:10 ngày 5 giờ Việt Nam là 17:10 ngày 4 UTC.
        assert_eq!(txn.booked_at, Timestamp::parse_rfc3339("2026-01-04T17:10:00Z").unwrap());
    }

    #[test]
    fn gia_tri_ngoai_tai_lieu_bi_bo() {
        for bad in [
            r#"{"id":1,"transactionDate":"2026-01-05 00:10:00","accountNumber":"1","transferType":"hold","transferAmount":1}"#,
            r#"{"id":1,"transactionDate":"2026-01-05 00:10:00","accountNumber":"1","transferType":"in","transferAmount":0}"#,
            r#"{"id":1,"transactionDate":"05/01/2026","accountNumber":"1","transferType":"in","transferAmount":5}"#,
        ] {
            let body: WebhookBody = serde_json::from_str(bad).unwrap();
            assert!(body.into_transaction().is_none(), "{bad}");
        }
    }

    #[test]
    fn ma_qua_dai_van_vua_cot_va_khac_nhau() {
        let long = "R".repeat(120);
        let a = transaction_key("1", "2026-01-05 00:00:00", Some(&long), 1, String::new());
        let b = transaction_key("1", "2026-01-05 00:00:00", Some(&long), 2, String::new());
        assert!(a.len() <= 100 && b.len() <= 100);
        assert_ne!(a, b);
    }
}
