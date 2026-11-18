//! Kiểm `id_token` của nhà cung cấp OIDC (bước 40 P3): JWT ký bất đối xứng, tự viết như token của
//! Autoxide (D-045) — thư viện JWT kéo thêm `rsa` (lỗ hổng RUSTSEC còn mở, D-077) hay cả một bộ
//! OIDC. Chỉ nhận hai thuật toán nhà cung cấp lớn dùng:
//!
//! - `RS256` kiểm bằng `ring` (đã có trong cây qua rustls và lớp mã hoá secret),
//! - `ES256` kiểm bằng `p256` (đã dùng cho passkey).
//!
//! Không bao giờ nhận `none` hay `HS*` (khoá đối xứng là `client_secret` — ai biết nó cũng ký được).
//! Khoá chọn theo `kid` trong JWKS của issuer; claim kiểm theo OpenID Connect Core §3.1.3.7.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde_json::Value;

/// Lệch giờ chấp nhận giữa máy chủ này và nhà cung cấp, giây.
pub(crate) const LEEWAY_SECONDS: i64 = 60;

/// Một khoá trong JWKS (RFC 7517). Chỉ giữ trường cần cho RSA và EC P-256.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Jwk {
    pub kty: String,
    #[serde(default)]
    pub kid: Option<String>,
    #[serde(default, rename = "use")]
    pub usage: Option<String>,
    #[serde(default)]
    pub n: Option<String>,
    #[serde(default)]
    pub e: Option<String>,
    #[serde(default)]
    pub crv: Option<String>,
    #[serde(default)]
    pub x: Option<String>,
    #[serde(default)]
    pub y: Option<String>,
}

/// JWKS của một issuer.
#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct JwkSet {
    #[serde(default)]
    pub keys: Vec<Jwk>,
}

/// Vì sao `id_token` bị từ chối — lý do ổn định cho máy (`reason` của lỗi, `{reason}` của câu).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rejection {
    Malformed,
    Algorithm,
    UnknownKey,
    Signature,
    Issuer,
    Audience,
    Expired,
    NotYetValid,
    Nonce,
}

impl Rejection {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Rejection::Malformed => "malformed",
            Rejection::Algorithm => "algorithm",
            Rejection::UnknownKey => "unknown_key",
            Rejection::Signature => "signature",
            Rejection::Issuer => "issuer",
            Rejection::Audience => "audience",
            Rejection::Expired => "expired",
            Rejection::NotYetValid => "not_yet_valid",
            Rejection::Nonce => "nonce",
        }
    }
}

/// Thuật toán được nhận.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Algorithm {
    Rs256,
    Es256,
}

impl Algorithm {
    fn key_type(self) -> &'static str {
        match self {
            Algorithm::Rs256 => "RSA",
            Algorithm::Es256 => "EC",
        }
    }
}

/// `id_token` đã tách, chưa kiểm chữ ký.
#[derive(Debug)]
pub(crate) struct Unverified<'t> {
    pub algorithm: Algorithm,
    pub kid: Option<String>,
    signing_input: &'t str,
    signature: Vec<u8>,
    pub claims: Value,
}

#[derive(Deserialize)]
struct Header {
    alg: String,
    #[serde(default)]
    kid: Option<String>,
}

/// Tách `token` dạng `header.payload.signature` (JWS Compact Serialization).
pub(crate) fn parse(token: &str) -> Result<Unverified<'_>, Rejection> {
    let mut parts = token.split('.');
    let (Some(header), Some(payload), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(Rejection::Malformed);
    };
    let decode = |part: &str| URL_SAFE_NO_PAD.decode(part).map_err(|_| Rejection::Malformed);
    let header: Header =
        serde_json::from_slice(&decode(header)?).map_err(|_| Rejection::Malformed)?;
    let algorithm = match header.alg.as_str() {
        "RS256" => Algorithm::Rs256,
        "ES256" => Algorithm::Es256,
        // `none`, `HS256`, `PS256`, `RS512`… — không bao giờ nhận.
        _ => return Err(Rejection::Algorithm),
    };
    let claims: Value =
        serde_json::from_slice(&decode(payload)?).map_err(|_| Rejection::Malformed)?;
    if !claims.is_object() {
        return Err(Rejection::Malformed);
    }
    let signing_input = &token[..header_len(token, payload)];
    Ok(Unverified {
        algorithm,
        kid: header.kid,
        signing_input,
        signature: decode(signature)?,
        claims,
    })
}

// Độ dài của `header.payload` trong token.
fn header_len(token: &str, payload: &str) -> usize {
    token.find('.').map_or(0, |dot| dot + 1 + payload.len())
}

