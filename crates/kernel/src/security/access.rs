//! Quyền truy cập: permission do module khai, và "ai đang gọi" ([`Principal`]).
//!
//! - Mỗi module khai các permission của mình qua `Module::permissions`. Tên viết
//!   `<module>.<việc>` (vd `notes.write`, `auth.roles.manage`); core kiểm tên lúc
//!   khởi động và gom thành một danh mục ([`PermissionCatalog`]).
//! - Một module (thường là `auth`) công bố trait [`Authenticator`]: đọc header
//!   `Authorization` và trả về [`Principal`]. Kernel gắn nó vào mọi request.
//! - Handler cần đăng nhập nhận `principal: Principal` làm tham số, rồi gọi
//!   `principal.require(&NOTES_WRITE)?`. Thiếu quyền thì trả 403 `permission_denied`.
//! - Endpoint công khai (ai cũng gọi được, người đăng nhập thấy nhiều hơn) nhận
//!   `principal: Option<Principal>`: không gửi `Authorization` thì `None`; gửi mà sai
//!   thì vẫn 401 như endpoint thường — không lặng lẽ coi như khách. Tài liệu OpenAPI của
//!   handler đó khai `security((), ("bearer" = []))`.
//!
//! Nhờ vậy module nghiệp vụ kiểm quyền mà không phụ thuộc module `auth`, và module
//! `auth` có thể thay bằng cách xác thực khác mà không module nào phải sửa.

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::extract::{FromRequestParts, OptionalFromRequestParts};
use axum::http::header;
use axum::http::request::Parts;
use uuid::Uuid;

use crate::error::Error;
use crate::module::is_identifier;

/// Một permission do module khai.
///
/// ```
/// use autoxide_kernel::access::Permission;
///
/// pub const NOTES_WRITE: Permission =
///     Permission::new("notes.write", "Create, change and delete notes.");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Permission {
    name: &'static str,
    description: &'static str,
    risk: Risk,
}

/// Mức rủi ro của một permission, đọc được bằng máy (D-205, đặc tả 54 P1): giao diện và AI agent
/// biết trước việc nào chỉ đọc, việc nào cần cẩn thận; tool MCP chỉ đọc chỉ được đòi permission
/// mức [`Risk::Read`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Risk {
    /// Chỉ xem.
    Read,
    /// Tạo, sửa.
    Write,
    /// Xoá hay ghi đè không lấy lại được.
    Destructive,
    /// Xem hay xuất dữ liệu cá nhân của người khác.
    SensitiveData,
    /// Chạm tới tiền.
    Money,
}

impl Risk {
    /// Tên ổn định trong API: `read`, `write`, `destructive`, `sensitive_data`, `money`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Risk::Read => "read",
            Risk::Write => "write",
            Risk::Destructive => "destructive",
            Risk::SensitiveData => "sensitive_data",
            Risk::Money => "money",
        }
    }
}

/// `text` kết thúc bằng `suffix` (dùng được trong `const fn`).
const fn ends_with(text: &str, suffix: &str) -> bool {
    let (text, suffix) = (text.as_bytes(), suffix.as_bytes());
    if suffix.len() > text.len() {
        return false;
    }
    let offset = text.len() - suffix.len();
    let mut index = 0;
    while index < suffix.len() {
        if text[offset + index] != suffix[index] {
            return false;
        }
        index += 1;
    }
    true
}

impl Permission {
    /// `name`: `<module>.<việc>`, các đoạn là chữ thường, số và `_`.
    /// `description`: câu tiếng Anh cho người cấp quyền đọc (hiện trong API).
    ///
    /// Mức rủi ro suy từ tên: `….read` là [`Risk::Read`], còn lại [`Risk::Write`]; khác thì đặt bằng
    /// [`with_risk`](Self::with_risk).
    pub const fn new(name: &'static str, description: &'static str) -> Self {
        let risk = if ends_with(name, ".read") { Risk::Read } else { Risk::Write };
        Permission { name, description, risk }
    }

    /// Đặt mức rủi ro, vd `Permission::new("notes.purge", "…").with_risk(Risk::Destructive)`.
    pub const fn with_risk(mut self, risk: Risk) -> Self {
        self.risk = risk;
        self
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn description(&self) -> &'static str {
        self.description
    }

    pub const fn risk(&self) -> Risk {
        self.risk
    }
}

