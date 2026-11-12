//! Đề proof-of-work theo giao thức ALTCHA v2 (MIT), tự viết bằng `hmac` / `sha2` đang có — không
//! dùng crate `altcha` vì nó kéo theo bản cũ của hai thư viện đó (D-120). Mặc định tắt
//! (`AUTOXIDE_ANTISPAM_CHALLENGE=altcha` để bật).
//!
//! Cố định phần giao thức (widget, bộ giải và bộ kiểm phải khớp nhau):
//!
//! - thuật toán `SHA-256` lặp: khoá = SHA-256 lặp `cost` lần trên `salt ‖ nonce ‖ bộ đếm`;
//! - bộ đếm 4 byte big-endian (`CounterMode::Uint32`, mặc định của v2);
//! - "deterministic mode": máy chủ chọn bộ đếm ngẫu nhiên trong [32, 64], `keyPrefix` là nửa đầu
//!   khoá của bộ đếm đó; trình duyệt dò bộ đếm từ 0 tới khi khớp;
//! - chữ ký: HMAC-SHA256 (hex) trên JSON của `parameters` đã sắp khoá ở mọi tầng;
//! - `data` mang phạm vi form và `binding` = nonce của token form, nên một lời giải chỉ dùng được
//!   cho đúng một token form — mà token form chỉ được nhận một lần (bảng nonce của module).
//!
//! Lời giải trình duyệt gửi là base64 (chuẩn) của JSON `{"challenge": …, "solution": …}`.

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chrono::Utc;
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{ChallengeOutcome, ChallengeVerifier};
use crate::error::Error;
use crate::secret::SecretKey;

/// Nhãn dẫn khoá ký đề.
const PURPOSE: &str = "autoxide/antispam/altcha/v1";
/// Độ dài khoá (byte) và tên thuật toán của giao thức.
const KEY_LENGTH: usize = 32;
const ALGORITHM: &str = "SHA-256";
/// Đề sống chừng này giây.
const TTL_SECONDS: i64 = 600;
/// Khoảng bộ đếm máy chủ chọn: trình duyệt thử trung bình ~48 lần, mỗi lần `cost` vòng SHA-256.
const COUNTER_MIN: u32 = 32;
const COUNTER_MAX: u32 = 64;

/// Bộ đề ALTCHA v2.
pub struct Altcha {
    secret: SecretKey,
    cost: u32,
}

impl Altcha {
    pub fn new(secret: SecretKey, cost: u32) -> Self {
        Altcha { secret, cost }
    }

    async fn key(&self) -> Result<String, Error> {
        Ok(hex(&self.secret.derive(PURPOSE).await?))
    }
}

#[async_trait::async_trait]
impl ChallengeVerifier for Altcha {
    async fn challenge(&self, scope: &str, binding: &str) -> Result<Option<Value>, Error> {
        let mut random = [0u8; 36];
        getrandom::fill(&mut random)
            .map_err(|error| Error::internal(format!("random number generator: {error}")))?;
        let span = COUNTER_MAX - COUNTER_MIN + 1;
        let counter = COUNTER_MIN
            + u32::from_be_bytes([random[32], random[33], random[34], random[35]]) % span;
        let (nonce, salt) = (random[..16].to_vec(), random[16..32].to_vec());
        // `cost` vòng SHA-256 trên một endpoint công khai: chạy trên thread cho việc chặn, không
        // giữ thread async (D-150).
        let (seed_salt, seed_nonce, cost) = (salt.clone(), nonce.clone(), self.cost);
        let derived =
            tokio::task::spawn_blocking(move || derive(&seed_salt, &seed_nonce, counter, cost))
                .await
                .map_err(Error::internal)?;
        let (nonce, salt) = (nonce.as_slice(), salt.as_slice());
        let parameters = json!({
            "algorithm": ALGORITHM,
            "cost": self.cost,
            "data": {"binding": binding, "scope": scope},
            "expiresAt": Utc::now().timestamp() + TTL_SECONDS,
            "keyLength": KEY_LENGTH,
            "keyPrefix": hex(&derived[..KEY_LENGTH / 2]),
            "nonce": hex(nonce),
            "salt": hex(salt),
        });
        let signature = sign(&parameters, &self.key().await?)?;
        Ok(Some(json!({"parameters": parameters, "signature": signature})))
    }