impl Unverified<'_> {
    /// Khoá trong `keys` dùng để kiểm: đúng `kid` và đúng loại khoá; token không có `kid` thì
    /// chỉ khi JWKS có đúng một khoá loại đó.
    pub(crate) fn select<'k>(&self, keys: &'k JwkSet) -> Option<&'k Jwk> {
        let kty = self.algorithm.key_type();
        let usable = |key: &&Jwk| key.kty == kty && key.usage.as_deref().is_none_or(|u| u == "sig");
        match &self.kid {
            Some(kid) => keys.keys.iter().filter(usable).find(|key| key.kid.as_ref() == Some(kid)),
            None => {
                let mut candidates = keys.keys.iter().filter(usable);
                match (candidates.next(), candidates.next()) {
                    (Some(only), None) => Some(only),
                    _ => None,
                }
            }
        }
    }

    /// Kiểm chữ ký bằng `key`.
    pub(crate) fn verify(&self, key: &Jwk) -> Result<(), Rejection> {
        verify_signature(self.algorithm, key, self.signing_input.as_bytes(), &self.signature)
    }
}

/// Kiểm chữ ký `signature` của `input` bằng khoá công khai `key`.
pub(crate) fn verify_signature(
    algorithm: Algorithm,
    key: &Jwk,
    input: &[u8],
    signature: &[u8],
) -> Result<(), Rejection> {
    let field = |value: &Option<String>| {
        value
            .as_deref()
            .and_then(|value| URL_SAFE_NO_PAD.decode(value).ok())
            .ok_or(Rejection::UnknownKey)
    };
    match algorithm {
        Algorithm::Rs256 => {
            if key.kty != "RSA" {
                return Err(Rejection::UnknownKey);
            }
            let (n, e) = (field(&key.n)?, field(&key.e)?);
            // Khoá ngắn hơn 2048 bit bị `ring` từ chối (thuật toán `_2048_8192`).
            ring::signature::RsaPublicKeyComponents { n: &n, e: &e }
                .verify(&ring::signature::RSA_PKCS1_2048_8192_SHA256, input, signature)
                .map_err(|_| Rejection::Signature)
        }
        Algorithm::Es256 => {
            use p256::ecdsa::signature::Verifier as _;
            if key.kty != "EC" || key.crv.as_deref() != Some("P-256") {
                return Err(Rejection::UnknownKey);
            }
            let (x, y) = (field(&key.x)?, field(&key.y)?);
            if x.len() != 32 || y.len() != 32 {
                return Err(Rejection::UnknownKey);
            }
            // Dạng SEC1 không nén: `04 || x || y`.
            let mut point = Vec::with_capacity(65);
            point.push(0x04);
            point.extend_from_slice(&x);
            point.extend_from_slice(&y);
            let verifying = p256::ecdsa::VerifyingKey::from_sec1_bytes(&point)
                .map_err(|_| Rejection::UnknownKey)?;
            // JWS dùng dạng `r || s` 64 byte, không phải DER.
            let signature =
                p256::ecdsa::Signature::from_slice(signature).map_err(|_| Rejection::Signature)?;
            verifying.verify(input, &signature).map_err(|_| Rejection::Signature)
        }
    }
}

/// Điều `id_token` phải thoả.
pub(crate) struct Expected<'a> {
    /// `iss` mong đợi. Microsoft đa tenant: mẫu chứa `{tenantid}`, thay bằng claim `tid`.
    pub issuer: &'a str,
    /// `iss` thứ hai được nhận (Google còn phát `accounts.google.com` không có `https://`).
    pub alternative_issuer: Option<&'a str>,
    pub client_id: &'a str,
    pub nonce: &'a str,
    /// Giây UNIX lúc kiểm.
    pub now: i64,
}

/// Claim cần dùng sau khi kiểm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Claims {
    pub subject: String,
    pub email: Option<String>,
    /// Claim `email_verified` (chuẩn OIDC).
    pub email_verified: bool,
    /// Claim `xms_edov` của Microsoft Entra: tên miền của email đã được chủ tenant xác minh.
    pub email_domain_verified: bool,
    pub name: Option<String>,
}