/// Mọi permission của chương trình, theo thứ tự tên.
#[derive(Debug, Clone, Default)]
pub struct PermissionCatalog {
    // `Arc`: danh mục dựng một lần lúc khởi động rồi chia cho nhiều module đọc.
    permissions: Arc<Vec<Permission>>,
}

impl PermissionCatalog {
    /// Kiểm và gom permission của các module (`(tên module, permission của nó)`).
    /// Trả về mọi vấn đề cùng lúc nếu có.
    pub fn new(modules: Vec<(&'static str, Vec<Permission>)>) -> Result<Self, Vec<String>> {
        let mut problems = Vec::new();
        let mut seen = BTreeSet::new();
        let mut permissions = Vec::new();
        for (module, declared) in modules {
            for permission in declared {
                problems.extend(name_problem(module, permission.name));
                if permission.description.trim().is_empty() {
                    problems.push(format!(
                        "permission `{}` of module `{module}` has no description",
                        permission.name
                    ));
                }
                if !seen.insert(permission.name) {
                    problems.push(format!("permission `{}` is declared twice", permission.name));
                }
                permissions.push(permission);
            }
        }
        if !problems.is_empty() {
            return Err(problems);
        }
        permissions.sort();
        Ok(PermissionCatalog { permissions: Arc::new(permissions) })
    }

    /// Có permission tên `name` không.
    pub fn contains(&self, name: &str) -> bool {
        self.permissions.iter().any(|permission| permission.name == name)
    }

    /// Mọi permission, theo thứ tự tên.
    pub fn iter(&self) -> impl Iterator<Item = &Permission> {
        self.permissions.iter()
    }
}

/// Vấn đề của tên permission `name` khai bởi module `module`, nếu có.
fn name_problem(module: &str, name: &str) -> Option<String> {
    let mut segments = name.split('.');
    let valid = segments.next() == Some(module)
        && segments.clone().next().is_some()
        && segments.all(is_identifier);
    (!valid).then(|| {
        format!(
            "permission `{name}` of module `{module}` must be named `{module}.<action>`, with \
             segments of lowercase letters, digits and `_` (e.g. `{module}.write`)"
        )
    })
}

/// Bên đang gọi API, đã xác thực: người dùng nào và có những permission nào.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    user_id: Uuid,
    grants: Grants,
    /// ID của API key, khi request đến bằng API key thay vì phiên đăng nhập.
    api_key: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Grants {
    /// Mọi permission, kể cả permission thêm sau (role `admin`).
    All,
    Only(Arc<BTreeSet<String>>),
}

impl Principal {
    /// Người dùng `user_id` với đúng các permission `permissions`.
    pub fn new(user_id: Uuid, permissions: impl IntoIterator<Item = String>) -> Self {
        Principal {
            user_id,
            grants: Grants::Only(Arc::new(permissions.into_iter().collect())),
            api_key: None,
        }
    }

    /// Người dùng `user_id` có mọi permission.
    pub fn with_all_permissions(user_id: Uuid) -> Self {
        Principal { user_id, grants: Grants::All, api_key: None }
    }

    /// Đánh dấu request đến bằng API key `id`.
    pub fn via_api_key(mut self, id: Uuid) -> Self {
        self.api_key = Some(id);
        self
    }

    /// ID của API key nếu request đến bằng API key.
    pub fn api_key_id(&self) -> Option<Uuid> {
        self.api_key
    }

    pub fn user_id(&self) -> Uuid {
        self.user_id
    }

    /// Có mọi permission (kể cả permission chưa tồn tại lúc cấp) không.
    pub fn has_all_permissions(&self) -> bool {
        self.grants == Grants::All
    }

    /// Các permission được cấp rõ ràng, theo thứ tự tên. Rỗng khi
    /// [`has_all_permissions`](Self::has_all_permissions).
    pub fn permissions(&self) -> Vec<String> {
        match &self.grants {
            Grants::All => Vec::new(),
            Grants::Only(set) => set.iter().cloned().collect(),
        }
    }

    pub fn has(&self, permission: &Permission) -> bool {
        self.allows(permission.name)
    }

    /// Như [`has`](Self::has), theo tên permission (vd tên do client gửi lên).
    pub fn allows(&self, permission: &str) -> bool {
        match &self.grants {
            Grants::All => true,
            Grants::Only(set) => set.contains(permission),
        }
    }

