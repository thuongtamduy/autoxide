//! Authenticator phần mềm cho test: làm đúng việc của điện thoại hay khoá bảo mật — tạo
//! khoá P-256, trả lời nghi thức đăng ký (attestation "none") và đăng nhập (chữ ký trên
//! `authenticatorData || SHA-256(clientDataJSON)`), theo W3C WebAuthn Level 3 và CTAP2.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::{Signature, SigningKey};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub struct SoftAuthenticator {
    key: SigningKey,
    pub credential_id: Vec<u8>,
    /// User handle máy chủ gửi lúc đăng ký (authenticator nhớ và gửi lại khi đăng nhập).
    pub user_handle: Vec<u8>,
    counter: u32,
    pub origin: String,
    rp_id: String,
    /// Loại authenticator khai lúc đăng ký (mặc định toàn số 0: không khai).
    pub aaguid: [u8; 16],
}

impl SoftAuthenticator {
    /// `seed` khác nhau cho khoá và credential ID khác nhau.
    pub fn new(seed: u8) -> Self {
        SoftAuthenticator {
            key: SigningKey::from_bytes(&[seed.max(1); 32].into()).unwrap(),
            credential_id: vec![seed; 16],
            user_handle: Vec::new(),
            counter: 0,
            origin: "https://example.com".to_owned(),
            rp_id: "example.com".to_owned(),
            aaguid: [0; 16],
        }
    }

    /// Authenticator của trang trên tên miền `rp_id` (RP ID và origin `https://<rp_id>`).
    pub fn on(mut self, rp_id: &str) -> Self {
        self.rp_id = rp_id.to_owned();
        self.origin = format!("https://{rp_id}");
        self
    }

    /// Câu trả lời cho tuỳ chọn đăng ký `options` (JSON máy chủ gửi).
    pub fn register(&mut self, options: &Value) -> Value {
        self.user_handle = B64.decode(options["user"]["id"].as_str().unwrap()).unwrap();
        let point = self.key.verifying_key().to_encoded_point(false);
        // Khoá COSE EC2 / ES256, khoá sắp theo thứ tự CBOR chuẩn của CTAP2: 1, 3, -1, -2, -3.
        let mut cose = vec![0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21];
        bytes(&mut cose, point.x().unwrap());
        cose.push(0x22);
        bytes(&mut cose, point.y().unwrap());
        let mut data = self.header(0x01 | 0x04 | 0x40);
        data.extend_from_slice(&self.aaguid); // AAGUID
        data.extend_from_slice(&u16::try_from(self.credential_id.len()).unwrap().to_be_bytes());
        data.extend_from_slice(&self.credential_id);
        data.extend_from_slice(&cose);
        let mut attestation = vec![0xa3];
        text(&mut attestation, "fmt");
        text(&mut attestation, "none");
        text(&mut attestation, "attStmt");
        attestation.push(0xa0);
        text(&mut attestation, "authData");
        bytes(&mut attestation, &data);
        let client = self.client_data("webauthn.create", options);
        json!({
            "id": B64.encode(&self.credential_id),
            "rawId": B64.encode(&self.credential_id),
            "type": "public-key",
            "response": {
                "clientDataJSON": B64.encode(client),
                "attestationObject": B64.encode(attestation),
                "transports": ["internal"],
            },
            "clientExtensionResults": {},
            "authenticatorAttachment": "platform",
        })
    }

    /// Câu trả lời cho tuỳ chọn đăng nhập `options`.
    pub fn sign_in(&mut self, options: &Value) -> Value {
        self.counter += 1;
        let data = self.header(0x01 | 0x04);
        let client = self.client_data("webauthn.get", options);
        let mut signed = data.clone();
        signed.extend_from_slice(&Sha256::digest(client.as_bytes()));
        let signature: Signature = self.key.sign(&signed);
        json!({
            "id": B64.encode(&self.credential_id),
            "rawId": B64.encode(&self.credential_id),
            "type": "public-key",
            "response": {
                "clientDataJSON": B64.encode(client),
                "authenticatorData": B64.encode(data),
                "signature": B64.encode(signature.to_der().as_bytes()),
                "userHandle": B64.encode(&self.user_handle),
            },
            "clientExtensionResults": {},
            "authenticatorAttachment": "platform",
        })
    }

    /// RP ID hash, cờ, bộ đếm chữ ký.
    fn header(&self, flags: u8) -> Vec<u8> {
        let mut data = Sha256::digest(self.rp_id.as_bytes()).to_vec();
        data.push(flags);
        data.extend_from_slice(&self.counter.to_be_bytes());
        data
    }

    fn client_data(&self, kind: &str, options: &Value) -> String {
        json!({
            "type": kind,
            "challenge": options["challenge"],
            "origin": self.origin,
            "crossOrigin": false,
        })
        .to_string()
    }
}

/// Chuỗi byte CBOR (major type 2).
fn bytes(out: &mut Vec<u8>, value: &[u8]) {
    head(out, 0x40, value.len());
    out.extend_from_slice(value);
}

/// Chuỗi chữ CBOR (major type 3).
fn text(out: &mut Vec<u8>, value: &str) {
    head(out, 0x60, value.len());
    out.extend_from_slice(value.as_bytes());
}

fn head(out: &mut Vec<u8>, major: u8, length: usize) {
    match u8::try_from(length) {
        Ok(small) if small < 24 => out.push(major | small),
        Ok(byte) => out.extend_from_slice(&[major | 24, byte]),
        Err(_) => {
            out.push(major | 25);
            out.extend_from_slice(&u16::try_from(length).unwrap().to_be_bytes());
        }
    }
}
