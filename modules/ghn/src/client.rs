//! Gọi API của GHN và đọc câu trả lời theo tài liệu: mọi câu trả lời là
//! `{"code": <int>, "message": "...", "data": ..., "code_message": "..."}`; `code` 200 là xong.
//! Header `Token` + `ShopId`. Không bao giờ ghi token, địa chỉ có tham số hay thân vào log
//! (`autoxide_kernel::provider::send` lo phần log và số đo).

use std::time::Duration;

use autoxide_kernel::error::Error;
use autoxide_kernel::provider::{ProviderReply, Target, http_client, send};
use autoxide_shipping::api::errors as carrier;
use serde_json::Value;

use crate::account::GhnAccount;
use crate::api::PROVIDER;
use crate::config::GhnConfig;
use crate::errors;

/// Hạn giờ của một lời gọi. `shipping` còn đặt hạn riêng cho cả thao tác
/// (`AUTOXIDE_SHIPPING_CARRIER_TIMEOUT_SECONDS`); đây là chốt chặn của riêng kết nối.
const TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct Client {
    http: reqwest::Client,
    config: GhnConfig,
}

/// GHN báo "không có đơn này" theo nhiều dạng tuỳ endpoint (tài liệu: `ORDER_NOT_FOUND`, câu
/// "Đơn hàng không tồn tại", "Không tìm thấy thông tin đơn hàng", hoặc HTTP 404).
fn is_order_not_found(status: u16, code_message: &str, message: &str) -> bool {
    status == 404
        || code_message == "ORDER_NOT_FOUND"
        || message.contains("Đơn hàng không tồn tại")
        || message.contains("Không tìm thấy thông tin đơn hàng")
}

/// Phần có nghĩa của `message`: GHN ghép tiền tố nội bộ rồi ` - ` rồi câu cho người đọc.
fn readable(body: &Value) -> String {
    let text = |name: &str| body.get(name).and_then(Value::as_str).unwrap_or("").trim();
    let message = text("message");
    let message = message.rsplit(" - ").next().unwrap_or(message);
    let chosen = if message.is_empty() { text("code_message_value") } else { message };
    let chosen = if chosen.is_empty() { "no reason given" } else { chosen };
    chosen.chars().take(300).collect()
}

/// Đổi một câu trả lời lỗi của GHN sang lỗi có `code` ổn định.
fn refused(status: u16, body: &Value) -> Error {
    let text = |name: &str| body.get(name).and_then(Value::as_str).unwrap_or("");
    let (code_message, message) = (text("code_message"), text("message"));
    if status == 401 || code_message == "CLIENT_NOT_OWNER_OF_SHOP" {
        return carrier::carrier_unauthorized();
    }
    if status == 403 {
        return errors::permission_missing();
    }
    if status >= 500 || status == 408 || status == 429 || code_message == "SERVER_ERROR_COMMON" {
        return carrier::carrier_unavailable();
    }
    if is_order_not_found(status, code_message, message) {
        return errors::order_not_found();
    }
    match code_message {
        "PHONE_INVALID" => errors::phone_invalid(),
        "WARD_IS_INVALID" => errors::address_not_recognized(),
        "ROUTE_NOT_FOUND_SERVICE" | "SERVICE_NOT_FOUND_CONFIG_FEE" => errors::route_not_supported(),
        // Tài liệu cho thấy lỗi tuyến có khi về không kèm `code_message`.
        _ if message.contains("route not found service") => errors::route_not_supported(),
        _ => carrier::carrier_error(readable(body)),
    }
}

/// `data` của một câu trả lời thành công; mọi dạng khác thành lỗi.
fn data_of(reply: &ProviderReply) -> Result<Value, Error> {
    let Some(mut body) = reply.json() else {
        // Tài liệu: vài endpoint trả 200 với thân rỗng khi không tìm thấy cửa hàng.
        return Err(if reply.status >= 500 {
            carrier::carrier_unavailable()
        } else if reply.status == 401 {
            carrier::carrier_unauthorized()
        } else {
            carrier::carrier_error("the answer is empty or not JSON")
        });
    };
    let code = body.get("code").and_then(Value::as_i64).unwrap_or(i64::from(reply.status));
    if reply.is_success() && code == 200 {
        return Ok(body.get_mut("data").map(Value::take).unwrap_or(Value::Null));
    }
    // GHN có lúc trả HTTP 200 với `code` lỗi trong thân: lấy mã trong thân.
    let status = if reply.is_success() { u16::try_from(code).unwrap_or(400) } else { reply.status };
    Err(refused(status, &body))
}

impl Client {
    pub(crate) fn new(config: GhnConfig) -> Result<Self, Error> {
        // `Target::Fixed`: địa chỉ GHN do người vận hành đặt, không phải khách chọn.
        Ok(Client { http: http_client(TIMEOUT, Target::Fixed)?, config })
    }