    /// `Ok` nếu có `permission`; không thì lỗi 403 `permission_denied` nêu tên nó.
    pub fn require(&self, permission: &Permission) -> Result<(), Error> {
        if self.has(permission) {
            return Ok(());
        }
        Err(Error::forbidden(
            "permission_denied",
            "This action needs the permission `{permission}`. Ask an administrator for a role \
             that has it.",
        )
        .with_param("permission", permission.name))
    }
}

/// Xác thực request: đọc giá trị header `Authorization` và trả về bên đang gọi.
/// Một module công bố trait này lúc khởi động
/// (`context.provide::<dyn Authenticator>(…)`); kernel gắn nó vào mọi request.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`.
#[async_trait::async_trait]
pub trait Authenticator: Send + Sync {
    /// `authorization` là nguyên giá trị header (`None` khi không có). Lỗi trả về
    /// thường là 401 (thiếu, sai, hết hạn) hoặc 403 (tài khoản bị khoá).
    async fn authenticate(&self, authorization: Option<&str>) -> Result<Principal, Error>;
}

/// Hỏi "tài khoản này đã xác thực (số điện thoại / định danh) để đăng nội dung chưa" — điểm nối
/// cho yêu cầu xác thực tài khoản đăng nội dung của Nghị định 147/2024/NĐ-CP (D-202). Module giữ
/// trạng thái xác thực (bước 37) đăng ký bằng
/// [`StartContext::posting_verifier`](crate::module::StartContext::posting_verifier); module nhận
/// nội dung của người dùng (vd `comments`) hỏi qua [`PostingVerification`].
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait PostingVerifier: Send + Sync {
    /// Người dùng `user_id` đã xác thực để đăng nội dung chưa.
    async fn is_verified(&self, user_id: Uuid) -> Result<bool, Error>;
}

/// Chỗ giữ bộ [`PostingVerifier`] duy nhất của chương trình. Clone rẻ: mọi bản clone thấy cùng
/// một chỗ, kể cả khi module đăng ký khởi động SAU module hỏi.
///
/// Chưa module nào đăng ký thì mọi tài khoản đăng nhập được tính là đã xác thực — giữ hành vi
/// trước D-202; khách (không tài khoản) thì bên hỏi tự từ chối.
#[derive(Clone, Default)]
pub struct PostingVerification {
    // Ghi một lần lúc khởi động; đọc mỗi lần hỏi. Không giữ khoá qua `await`.
    verifier: Arc<std::sync::RwLock<Option<Registered>>>,
}

/// Bộ hỏi đã đăng ký và module đăng ký nó.
type Registered = (&'static str, Arc<dyn PostingVerifier>);

impl std::fmt::Debug for PostingVerification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostingVerification").field("registered", &self.is_registered()).finish()
    }
}

impl PostingVerification {
    /// Đăng ký bộ hỏi của module `module`. Chỉ một module được đăng ký.
    pub fn register(
        &self,
        module: &'static str,
        verifier: Arc<dyn PostingVerifier>,
    ) -> Result<(), Error> {
        let mut slot = self.verifier.write().map_err(|_| poisoned_verification())?;
        if let Some((owner, _)) = slot.as_ref() {
            return Err(Error::internal(format!(
                "module `{module}` registers an account verifier, but module `{owner}` already \
                 does; there is exactly one"
            )));
        }
        *slot = Some((module, verifier));
        Ok(())
    }

    /// Đã có module đăng ký bộ hỏi chưa.
    pub fn is_registered(&self) -> bool {
        self.verifier.read().is_ok_and(|slot| slot.is_some())
    }

    /// Người dùng `user_id` đã xác thực chưa; `true` khi chưa module nào đăng ký bộ hỏi.
    pub async fn is_verified(&self, user_id: Uuid) -> Result<bool, Error> {
        let verifier = {
            let slot = self.verifier.read().map_err(|_| poisoned_verification())?;
            slot.as_ref().map(|(_, verifier)| verifier.clone())
        };
        match verifier {
            Some(verifier) => verifier.is_verified(user_id).await,
            None => Ok(true),
        }
    }
}

fn poisoned_verification() -> Error {
    Error::internal("the account verification slot is unusable after a panic")
}

// Handler viết `principal: Principal` thì axum gọi hàm này trước khi chạy handler:
// request không xác thực được bị trả lỗi ngay, handler không bao giờ chạy.
impl<S: Send + Sync> FromRequestParts<S> for Principal {
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let authenticator =
            parts.extensions.get::<Arc<dyn Authenticator>>().cloned().ok_or_else(|| {
                Error::internal(
                    "an endpoint needs a signed-in user, but no module provides \
                     `dyn Authenticator`; register the `auth` module",
                )
            })?;
        let authorization =
            parts.headers.get(header::AUTHORIZATION).and_then(|value| value.to_str().ok());
        authenticator.authenticate(authorization).await
    }
}

