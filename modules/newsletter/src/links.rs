//! Token trong link của thư (huỷ đăng ký, trang lựa chọn, xác nhận) mang khách của nó (bước 33).
//!
//! Người nhận bấm link từ hộp thư: request có thể tới tên miền chính, không header
//! `Autoxide-Tenant` — không biết khách nào. Nên token của khách khác khách mặc định ghi ID khách
//! ngay trong chuỗi: `t<UUID 32 chữ hex>.<phần thô>`. Request được định tuyến tới đúng database
//! TRƯỚC khi tra bản băm (đặc tả 33, "token dùng một lần cấp sau bước này mang ID khách"). Token của
//! khách mặc định giữ nguyên dạng cũ (chỉ phần thô): cài đặt một khách và thư đã gửi không đổi.
//!
//! Database chỉ lưu bản băm của **phần thô**, tra trong khách đã định tuyến. Ghép ID của khách
//! khác với phần thô của khách này thì tra ở khách kia, không thấy — như token sai. Phần thô là
//! base64url (không có dấu `.`), nên có dấu `.` nghĩa là có tiền tố khách.

use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, TenantContext, TenantId, ensure_serving};
use uuid::Uuid;

use crate::service::NewsletterService;

/// Token đặt vào link cho khách `tenant`, từ phần thô `raw`.
pub(crate) fn link_token(tenant: &TenantContext, raw: &str) -> String {
    if tenant.id() == DEFAULT_TENANT {
        raw.to_owned()
    } else {
        format!("t{}.{raw}", tenant.id().as_uuid().simple())
    }
}

/// Tách token của link: (khách ghi trong token, nếu có; phần thô). Sai dạng: `None`.
fn split(token: &str) -> Option<(Option<TenantId>, &str)> {
    let Some((prefix, raw)) = token.split_once('.') else {
        return Some((None, token));
    };
    let hex = prefix.strip_prefix('t').filter(|hex| hex.len() == 32)?;
    let id = Uuid::try_parse(hex).ok()?;
    (!raw.is_empty() && !raw.contains('.')).then_some((Some(TenantId::from_uuid(id)), raw))
}

impl NewsletterService {
    /// Khách và phần thô của token trong link. `request`: khách của request (`None`: không xác
    /// định được, vd tên miền chính không header). Token có tiền tố: khách trong token (không có,
    /// không phục vụ được hay sai dạng → `None`, như token sai — không lộ khách nào tồn tại).
    /// Token không tiền tố: khách của request, không có thì khách mặc định (token cấp trước bước
    /// 33 thuộc khách mặc định).
    pub(crate) async fn token_tenant<'t>(
        &self,
        request: Option<&TenantContext>,
        token: &'t str,
    ) -> Result<Option<(TenantContext, &'t str)>, Error> {
        let token = token.trim();
        if token.is_empty() || token.len() > 200 {
            return Ok(None);
        }
        let Some((id, raw)) = split(token) else {
            return Ok(None);
        };
        let tenant = match id {
            None => request.cloned().unwrap_or_else(|| self.databases.default_tenant()),
            Some(id) => match self.databases.tenant(id).await {
                Ok(tenant) if ensure_serving(&tenant).is_ok() => tenant,
                Ok(_) => return Ok(None),
                Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error),
            },
        };
        Ok(Some((tenant, raw)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tach_token_co_va_khong_tien_to_khach() {
        let id = Uuid::now_v7();
        let prefixed = format!("t{}.abc_-9", id.simple());
        assert_eq!(split(&prefixed), Some((Some(TenantId::from_uuid(id)), "abc_-9")));
        assert_eq!(split("abc_-9"), Some((None, "abc_-9")));
        for bad in ["x.abc", "t123.abc", &format!("t{}.", id.simple()), &format!("{prefixed}.x")] {
            assert_eq!(split(bad), None, "{bad}");
        }
    }
}