    fn base(&self, account: &GhnAccount) -> &str {
        if account.sandbox { &self.config.sandbox_api_url } else { &self.config.api_url }
    }

    fn url(
        &self,
        account: &GhnAccount,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<reqwest::Url, Error> {
        let mut url = reqwest::Url::parse(&format!("{}{path}", self.base(account)))
            .map_err(|error| Error::internal(format!("GHN address: {error}")))?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        Ok(url)
    }

    fn signed(
        &self,
        request: reqwest::RequestBuilder,
        account: &GhnAccount,
    ) -> reqwest::RequestBuilder {
        request.header("Token", &account.token).header("ShopId", account.shop_id.to_string())
    }

    /// `POST {path}` với thân JSON; trả `data`.
    pub(crate) async fn post(
        &self,
        account: &GhnAccount,
        path: &str,
        purpose: &'static str,
        body: &Value,
    ) -> Result<Value, Error> {
        let request = self
            .signed(self.http.post(self.url(account, path, &[])?), account)
            .header("Content-Type", "application/json")
            .body(serde_json::to_vec(body).map_err(Error::internal)?);
        let reply =
            send(request, PROVIDER, purpose).await.map_err(|_| carrier::carrier_unavailable())?;
        data_of(&reply)
    }

    /// `GET {path}?{query}`; trả `data`.
    pub(crate) async fn get(
        &self,
        account: &GhnAccount,
        path: &str,
        purpose: &'static str,
        query: &[(&str, &str)],
    ) -> Result<Value, Error> {
        let request = self.signed(self.http.get(self.url(account, path, query)?), account);
        let reply =
            send(request, PROVIDER, purpose).await.map_err(|_| carrier::carrier_unavailable())?;
        data_of(&reply)
    }

    /// Trang in vận đơn: thân thô (không phải JSON). Token in nằm trong địa chỉ, không cần header.
    pub(crate) async fn print_page(
        &self,
        account: &GhnAccount,
        print_token: &str,
    ) -> Result<Vec<u8>, Error> {
        let path = format!("/a5/public-api/{}", account.label_page);
        let request = self.http.get(self.url(account, &path, &[("token", print_token)])?);
        let reply = send(request, PROVIDER, "print_label")
            .await
            .map_err(|_| carrier::carrier_unavailable())?;
        if !reply.is_success() {
            return Err(if reply.status >= 500 {
                carrier::carrier_unavailable()
            } else {
                carrier::carrier_error("the label page is not available")
            });
        }
        Ok(reply.body.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn loi_cua_ghn_thanh_ma_on_dinh() {
        let code = |status, body: Value| refused(status, &body).code().to_owned();
        assert_eq!(
            code(401, json!({"message": "Authorization header is required!"})),
            "shipping.carrier_unauthorized"
        );
        assert_eq!(
            code(400, json!({"code_message": "CLIENT_NOT_OWNER_OF_SHOP"})),
            "shipping.carrier_unauthorized"
        );
        assert_eq!(code(403, json!({})), "ghn.permission_missing");
        assert_eq!(
            code(500, json!({"code_message": "SERVER_ERROR_COMMON"})),
            "shipping.carrier_unavailable"
        );
        assert_eq!(code(429, json!({})), "shipping.carrier_unavailable");
        assert_eq!(code(400, json!({"code_message": "PHONE_INVALID"})), "ghn.phone_invalid");
        assert_eq!(
            code(400, json!({"code_message": "WARD_IS_INVALID"})),
            "ghn.address_not_recognized"
        );
        assert_eq!(
            code(400, json!({"code_message": "ROUTE_NOT_FOUND_SERVICE"})),
            "ghn.route_not_supported"
        );
        assert_eq!(
            code(
                400,
                json!({"message": "calculate fee error: calculate service fee error: route not found service"})
            ),
            "ghn.route_not_supported"
        );
        assert_eq!(
            code(
                400,
                json!({"message": "Lỗi gọi API: corev2_tenant_order_detail - Đơn hàng không tồn tại"})
            ),
            "ghn.order_not_found"
        );
        assert_eq!(
            code(404, json!({"message": "Không tìm thấy thông tin đơn hàng"})),
            "ghn.order_not_found"
        );
        let other = refused(
            400,
            &json!({"message": "Sai thông tin Required Note", "code_message": "USER_ERR_COMMON"}),
        );
        assert_eq!(other.code(), "shipping.carrier_error");
    }

    #[test]
    fn cau_cho_nguoi_doc_bo_tien_to_noi_bo() {
        assert_eq!(
            readable(&json!({"message": "Lỗi gọi API: x - Sai số điện thoại"})),
            "Sai số điện thoại"
        );
        assert_eq!(
            readable(&json!({"code_message_value": "Sai thông tin đầu vào."})),
            "Sai thông tin đầu vào."
        );
        assert_eq!(readable(&json!({})), "no reason given");
    }
}
