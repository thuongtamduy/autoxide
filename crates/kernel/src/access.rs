//! Quyền truy cập: permission do module khai, và "ai đang gọi" ([`Principal`]).
//!
//! - Mỗi module khai các permission của mình qua `Module::permissions`. Tên viết
//!   `<module>.<việc>` (vd `notes.write`, `auth.roles.manage`); core kiểm tên lúc
//!   khởi động và gom thành một danh mục ([`PermissionCatalog`]).
//! - Một module (thường là `auth`) công bố trait [`Authenticator`]: đọc header
//!   `Authorization` và trả về [`Principal`]. Kernel gắn nó vào mọi request.
//! - Handler cần đăng nhập nhận `principal: Principal` làm tham số, rồi gọi
//!   `principal.require(&NOTES_WRITE)?`. Thiếu quyền thì trả 403 `permission_denied`.
//!
//! Nhờ vậy module nghiệp vụ kiểm quyền mà không phụ thuộc module `auth`, và module
//! `auth` có thể thay bằng cách xác thực khác mà không module nào phải sửa.

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::extract::FromRequestParts;
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
}

impl Permission {
    /// `name`: `<module>.<việc>`, các đoạn là chữ thường, số và `_`.
    /// `description`: câu tiếng Anh cho người cấp quyền đọc (hiện trong API).
    pub const fn new(name: &'static str, description: &'static str) -> Self {
        Permission { name, description }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn description(&self) -> &'static str {
        self.description
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
        Principal { user_id, grants: Grants::Only(Arc::new(permissions.into_iter().collect())) }
    }

    /// Người dùng `user_id` có mọi permission.
    pub fn with_all_permissions(user_id: Uuid) -> Self {
        Principal { user_id, grants: Grants::All }
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
        match &self.grants {
            Grants::All => true,
            Grants::Only(set) => set.contains(permission.name),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;

    const READ: Permission = Permission::new("notes.read", "Read notes.");
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
        assert_eq!(reader.user_id(), user);
    }
}