    async fn verify(
        &self,
        scope: &str,
        binding: &str,
        solution: Option<&str>,
    ) -> Result<ChallengeOutcome, Error> {
        let Some(solution) = solution.filter(|solution| !solution.trim().is_empty()) else {
            return Ok(ChallengeOutcome::Missing);
        };
        let (key, cost, now) = (self.key().await?, self.cost, Utc::now().timestamp());
        let (solution, scope, binding) =
            (solution.to_owned(), scope.to_owned(), binding.to_owned());
        // Kiểm lời giải cũng tính `cost` vòng SHA-256: thread cho việc chặn.
        let passed = tokio::task::spawn_blocking(move || {
            let expected = Expected { scope: &scope, binding: &binding, cost, now };
            check(&solution, &key, &expected)
        })
        .await
        .map_err(Error::internal)?;
        Ok(if passed { ChallengeOutcome::Passed } else { ChallengeOutcome::Failed })
    }
}

/// Điều lời giải phải khớp.
struct Expected<'a> {
    scope: &'a str,
    binding: &'a str,
    cost: u32,
    now: i64,
}

/// Lời giải `payload` (base64 JSON) có đúng không. Mọi chỗ sai dạng đều là "sai".
fn check(payload: &str, key: &str, expected: &Expected<'_>) -> bool {
    let Ok(bytes) = STANDARD.decode(payload.trim()) else {
        return false;
    };
    let Ok(payload) = serde_json::from_slice::<Value>(&bytes) else {
        return false;
    };
    let parameters = &payload["challenge"]["parameters"];
    let (Some(signature), Some(counter), Some(claimed)) = (
        payload["challenge"]["signature"].as_str(),
        payload["solution"]["counter"].as_u64().and_then(|counter| u32::try_from(counter).ok()),
        payload["solution"]["derivedKey"].as_str(),
    ) else {
        return false;
    };
    // 1. Hết hạn (luôn có hạn: máy chủ này luôn đặt).
    if !parameters["expiresAt"].as_i64().is_some_and(|at| at >= expected.now) {
        return false;
    }
    // 2. Chữ ký, so sánh thời gian không đổi.
    let Ok(wanted) = sign(parameters, key) else {
        return false;
    };
    if !constant_time_eq(wanted.as_bytes(), signature.as_bytes()) {
        return false;
    }
    // 3. Đề do chính máy chủ này phát cho đúng form và đúng token form, đúng thông số.
    let fixed = parameters["algorithm"] == ALGORITHM
        && parameters["keyLength"].as_u64() == Some(KEY_LENGTH as u64)
        && parameters["cost"].as_u64() == Some(u64::from(expected.cost))
        && parameters["data"]["scope"] == expected.scope
        && parameters["data"]["binding"] == expected.binding;
    if !fixed {
        return false;
    }
    // 4. Dựng lại khoá từ bộ đếm, so với khoá gửi lên và với `keyPrefix` đã ký.
    let (Some(salt), Some(nonce), Some(prefix)) = (
        parameters["salt"].as_str().and_then(unhex),
        parameters["nonce"].as_str().and_then(unhex),
        parameters["keyPrefix"].as_str().and_then(unhex),
    ) else {
        return false;
    };
    let derived = derive(&salt, &nonce, counter, expected.cost);
    constant_time_eq(hex(&derived).as_bytes(), claimed.to_ascii_lowercase().as_bytes())
        && derived.starts_with(&prefix)
}