/// Kiểm claim của `id_token` đã qua chữ ký.
pub(crate) fn check_claims(claims: &Value, expected: &Expected<'_>) -> Result<Claims, Rejection> {
    let text = |name: &str| claims.get(name).and_then(Value::as_str);
    let number = |name: &str| claims.get(name).and_then(Value::as_i64);
    let issuer = text("iss").ok_or(Rejection::Issuer)?;
    let wanted = if expected.issuer.contains("{tenantid}") {
        // Chỉ nhận `tid` dạng GUID: không để claim tự chế thành một issuer khác.
        let tid = text("tid")
            .filter(|tid| !tid.is_empty() && tid.chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
            .ok_or(Rejection::Issuer)?;
        expected.issuer.replace("{tenantid}", tid)
    } else {
        expected.issuer.to_owned()
    };
    if issuer != wanted && Some(issuer) != expected.alternative_issuer {
        return Err(Rejection::Issuer);
    }
    let audience_ok = match claims.get("aud") {
        Some(Value::String(aud)) => aud == expected.client_id,
        Some(Value::Array(list)) => list.iter().any(|aud| aud.as_str() == Some(expected.client_id)),
        _ => false,
    };
    if !audience_ok {
        return Err(Rejection::Audience);
    }
    if let Some(azp) = claims.get("azp")
        && azp.as_str() != Some(expected.client_id)
    {
        return Err(Rejection::Audience);
    }
    let exp = number("exp").ok_or(Rejection::Expired)?;
    if exp + LEEWAY_SECONDS < expected.now {
        return Err(Rejection::Expired);
    }
    let iat = number("iat").ok_or(Rejection::Malformed)?;
    if iat - LEEWAY_SECONDS > expected.now {
        return Err(Rejection::NotYetValid);
    }
    if number("nbf").is_some_and(|nbf| nbf - LEEWAY_SECONDS > expected.now) {
        return Err(Rejection::NotYetValid);
    }
    // `nonce` so bằng thời gian hằng: nó là bí mật của lượt đăng nhập này.
    let nonce = text("nonce").ok_or(Rejection::Nonce)?;
    if !constant_eq(nonce.as_bytes(), expected.nonce.as_bytes()) {
        return Err(Rejection::Nonce);
    }
    let subject = text("sub").filter(|sub| !sub.is_empty()).ok_or(Rejection::Malformed)?;
    Ok(Claims {
        subject: subject.to_owned(),
        email: text("email").map(str::to_owned).filter(|email| !email.is_empty()),
        email_verified: flag(claims.get("email_verified")),
        email_domain_verified: flag(claims.get("xms_edov")),
        name: text("name").map(str::to_owned).filter(|name| !name.is_empty()),
    })
}

/// Claim cờ: `true` hay chuỗi `"true"` (vài nhà cung cấp gửi chuỗi; Entra gửi `1`).
fn flag(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::String(text)) => text.eq_ignore_ascii_case("true") || text == "1",
        Some(Value::Number(number)) => number.as_i64() == Some(1),
        _ => false,
    }
}

