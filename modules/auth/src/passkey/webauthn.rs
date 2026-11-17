//! Lớp mỏng quanh thư viện `webauthn_rp`: mọi kiểu phức tạp của thư viện (generic,
//! lifetime) ở lại trong file này; phần còn lại của module chỉ thấy byte và JSON.
//!
//! Thư viện kiểm đúng các bước của đặc tả W3C WebAuthn Level 3 (challenge, origin, RP
//! ID, cờ người dùng có mặt / đã xác minh, chữ ký, bộ đếm chữ ký). Ở đây chỉ chuyển dữ
//! liệu vào ra.

use serde_json::Value;
use webauthn_rp::bin::{Decode, Encode};
use webauthn_rp::request::auth::AuthenticationVerificationOptions;
use webauthn_rp::request::register::{
    CredProtect, Nickname, PublicKeyCredentialUserEntity, RegistrationVerificationOptions,
    UserHandle64, Username,
};
use webauthn_rp::request::{AsciiDomain, ExtensionInfo, PublicKeyCredentialDescriptor, RpId};
use webauthn_rp::response::auth::ser_relaxed::DiscoverableAuthenticationRelaxed64;
use webauthn_rp::response::register::ser_relaxed::RegistrationRelaxed;
use webauthn_rp::response::register::{CompressedPubKey, DynamicState, StaticState};
use webauthn_rp::response::{AuthTransports, CredentialId};
use webauthn_rp::{
    AuthenticatedCredential, DiscoverableAuthentication64, DiscoverableAuthenticationServerState,
    DiscoverableCredentialRequestOptions, PublicKeyCredentialCreationOptions,
    RegistrationServerState,
};

use crate::config::PasskeyConfig;

/// Khoá công khai đã lưu, dạng thư viện đọc lại được.
type StoredKey = CompressedPubKey<[u8; 32], [u8; 32], [u8; 48], Vec<u8>>;

/// Lỗi của một bước: câu tiếng Anh nói rõ chỗ sai (cho người tích hợp đọc).
pub(super) struct Rejected(pub String);

/// Một nghi thức vừa bắt đầu: trạng thái giữ ở máy chủ và tuỳ chọn gửi cho trình duyệt.
pub(super) struct Started {
    pub state: Vec<u8>,
    pub options: Value,
}

/// Passkey vừa đăng ký xong, các phần cần lưu.
pub(super) struct Registered {
    pub credential_id: Vec<u8>,
    pub user_handle: Vec<u8>,
    pub static_state: Vec<u8>,
    pub dynamic_state: Vec<u8>,
    /// Loại authenticator nó tự khai (AAGUID); `None` khi không khai.
    pub aaguid: Option<uuid::Uuid>,
}

/// Câu trả lời đăng nhập đã đọc: credential nào, của user handle nào.
pub(super) struct Assertion {
    response: DiscoverableAuthentication64,
}

impl Assertion {
    pub fn credential_id(&self) -> Vec<u8> {
        self.response.raw_id().as_ref().to_vec()
    }
}

pub(super) struct Engine {
    rp_id: RpId,
    origins: Vec<String>,
}

impl Engine {
    pub fn new(config: &PasskeyConfig) -> Result<Self, Rejected> {
        let domain = AsciiDomain::try_from(config.rp_id.clone())
            .map_err(|_| Rejected(format!("`{}` is not a valid RP ID", config.rp_id)))?;
        Ok(Engine { rp_id: RpId::Domain(domain), origins: config.origins.clone() })
    }

    /// User handle mới, ngẫu nhiên 64 byte (khuyến nghị của đặc tả: không chứa thông tin
    /// cá nhân).
    pub fn new_user_handle() -> Vec<u8> {
        UserHandle64::new().as_ref().to_vec()
    }

    /// Bắt đầu đăng ký một passkey cho người dùng `handle`; `existing` là credential ID đã
    /// có của người đó (để authenticator không tạo trùng).
    pub fn start_registration(
        &self,
        handle: &[u8],
        email: &str,
        name: &str,
        existing: &[Vec<u8>],
    ) -> Result<Started, Rejected> {
        let handle = user_handle(handle)?;
        let username = Username::try_from(email)
            .map_err(|_| Rejected("the account email cannot be used as a passkey name".into()))?;
        let entity = PublicKeyCredentialUserEntity {
            name: username,
            id: &handle,
            // Tên hiển thị không hợp luật Nickname (RFC 8266) thì để trống.
            display_name: Nickname::try_from(name).ok(),
        };
        let mut exclude = Vec::with_capacity(existing.len());
        for id in existing {
            let id = CredentialId::<&[u8]>::decode(id.as_slice())
                .map_err(|_| Rejected("a stored credential id is damaged".into()))?;
            let transports = AuthTransports::decode(0)
                .map_err(|_| Rejected("cannot build the transports".into()))?;
            exclude.push(PublicKeyCredentialDescriptor { id: id.into(), transports });
        }
        let mut creation =
            PublicKeyCredentialCreationOptions::passkey(&self.rp_id, entity, exclude);
        // Xin `credProtect = userVerificationRequired` nhưng không ÉP (D-334): nhiều authenticator
        // (Chromium ảo, một số khoá / thiết bị cũ) không hỗ trợ phần mở rộng này và từ chối hẳn khi bị
        // ép. Đăng nhập bằng passkey vẫn luôn đòi xác minh người dùng (`authenticator_selection` của
        // passkey), nên không yếu đi.
        creation.extensions.cred_protect =
            CredProtect::UserVerificationRequired(ExtensionInfo::AllowDontEnforceValue);
        let (server, client) =
            creation.start_ceremony().map_err(|error| Rejected(error.to_string()))?;
        let options = serde_json::to_value(&client).map_err(|error| Rejected(error.to_string()))?;
        let state = server.encode().map_err(|error| Rejected(error.to_string()))?;
        Ok(Started { state, options })
    }