/// Khoá của bộ đếm `counter`: SHA-256 lặp `cost` lần trên `salt ‖ nonce ‖ counter`.
fn derive(salt: &[u8], nonce: &[u8], counter: u32, cost: u32) -> Vec<u8> {
    let mut data: Vec<u8> =
        salt.iter().chain(nonce).copied().chain(counter.to_be_bytes()).collect();
    for _ in 0..cost.max(1) {
        data = Sha256::digest(&data).to_vec();
    }
    data.truncate(KEY_LENGTH);
    data
}

/// HMAC-SHA256 (hex) trên JSON của `parameters` đã sắp khoá ở mọi tầng; khoá là chuỗi `key`.
fn sign(parameters: &Value, key: &str) -> Result<String, Error> {
    let canonical = serde_json::to_string(&sorted(parameters.clone())).map_err(Error::internal)?;
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key.as_bytes())
        .map_err(|error| Error::internal(format!("hmac key: {error}")))?;
    mac.update(canonical.as_bytes());
    Ok(hex(&mac.finalize().into_bytes()))
}

fn sorted(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let ordered: BTreeMap<String, Value> =
                map.into_iter().map(|(key, value)| (key, sorted(value))).collect();
            Value::Object(ordered.into_iter().collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len()).step_by(2).map(|at| u8::from_str_radix(&text[at..at + 2], 16).ok()).collect()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sinh MỘT lần bằng thư viện chính thức `altcha` 1.1.0 (`create_challenge` với
    /// `SHA-256`, `cost` 500, `counter` 73, khoá ký `fixture-secret`, rồi `solve_challenge`).
    const FIXTURE: &str = include_str!("fixtures/altcha_v2.json");
    const KEY: &str = "fixture-secret";

    fn expected(now: i64) -> Expected<'static> {
        Expected { scope: "forms:contact", binding: "bm9uY2UtZm9yLWZpeHR1cmU", cost: 500, now }
    }

    fn encode(value: &Value) -> String {
        STANDARD.encode(serde_json::to_vec(value).unwrap())
    }

    #[test]
    fn khop_vector_cua_thu_vien_chinh_thuc() {
        let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
        assert_eq!(
            sign(&fixture["challenge"]["parameters"], KEY).unwrap(),
            fixture["challenge"]["signature"]
        );
        assert!(check(&encode(&fixture), KEY, &expected(1_800_000_000)));
    }

    #[test]
    fn tu_choi_het_han_sai_chu_ky_sai_loi_giai_sai_form() {
        let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
        assert!(!check(&encode(&fixture), KEY, &expected(4_102_444_801)), "expired");
        assert!(!check(&encode(&fixture), "other-secret", &expected(0)), "other key");
        let mut wrong = fixture.clone();
        wrong["solution"]["counter"] = json!(72);
        assert!(!check(&encode(&wrong), KEY, &expected(0)), "wrong counter");
        let mut forged = fixture.clone();
        forged["challenge"]["parameters"]["cost"] = json!(1);
        assert!(!check(&encode(&forged), KEY, &expected(0)), "parameters changed");
        let other_form = Expected { scope: "forms:other", ..expected(0) };
        assert!(!check(&encode(&fixture), KEY, &other_form));
        let other_token = Expected { binding: "another", ..expected(0) };
        assert!(!check(&encode(&fixture), KEY, &other_token));
        assert!(!check("not base64!", KEY, &expected(0)));
        assert!(!check(&STANDARD.encode("{}"), KEY, &expected(0)));
    }

    /// Đề do bộ này phát giải được theo đúng cách widget giải (dò bộ đếm từ 0).
    #[test]
    fn de_tu_phat_giai_duoc() {
        let (salt, nonce, cost) = ([1u8; 16], [2u8; 16], 50);
        let target = derive(&salt, &nonce, 40, cost);
        let prefix = &target[..KEY_LENGTH / 2];
        let found = (0..=COUNTER_MAX)
            .find(|counter| derive(&salt, &nonce, *counter, cost).starts_with(prefix));
        assert_eq!(found, Some(40));
    }
}