// Handler viết `principal: Option<Principal>`: endpoint công khai. Không có header
// `Authorization` thì `None` (kể cả khi chưa module nào xác thực); có thì xác thực như
// trên, sai là lỗi.
impl<S: Send + Sync> OptionalFromRequestParts<S> for Principal {
    type Rejection = Error;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> Result<Option<Self>, Self::Rejection> {
        if !parts.headers.contains_key(header::AUTHORIZATION) {
            return Ok(None);
        }
        <Principal as FromRequestParts<S>>::from_request_parts(parts, state).await.map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;

    const READ: Permission = Permission::new("notes.read", "Read notes.");

    struct OnlyOne(Uuid);

    #[async_trait::async_trait]
    impl PostingVerifier for OnlyOne {
        async fn is_verified(&self, user_id: Uuid) -> Result<bool, Error> {
            Ok(user_id == self.0)
        }
    }

    #[tokio::test]
    async fn xac_thuc_tai_khoan_mac_dinh_cho_qua_khi_chua_ai_dang_ky() {
        let slot = PostingVerification::default();
        let early = slot.clone();
        let (verified, other) = (Uuid::now_v7(), Uuid::now_v7());
        assert!(!slot.is_registered());
        assert!(slot.is_verified(other).await.unwrap(), "no verifier: as before D-202");
        slot.register("auth", Arc::new(OnlyOne(verified))).unwrap();
        assert!(slot.register("compliance", Arc::new(OnlyOne(verified))).is_err(), "one only");
        assert!(early.is_registered(), "clones see a later registration");
        assert!(early.is_verified(verified).await.unwrap());
        assert!(!early.is_verified(other).await.unwrap());
    }
    const WRITE: Permission = Permission::new("notes.write", "Write notes.");

    #[test]
    fn danh_muc_hop_le_sap_theo_ten() {
        let catalog = PermissionCatalog::new(vec![
            ("notes", vec![WRITE, READ]),
            ("auth", vec![Permission::new("auth.roles.manage", "Manage roles.")]),
        ])
        .unwrap();

        let names: Vec<&str> = catalog.iter().map(Permission::name).collect();
        assert_eq!(names, ["auth.roles.manage", "notes.read", "notes.write"]);
        assert!(catalog.contains("notes.read"));
        assert!(!catalog.contains("notes.delete"));
    }

    #[test]
    fn ten_sai_trung_hoac_thieu_mo_ta_bi_bao_cung_luc() {
        let problems = PermissionCatalog::new(vec![(
            "notes",
            vec![
                Permission::new("notes", "No action."),
                Permission::new("tags.read", "Wrong module."),
                Permission::new("notes.Write", "Upper case."),
                Permission::new("notes..read", "Empty segment."),
                Permission::new("notes.read", "First."),
                Permission::new("notes.read", " "),
            ],
        )])
        .unwrap_err();

        assert_eq!(problems.len(), 6, "{problems:#?}");
        assert!(problems[0].contains("must be named `notes.<action>`"), "{problems:#?}");
        assert!(problems.iter().any(|problem| problem.contains("declared twice")));
        assert!(problems.iter().any(|problem| problem.contains("has no description")));
    }

    #[test]
    fn principal_kiem_permission() {
        let user = Uuid::now_v7();
        let reader = Principal::new(user, ["notes.read".to_owned()]);
        let admin = Principal::with_all_permissions(user);

        assert!(reader.require(&READ).is_ok());
        let denied = reader.require(&WRITE).unwrap_err();
        assert_eq!(denied.kind(), ErrorKind::Forbidden);
        assert_eq!(denied.code(), "permission_denied");
        assert_eq!(
            denied.message(),
            "This action needs the permission `notes.write`. Ask an administrator for a role \
             that has it."
        );
        assert!(admin.require(&WRITE).is_ok());
        assert!(admin.has_all_permissions());
        assert_eq!(reader.permissions(), ["notes.read"]);
        assert!(reader.allows("notes.read") && !reader.allows("notes.write"));
        assert_eq!(reader.api_key_id(), None);
        let key = Uuid::now_v7();
        assert_eq!(reader.clone().via_api_key(key).api_key_id(), Some(key));
        assert_eq!(reader.user_id(), user);
    }
}
