//! Token trong link của thư (huỷ đăng ký, xác nhận, xác minh email…) mang khách của nó (bước 33,
//! D-281). Một cách làm cho mọi module gửi link có token: `newsletter`, `comments`, sau này `auth`.
//!
//! Người nhận bấm link từ hộp thư: request có thể tới tên miền chính, không header
//! `Autoxide-Tenant` — không biết khách nào. Nên token của khách khác khách mặc định ghi ID khách
//! ngay trong chuỗi: `t<UUID 32 chữ hex>.<phần thô>` ([`link_token`]). Endpoint tìm khách từ token
//! ([`resolve_link_token`]) TRƯỚC khi tra bản băm của phần thô trong database của khách đó. Token
//! của khách mặc định giữ nguyên dạng cũ (chỉ phần thô): cài đặt một khách và thư đã gửi không đổi.
//!
//! Database chỉ lưu bản băm của **phần thô**, tra trong khách đã định tuyến. Ghép ID của khách
//! khác với phần thô của khách này thì tra ở khách kia, không thấy — như token sai. Phần thô không
//! được chứa dấu `.` (vd base64url), nên có dấu `.` nghĩa là có tiền tố khách.
//!
//! Handler nhận token trong link dùng [`OptionalTenant`] thay cho `TenantContext`: khách của
//! request nếu xác định được, `None` khi không (tên miền chính không header) — thay vì
//! `tenant_required`.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use uuid::Uuid;

use super::{DEFAULT_TENANT, DatabaseRegistry, TenantContext, TenantId, ensure_serving};
use crate::error::{Error, ErrorKind};

/// Token dài hơn mức này coi như sai (không ai cấp token dài thế; khỏi tra gì).
const MAX_TOKEN_LEN: usize = 200;

/// Token đặt vào link cho khách `tenant`, từ phần thô `raw` (không chứa `.`): khách mặc định giữ
/// nguyên `raw`, khách khác thành `t<ID khách, 32 hex>.<raw>`.
pub fn link_token(tenant: &TenantContext, raw: &str) -> String {
    if tenant.id() == DEFAULT_TENANT {
        raw.to_owned()
    } else {
        format!("t{}.{raw}", tenant.id().as_uuid().simple())
    }
}

/// Tách token của link: (khách ghi trong token, nếu có; phần thô). Sai dạng: `None`.
pub fn split_link_token(token: &str) -> Option<(Option<TenantId>, &str)> {
    let Some((prefix, raw)) = token.split_once('.') else {
        return Some((None, token));
    };
    let hex = prefix.strip_prefix('t').filter(|hex| hex.len() == 32)?;
    let id = Uuid::try_parse(hex).ok()?;
    (!raw.is_empty() && !raw.contains('.')).then_some((Some(TenantId::from_uuid(id)), raw))
}

/// Khách và phần thô của token trong link. `request`: khách của request (`None`: không xác định
/// được, vd tên miền chính không header).
///
/// - Token có tiền tố: khách ghi trong token, tra qua `registry`. Khách không có, không phục vụ
///   được (tạm khoá, bảo trì) hay token sai dạng → `Ok(None)`, như token sai: không lộ khách nào
///   tồn tại.
/// - Token không tiền tố: khách của request, không có thì khách mặc định (token cấp trước bước 33
///   thuộc khách mặc định).
///
/// Lỗi khác của sổ database (vd database điều khiển không trả lời) trả về nguyên.
pub async fn resolve_link_token<'t>(
    registry: &DatabaseRegistry,
    request: Option<&TenantContext>,
    token: &'t str,
) -> Result<Option<(TenantContext, &'t str)>, Error> {
    let token = token.trim();
    if token.is_empty() || token.len() > MAX_TOKEN_LEN {
        return Ok(None);
    }
    let Some((id, raw)) = split_link_token(token) else {
        return Ok(None);
    };
    let tenant = match id {
        None => request.cloned().unwrap_or_else(|| registry.default_tenant()),
        Some(id) => match registry.tenant(id).await {
            Ok(tenant) if ensure_serving(&tenant).is_ok() => tenant,
            Ok(_) => return Ok(None),
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        },
    };
    Ok(Some((tenant, raw)))
}