fn constant_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len() && left.iter().zip(right).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    // RFC 7515 phụ lục A.2: JWS ký RS256 và khoá CÔNG KHAI của ví dụ (không có khoá riêng nào).
    const RFC7515_JWS: &str = concat!(
        "eyJhbGciOiJSUzI1NiJ9",
        ".",
        "eyJpc3MiOiJqb2UiLA0KICJleHAiOjEzMDA4MTkzODAsDQogImh0dHA6Ly9leGFt",
        "cGxlLmNvbS9pc19yb290Ijp0cnVlfQ",
        ".",
        "cC4hiUPoj9Eetdgtv3hF80EGrhuB__dzERat0XF9g2VtQgr9PJbu3XOiZj5RZmh7",
        "AAuHIm4Bh-0Qc_lF5YKt_O8W2Fp5jujGbds9uJdbF9CUAr7t1dnZcAcQjbKBYNX4",
        "BAynRFdiuB--f_nZLgrnbyTyWzO75vRK5h6xBArLIARNPvkSjtQBMHlb1L07Qe7K",
        "0GarZRmB_eSN9383LcOLn6_dO--xi12jzDwusC-eOkHWEsqtFZESc6BfI7noOPqv",
        "hJ1phCnvWh6IeYI2w9QOYEUipUTI8np6LbgGY9Fs98rqVt5AXLIhWkWywlVmtVrB",
        "p0igcN_IoypGlUPQGe77Rw",
    );
    const RFC7515_N: &str = concat!(
        "ofgWCuLjybRlzo0tZWJjNiuSfb4p4fAkd_wWJcyQoTbji9k0l8W26mPddx",
        "HmfHQp-Vaw-4qPCJrcS2mJPMEzP1Pt0Bm4d4QlL-yRT-SFd2lZS-pCgNMs",
        "D1W_YpRPEwOWvG6b32690r2jZ47soMZo9wGzjb_7OMg0LOL-bSf63kpaSH",
        "SXndS5z5rexMdbBYUsLA9e-KXBdQOS-UTo7WTBEMa2R2CapHg665xsmtdV",
        "MTBQY4uDZlxvb3qCo5ZwKh9kG4LT6_I5IhlJH7aGhyxXFvUK-DWNmoudF8",
        "NAco9_h9iaGNj8q2ethFkMLs91kzk2PAcDTW9gb54h4FRWyuXpoQ",
    );

    fn rsa_key(kid: Option<&str>) -> Jwk {
        Jwk {
            kty: "RSA".to_owned(),
            kid: kid.map(str::to_owned),
            usage: None,
            n: Some(RFC7515_N.to_owned()),
            e: Some("AQAB".to_owned()),
            crv: None,
            x: None,
            y: None,
        }
    }

    #[test]
    fn rs256_kiem_dung_vi_du_rfc7515() {
        let token = parse(RFC7515_JWS).unwrap();
        assert_eq!(token.algorithm, Algorithm::Rs256);
        assert_eq!(token.claims["iss"], "joe");
        let keys = JwkSet { keys: vec![rsa_key(None)] };
        let key = token.select(&keys).unwrap();
        token.verify(key).unwrap();
        // Sửa một ký tự của payload: chữ ký không còn khớp.
        let tampered = RFC7515_JWS.replacen("eyJpc3MiOiJqb2Ui", "eyJpc3MiOiJqb2Ii", 1);
        let token = parse(&tampered).unwrap();
        assert_eq!(token.verify(&rsa_key(None)), Err(Rejection::Signature));
    }

    #[test]
    fn chon_khoa_theo_kid() {
        let token = parse(RFC7515_JWS).unwrap();
        // Không có `kid`: chỉ chọn khi đúng một khoá cùng loại.
        let two = JwkSet { keys: vec![rsa_key(Some("a")), rsa_key(Some("b"))] };
        assert!(token.select(&two).is_none());
    }

    fn unsigned(header: &Value) -> String {
        let claims = URL_SAFE_NO_PAD.encode(br#"{"sub":"1"}"#);
        format!("{}.{claims}.", URL_SAFE_NO_PAD.encode(header.to_string()))
    }

    #[test]
    fn alg_none_va_hs256_bi_tu_choi() {
        for alg in ["none", "HS256", "RS512", "PS256"] {
            let token = unsigned(&json!({"alg": alg}));
            assert_eq!(parse(&token).unwrap_err(), Rejection::Algorithm, "{alg}");
        }
        assert_eq!(parse("a.b").unwrap_err(), Rejection::Malformed);
    }

    fn expected(now: i64) -> Expected<'static> {
        Expected {
            issuer: "https://issuer.example",
            alternative_issuer: None,
            client_id: "client",
            nonce: "n-1",
            now,
        }
    }

    #[test]
    fn claim_sai_bi_tu_choi() {
        let now = 1_800_000_000;
        let good = json!({
            "iss": "https://issuer.example", "aud": "client", "sub": "s", "nonce": "n-1",
            "exp": now + 300, "iat": now, "email": "a@example.com", "email_verified": "true",
        });
        let claims = check_claims(&good, &expected(now)).unwrap();
        assert!(claims.email_verified);
        let cases = [
            ("iss", json!("https://other.example"), Rejection::Issuer),
            ("aud", json!(["other"]), Rejection::Audience),
            ("azp", json!("other"), Rejection::Audience),
            ("exp", json!(now - 61), Rejection::Expired),
            ("iat", json!(now + 61), Rejection::NotYetValid),
            ("nbf", json!(now + 61), Rejection::NotYetValid),
            ("nonce", json!("n-2"), Rejection::Nonce),
        ];
        for (name, value, rejection) in cases {
            let mut claims = good.clone();
            claims[name] = value;
            assert_eq!(check_claims(&claims, &expected(now)), Err(rejection), "{name}");
        }
        // Trong khoảng lệch giờ thì vẫn nhận.
        let mut late = good.clone();
        late["exp"] = json!(now - 30);
        assert!(check_claims(&late, &expected(now)).is_ok());
    }

    #[test]
    fn issuer_microsoft_theo_tid() {
        let now = 1_800_000_000;
        let template = "https://login.example/{tenantid}/v2.0";
        let wanted = Expected { issuer: template, ..expected(now) };
        let claims = json!({
            "iss": "https://login.example/1f2e-3d/v2.0", "tid": "1f2e-3d", "aud": "client",
            "sub": "s", "nonce": "n-1", "exp": now + 60, "iat": now,
        });
        assert!(check_claims(&claims, &wanted).is_ok());
        let mut forged = claims.clone();
        forged["tid"] = json!("evil/../x");
        assert_eq!(check_claims(&forged, &wanted), Err(Rejection::Issuer));
    }
}