    /// Kiểm câu trả lời đăng ký `credential` (JSON `RegistrationResponseJSON` của trình
    /// duyệt) với trạng thái `state` đã lưu.
    pub fn finish_registration(
        &self,
        state: &[u8],
        credential: &Value,
    ) -> Result<Registered, Rejected> {
        let response = serde_json::from_value::<RegistrationRelaxed>(credential.clone())
            .map_err(|error| {
                Rejected(format!("the credential is not a registration response: {error}"))
            })?
            .0;
        let server = RegistrationServerState::<64>::decode(state)
            .map_err(|_| Rejected("the stored ceremony is damaged".into()))?;
        let options = RegistrationVerificationOptions::<String, String> {
            allowed_origins: &self.origins,
            ..Default::default()
        };
        let registered = server
            .verify(&self.rp_id, &response, &options)
            .map_err(|error| Rejected(error.to_string()))?;
        Ok(Registered {
            aaguid: super::authenticators::from_bytes(registered.metadata().aaguid.data()),
            credential_id: registered.id().as_ref().to_vec(),
            user_handle: registered.user_id().as_ref().to_vec(),
            static_state: registered
                .static_state()
                .encode()
                .map_err(|error| Rejected(error.to_string()))?,
            dynamic_state: registered
                .dynamic_state()
                .encode()
                .map_err(|error| Rejected(error.to_string()))?
                .to_vec(),
        })
    }

    /// Bắt đầu đăng nhập: không cần biết trước người dùng (passkey tự nói nó là ai).
    pub fn start_login(&self) -> Result<Started, Rejected> {
        let (server, client) = DiscoverableCredentialRequestOptions::passkey(&self.rp_id)
            .start_ceremony()
            .map_err(|error| Rejected(error.to_string()))?;
        let options = serde_json::to_value(&client).map_err(|error| Rejected(error.to_string()))?;
        let state = server.encode().map_err(|error| Rejected(error.to_string()))?;
        Ok(Started { state, options })
    }

    /// Đọc câu trả lời đăng nhập (JSON `AuthenticationResponseJSON`).
    pub fn read_assertion(&self, credential: &Value) -> Result<Assertion, Rejected> {
        let response =
            serde_json::from_value::<DiscoverableAuthenticationRelaxed64>(credential.clone())
                .map_err(|error| {
                    Rejected(format!("the credential is not an authentication response: {error}"))
                })?
                .0;
        Ok(Assertion { response })
    }

    /// Kiểm câu trả lời đăng nhập với trạng thái `state` và passkey đã lưu; trả trạng thái
    /// động mới (bộ đếm chữ ký…) nếu nó đổi.
    pub fn finish_login(
        &self,
        state: &[u8],
        assertion: &Assertion,
        stored_handle: &[u8],
        static_state: &[u8],
        dynamic_state: &[u8],
    ) -> Result<Option<Vec<u8>>, Rejected> {
        let damaged = || Rejected("the stored passkey is damaged".into());
        let handle = user_handle(stored_handle)?;
        // Credential phải thuộc đúng người mà authenticator nói (đặc tả, mục 7.2).
        if assertion.response.response().user_handle() != &handle {
            return Err(Rejected("the passkey belongs to another account".into()));
        }
        let static_state = StaticState::<StoredKey>::decode(static_state).map_err(|_| damaged())?;
        let dynamic: [u8; 7] = dynamic_state.try_into().map_err(|_| damaged())?;
        let dynamic = DynamicState::decode(dynamic).map_err(|_| damaged())?;
        let mut credential = AuthenticatedCredential::new(
            assertion.response.raw_id(),
            &handle,
            static_state,
            dynamic,
        )
        .map_err(|error| Rejected(error.to_string()))?;
        let server = DiscoverableAuthenticationServerState::decode(state)
            .map_err(|_| Rejected("the stored ceremony is damaged".into()))?;
        let options = AuthenticationVerificationOptions::<String, String> {
            allowed_origins: &self.origins,
            ..Default::default()
        };
        let changed = server
            .verify(&self.rp_id, &assertion.response, &mut credential, &options)
            .map_err(|error| Rejected(error.to_string()))?;
        if !changed {
            return Ok(None);
        }
        let encoded =
            credential.dynamic_state().encode().map_err(|error| Rejected(error.to_string()))?;
        Ok(Some(encoded.to_vec()))
    }
}

fn user_handle(bytes: &[u8]) -> Result<UserHandle64, Rejected> {
    let damaged = || Rejected("the stored user handle is damaged".into());
    let array: [u8; 64] = bytes.try_into().map_err(|_| damaged())?;
    UserHandle64::decode(array).map_err(|_| damaged())
}