/// Khách của request nếu xác định được (tên miền của khách, header `Autoxide-Tenant`,
/// credential); `None` khi không xác định được (`tenant_required`, vd tên miền chính không
/// header). Lỗi khác (khách tạm khoá, header trỏ khách khác credential…) vẫn trả về như
/// `TenantContext`. Cho endpoint nhận token trong link: khách lấy từ token bằng
/// [`resolve_link_token`].
#[derive(Clone, Debug, Default)]
pub struct OptionalTenant(pub Option<TenantContext>);

impl OptionalTenant {
    /// Khách của request, nếu xác định được.
    pub fn get(&self) -> Option<&TenantContext> {
        self.0.as_ref()
    }
}

impl<S: Send + Sync> FromRequestParts<S> for OptionalTenant {
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Credential hỏng trong `Authorization` không chặn endpoint đổi token: token tự nêu khách.
        match Box::pin(super::request::tenant_ignoring_bad_credential(parts)).await {
            Ok(tenant) => Ok(OptionalTenant(Some(tenant))),
            Err(error) if error.code() == "tenant_required" => Ok(OptionalTenant(None)),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::tenancy::{MAIN_PLACEMENT, TenantStatus};

    fn other() -> TenantContext {
        TenantContext::new(
            TenantId::from_uuid(Uuid::now_v7()),
            "other",
            "Other",
            TenantStatus::Active,
            MAIN_PLACEMENT,
            Database::detached(),
        )
    }

    #[test]
    fn tach_token_co_va_khong_tien_to_khach() {
        let id = Uuid::now_v7();
        let prefixed = format!("t{}.abc_-9", id.simple());
        assert_eq!(split_link_token(&prefixed), Some((Some(TenantId::from_uuid(id)), "abc_-9")));
        assert_eq!(split_link_token("abc_-9"), Some((None, "abc_-9")));
        for bad in ["x.abc", "t123.abc", &format!("t{}.", id.simple()), &format!("{prefixed}.x")] {
            assert_eq!(split_link_token(bad), None, "{bad}");
        }
    }

    #[test]
    fn khach_mac_dinh_giu_token_tho_khach_khac_co_tien_to() {
        let default = TenantContext::default_on(Database::detached());
        assert_eq!(link_token(&default, "raw"), "raw");
        let other = other();
        let token = link_token(&other, "raw");
        assert_eq!(token, format!("t{}.raw", other.id().as_uuid().simple()));
        assert_eq!(split_link_token(&token), Some((Some(other.id()), "raw")));
    }

    #[tokio::test]
    async fn tim_khach_tu_token() {
        // Sổ `single` không truy vấn gì: khách mặc định có, khách khác `tenant_not_found`.
        let registry = DatabaseRegistry::single(Database::detached());
        let other = other();
        // Không tiền tố: khách của request, không có thì khách mặc định.
        let (tenant, raw) = resolve_link_token(&registry, None, " raw ").await.unwrap().unwrap();
        assert_eq!((tenant.id(), raw), (DEFAULT_TENANT, "raw"));
        let (tenant, _) =
            resolve_link_token(&registry, Some(&other), "raw").await.unwrap().unwrap();
        assert_eq!(tenant, other);
        // Tiền tố của khách mặc định: tra được dù request là khách khác.
        let token = format!("t{}.raw", DEFAULT_TENANT.as_uuid().simple());
        let (tenant, raw) =
            resolve_link_token(&registry, Some(&other), &token).await.unwrap().unwrap();
        assert_eq!((tenant.id(), raw), (DEFAULT_TENANT, "raw"));
        // Khách không có, sai dạng, rỗng, quá dài: như token sai.
        let forged = link_token(&other, "raw");
        let long = "a".repeat(MAX_TOKEN_LEN + 1);
        for bad in [forged.as_str(), "x.raw", "", "  ", long.as_str()] {
            assert!(resolve_link_token(&registry, None, bad).await.unwrap().is_none(), "{bad}");
        }
    }
}
