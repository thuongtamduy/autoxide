//! `ShippingCarrier` cho GHN. Mỗi hàm gọi mạng một hay hai lần; `shipping` không bao giờ gọi chúng
//! trong transaction ghi. Đơn vị của GHN: VND, gram, cm (Autoxide: đơn vị nhỏ nhất, gram, mm).

use autoxide_kernel::error::Error;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::time::Timestamp;
use autoxide_shipping::api::{
    Address, CarrierAccount, CarrierCapabilities, CarrierParcel, CarrierQuoteRequest, CarrierRate,
    CarrierShipment, CarrierShipmentRequest, CarrierUpdate, Direction, Label, ShipmentStatus,
    ShippingCarrier, errors as carrier,
};
use serde_json::{Map, Value, json};

use crate::account::{self, GhnAccount};
use crate::api::PROVIDER;
use crate::client::Client;
use crate::{errors, status, webhook};

const SHOPS: &str = "/shiip/public-api/v2/shop/all";
const PREVIEW: &str = "/shiip/public-api/v2/shipping-order/preview";
const CREATE: &str = "/shiip/public-api/v2/shipping-order/create";
const DETAIL: &str = "/shiip/public-api/v2/shipping-order/detail";
const DETAIL_BY_CLIENT_CODE: &str = "/shiip/public-api/v2/shipping-order/detail-by-client-code";
const CANCEL: &str = "/shiip/public-api/v2/switch-status/cancel";
const PRINT_TOKEN: &str = "/shiip/public-api/v2/a5/gen-token";

/// Dịch vụ của GHN (`service_type_id`): hàng nhẹ dưới 20 kg / hàng nặng hoặc nhiều kiện.
const LIGHT: i64 = 2;
const HEAVY: i64 = 5;
/// Ngưỡng cân nặng giữa hai dịch vụ (gram).
const HEAVY_FROM_GRAMS: i64 = 20_000;
/// GHN nhận giá trị khai báo (bảo hiểm) tối đa 5 000 000 đ; cao hơn thì khai ở mức trần.
const INSURANCE_MAX: i64 = 5_000_000;
/// GHN buộc có kích thước; kiện không khai kích thước thì gửi 10 cm mỗi cạnh.
const DEFAULT_SIDE_CM: i64 = 10;
/// `check` đọc danh sách cửa hàng theo trang; dừng sau từng này trang.
const SHOP_PAGES: usize = 20;
const SHOP_PAGE_SIZE: &str = "200";

pub(crate) struct GhnCarrier {
    client: Client,
}

/// Số tiền VND (đơn vị nhỏ nhất của VND là đồng); tiền tệ khác → `ghn.currency_unsupported`.
fn vnd(money: Money) -> Result<i64, Error> {
    if money.currency() == Currency::VND {
        Ok(money.minor())
    } else {
        Err(errors::currency_unsupported(money.currency().code()))
    }
}

/// mm → cm, làm tròn lên (khai nhỏ hơn thật thì GHN tính lại phí).
fn cm(mm: Option<i64>) -> i64 {
    mm.filter(|mm| *mm > 0).map_or(DEFAULT_SIDE_CM, |mm| (mm + 9) / 10)
}

fn service_of(code: Option<&str>, parcels: &[CarrierParcel]) -> Result<i64, Error> {
    match code.map(str::trim) {
        Some("2") => Ok(LIGHT),
        Some("5") => Ok(HEAVY),
        Some(other) if !other.is_empty() => Err(errors::service_unknown(other)),
        _ => {
            let total: i64 = parcels.iter().map(|parcel| parcel.weight_grams).sum();
            Ok(if parcels.len() > 1 || total >= HEAVY_FROM_GRAMS { HEAVY } else { LIGHT })
        }
    }
}

/// Địa chỉ theo mô hình hai cấp (tỉnh + phường / xã, sau 01/07/2025): GHN nhận theo TÊN, kèm cờ
/// `is_new_<prefix>_address`. Module `address` của Autoxide cũng dùng mô hình hai cấp, nên không
/// có bảng ánh xạ mã nào ở đây.
fn put_address(
    body: &mut Map<String, Value>,
    prefix: &str,
    address: &Address,
) -> Result<(), Error> {
    if !address.country.eq_ignore_ascii_case("VN") {
        return Err(errors::country_unsupported(&address.country));
    }
    let full = format!("{}, {}, {}", address.line, address.commune_name, address.province_name);
    body.insert(format!("{prefix}_address"), json!(full));
    body.insert(format!("{prefix}_ward_name"), json!(address.commune_name));
    body.insert(format!("{prefix}_province_name"), json!(address.province_name));
    body.insert(format!("is_new_{prefix}_address"), json!(true));
    Ok(())
}

/// Phần chung của thân `preview` và `create`: dịch vụ, cân nặng, kích thước, giá trị.
fn put_parcels(
    body: &mut Map<String, Value>,
    account: &GhnAccount,
    service: i64,
    parcels: &[CarrierParcel],
    declared: Money,
    cod: Option<Money>,
) -> Result<(), Error> {
    let weight: i64 = parcels.iter().map(|parcel| parcel.weight_grams.max(1)).sum();
    let side = |pick: fn(&CarrierParcel) -> Option<i64>| {
        parcels.iter().map(|parcel| cm(pick(parcel))).max().unwrap_or(DEFAULT_SIDE_CM)
    };
    body.insert("service_type_id".to_owned(), json!(service));
    body.insert("payment_type_id".to_owned(), json!(account.payment_type_id));
    body.insert("required_note".to_owned(), json!(account.required_note));
    body.insert("content".to_owned(), json!(account.content));
    body.insert("weight".to_owned(), json!(weight.max(1)));
    body.insert("length".to_owned(), json!(side(|parcel| parcel.length_mm)));
    body.insert("width".to_owned(), json!(side(|parcel| parcel.width_mm)));
    body.insert("height".to_owned(), json!(side(|parcel| parcel.height_mm)));
    body.insert("insurance_value".to_owned(), json!(vnd(declared)?.clamp(0, INSURANCE_MAX)));
    body.insert("cod_amount".to_owned(), json!(cod.map(vnd).transpose()?.unwrap_or(0).max(0)));
    if service == HEAVY {
        // Hàng nặng / nhiều kiện: GHN tính phí theo từng kiện trong `items`.
        let items: Vec<Value> = parcels
            .iter()
            .enumerate()
            .map(|(index, parcel)| {
                json!({
                    "name": format!("{} {}", account.content, index + 1),
                    "quantity": 1,
                    "weight": parcel.weight_grams.max(1),
                    "length": cm(parcel.length_mm),
                    "width": cm(parcel.width_mm),
                    "height": cm(parcel.height_mm),
                })
            })
            .collect();
        body.insert("items".to_owned(), Value::Array(items));
    }
    Ok(())
}

fn text<'a>(data: &'a Value, name: &str) -> &'a str {
    data.get(name).and_then(Value::as_str).unwrap_or("")
}

fn fee_of(data: &Value, name: &str) -> Option<Money> {
    data.get(name).and_then(Value::as_i64).map(|fee| Money::new(fee, Currency::VND))
}

/// Số ngày từ bây giờ tới `at` (làm tròn lên, ít nhất 1).
fn days_until(at: Timestamp) -> u16 {
    let seconds = at.duration_since(Timestamp::now()).as_secs().max(0);
    u16::try_from((seconds + 86_399) / 86_400).unwrap_or(u16::MAX).max(1)
}

/// Vận đơn từ `data` của `detail` / `detail-by-client-code`.
fn shipment_of(data: &Value) -> Option<CarrierShipment> {
    let code = text(data, "order_code");
    if code.is_empty() {
        return None;
    }
    let mut shipment = CarrierShipment::new(text(data, "client_order_code"), code);
    shipment.status = status::map(text(data, "status")).unwrap_or(ShipmentStatus::ReadyToShip);
    Some(shipment)
}

impl GhnCarrier {
    pub(crate) fn new(client: Client) -> Self {
        GhnCarrier { client }
    }

    async fn detail(&self, account: &GhnAccount, order_code: &str) -> Result<Value, Error> {
        self.client.get(account, DETAIL, "order_detail", &[("order_code", order_code)]).await
    }
}

#[async_trait::async_trait]
impl ShippingCarrier for GhnCarrier {
    fn code(&self) -> &'static str {
        PROVIDER
    }

    fn capabilities(&self) -> CarrierCapabilities {
        let mut capabilities = CarrierCapabilities::all();
        // GHN không có API bảng kê COD trong tài liệu công khai: đối soát bằng nhập file.
        capabilities.statement_api = false;
        capabilities
    }

    /// Token đọc được danh sách cửa hàng và `shop_id` là một cửa hàng đang hoạt động trong đó.
    async fn check(&self, account: &CarrierAccount) -> Result<(), Error> {
        let account = account::read(account)?;
        let mut offset = "0".to_owned();
        for _ in 0..SHOP_PAGES {
            let query = [("offset", offset.as_str()), ("limit", SHOP_PAGE_SIZE)];
            let data = self.client.get(&account, SHOPS, "list_shops", &query).await?;
            let shops = data.get("shops").and_then(Value::as_array).cloned().unwrap_or_default();
            if let Some(shop) = shops
                .iter()
                .find(|shop| shop.get("_id").and_then(Value::as_i64) == Some(account.shop_id))
            {
                return match shop.get("status").and_then(Value::as_i64) {
                    Some(1) | None => Ok(()),
                    Some(_) => Err(errors::shop_not_found(account.shop_id)),
                };
            }
            let next = data.get("last_offset").and_then(Value::as_i64).map(|next| next.to_string());
            match next {
                Some(next) if !shops.is_empty() && next != offset => offset = next,
                _ => break,
            }
        }
        Err(errors::shop_not_found(account.shop_id))
    }

    /// Báo giá bằng `preview` — chạy thử của `create`, là endpoint tính phí duy nhất nhận địa chỉ
    /// hai cấp theo tên (các endpoint tính phí riêng chỉ nhận mã quận của mô hình cũ).
    async fn quote(
        &self,
        account: &CarrierAccount,
        request: &CarrierQuoteRequest,
    ) -> Result<Vec<CarrierRate>, Error> {
        let account = account::read(account)?;
        let service = service_of(request.service_code.as_deref(), &request.parcels)?;
        let mut body = Map::new();
        // `preview` buộc có người nhận như `create`; lúc báo giá chưa có nên gửi giá trị giữ chỗ.
        body.insert("to_name".to_owned(), json!("Quote"));
        body.insert("to_phone".to_owned(), json!("0900000000"));
        put_address(&mut body, "to", &request.destination)?;
        if let Some(origin) = &request.origin {
            put_address(&mut body, "from", origin)?;
        }
        put_parcels(
            &mut body,
            &account,
            service,
            &request.parcels,
            request.declared_value,
            request.cod_amount,
        )?;
        let data =
            self.client.post(&account, PREVIEW, "preview_order", &Value::Object(body)).await?;
        let Some(fee) = fee_of(&data, "total_fee") else {
            return Err(carrier::carrier_error("the quote has no total_fee"));
        };
        let mut rate = CarrierRate::new(service.to_string(), fee);
        if let Ok(expected) = Timestamp::parse_rfc3339(text(&data, "expected_delivery_time")) {
            let days = days_until(expected);
            rate = rate.days(days, days);
        }
        Ok(vec![rate])
    }

    /// Tạo đơn. `client_order_code` = ID vận đơn của Autoxide: GHN trả lại `order_code` cũ khi nhận
    /// lại cùng mã, nên gọi lại không tạo đơn thứ hai.
    async fn create_shipment(
        &self,
        account: &CarrierAccount,
        request: &CarrierShipmentRequest,
    ) -> Result<CarrierShipment, Error> {
        let account = account::read(account)?;
        if request.direction != Direction::Outbound {
            return Err(errors::direction_unsupported());
        }
        let service = service_of(request.service_code.as_deref(), &request.parcels)?;
        let mut body = Map::new();
        body.insert("client_order_code".to_owned(), json!(request.client_reference));
        body.insert("to_name".to_owned(), json!(request.recipient_name));
        body.insert("to_phone".to_owned(), json!(request.recipient_phone));
        put_address(&mut body, "to", &request.destination)?;
        // Không có nơi lấy hàng: GHN dùng địa chỉ của cửa hàng (ShopId).
        if let Some(origin) = &request.origin {
            put_address(&mut body, "from", origin)?;
        }
        put_parcels(
            &mut body,
            &account,
            service,
            &request.parcels,
            request.declared_value,
            request.cod_amount,
        )?;
        if !request.note.is_empty() {
            // GHN: tối đa 5000 ký tự.
            body.insert(
                "note".to_owned(),
                json!(request.note.chars().take(5000).collect::<String>()),
            );
        }
        let data = self.client.post(&account, CREATE, "create_order", &Value::Object(body)).await?;
        let code = text(&data, "order_code");
        if code.is_empty() {
            return Err(carrier::carrier_error("the order has no order_code"));
        }
        Ok(CarrierShipment::new(request.client_reference.clone(), code)
            .fee(fee_of(&data, "total_fee")))
    }

    async fn find_by_reference(
        &self,
        account: &CarrierAccount,
        client_reference: &str,
    ) -> Result<Option<CarrierShipment>, Error> {
        let account = account::read(account)?;
        let query = [("client_order_code", client_reference)];
        match self.client.get(&account, DETAIL_BY_CLIENT_CODE, "find_order", &query).await {
            Ok(data) => Ok(shipment_of(&data).map(|mut shipment| {
                shipment.client_reference = client_reference.to_owned();
                shipment
            })),
            Err(error) if error.code() == "ghn.order_not_found" => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn cancel(&self, account: &CarrierAccount, tracking_number: &str) -> Result<(), Error> {
        let account = account::read(account)?;
        let body = json!({"order_codes": [tracking_number]});
        let data = self.client.post(&account, CANCEL, "cancel_order", &body).await?;
        // Một dòng cho mỗi mã; `result: false` = trạng thái hiện tại không huỷ được nữa.
        let cancelled = data
            .as_array()
            .and_then(|lines| lines.iter().find(|line| text(line, "order_code") == tracking_number))
            .and_then(|line| line.get("result").and_then(Value::as_bool));
        match cancelled {
            Some(true) => Ok(()),
            Some(false) => Err(carrier::cancel_too_late()),
            None => Err(carrier::carrier_error("the cancellation has no result for this order")),
        }
    }

    /// Vận đơn để in: xin token in (sống khoảng 30 phút) rồi tải trang in của GHN theo khổ giấy của
    /// tài khoản. Tài liệu không nêu kiểu nội dung của trang; nhận ra PDF theo chữ ký đầu file, còn
    /// lại coi là HTML.
    async fn label(&self, account: &CarrierAccount, tracking_number: &str) -> Result<Label, Error> {
        let account = account::read(account)?;
        let body = json!({"order_codes": [tracking_number]});
        let data = self.client.post(&account, PRINT_TOKEN, "print_token", &body).await?;
        let token = text(&data, "token");
        if token.is_empty() {
            return Err(carrier::carrier_error("no print token was issued"));
        }
        let bytes = self.client.print_page(&account, token).await?;
        let content_type =
            if bytes.starts_with(b"%PDF") { "application/pdf" } else { "text/html; charset=utf-8" };
        Ok(Label::new(content_type, bytes))
    }

    /// Tài liệu của `detail` chỉ mô tả trạng thái hiện tại (không có lịch sử): trả một cập nhật.
    /// Hành trình đầy đủ đến từ webhook.
    async fn track(
        &self,
        account: &CarrierAccount,
        tracking_number: &str,
    ) -> Result<Vec<CarrierUpdate>, Error> {
        let account = account::read(account)?;
        let data = self.detail(&account, tracking_number).await?;
        let raw = text(&data, "status");
        let Some(mapped) = status::map(raw) else { return Ok(Vec::new()) };
        let updated = text(&data, "updated_date");
        let at = Timestamp::parse_rfc3339(updated).unwrap_or_else(|_| Timestamp::now());
        let mut update = CarrierUpdate::new(
            format!("{tracking_number}:detail:{raw}:{updated}"),
            tracking_number,
            mapped,
            at,
        )
        .carrier_status(raw);
        if data.get("is_cod_collected").and_then(Value::as_bool) == Some(true) {
            update = update.cod_collected(fee_of(&data, "cod_amount"));
        }
        Ok(vec![update])
    }

    fn parse_webhook(
        &self,
        account: &CarrierAccount,
        _headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> Result<Vec<CarrierUpdate>, Error> {
        webhook::parse(account, body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> GhnAccount {
        let credentials = autoxide_shipping::api::CarrierCredentials::new(
            [("token".to_owned(), "t".to_owned())].into(),
        );
        let account = CarrierAccount::new(
            Default::default(),
            PROVIDER,
            false,
            json!({"shop_id": 1, "required_note": "CHOTHUHANG", "payment_type_id": 2}),
            credentials,
        );
        account::read(&account).unwrap()
    }

    #[test]
    fn dich_vu_theo_can_nang_va_so_kien() {
        let light = [CarrierParcel::new(19_999)];
        let heavy = [CarrierParcel::new(20_000)];
        let two = [CarrierParcel::new(100), CarrierParcel::new(100)];
        assert_eq!(service_of(None, &light).unwrap(), LIGHT);
        assert_eq!(service_of(None, &heavy).unwrap(), HEAVY);
        assert_eq!(service_of(None, &two).unwrap(), HEAVY);
        assert_eq!(service_of(Some("5"), &light).unwrap(), HEAVY);
        assert_eq!(service_of(Some(" 2 "), &heavy).unwrap(), LIGHT);
        assert_eq!(service_of(Some("express"), &light).unwrap_err().code(), "ghn.service_unknown");
    }

    #[test]
    fn nhieu_kien_gui_tung_kien_trong_items() {
        let parcels = [
            CarrierParcel::new(650),
            CarrierParcel::new(550).dimensions(Some(301), Some(200), Some(95)),
        ];
        let mut body = Map::new();
        let vnd = |amount| Money::new(amount, Currency::VND);
        put_parcels(&mut body, &account(), HEAVY, &parcels, vnd(100_000), None).unwrap();
        assert_eq!(body["weight"], 1200);
        // mm → cm làm tròn lên; cạnh lớn nhất trong các kiện; kiện không khai thì 10 cm.
        assert_eq!(
            (&body["length"], &body["width"], &body["height"]),
            (&json!(31), &json!(20), &json!(10))
        );
        assert_eq!(body["required_note"], "CHOTHUHANG");
        assert_eq!(body["payment_type_id"], 2);
        assert_eq!(body["cod_amount"], 0);
        let items = body["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[1],
            json!({"name": "Hàng hóa 2", "quantity": 1, "weight": 550, "length": 31, "width": 20, "height": 10})
        );
    }

    #[test]
    fn chi_nhan_vnd_va_dia_chi_viet_nam() {
        let usd = Money::new(100, Currency::USD);
        let mut body = Map::new();
        let error = put_parcels(&mut body, &account(), LIGHT, &[CarrierParcel::new(1)], usd, None);
        assert_eq!(error.unwrap_err().code(), "ghn.currency_unsupported");
        let abroad: Address = serde_json::from_value(json!({"country": "US", "province_code": "x",
            "province_name": "x", "commune_code": "x", "commune_name": "x", "line": "x"}))
        .unwrap();
        assert_eq!(
            put_address(&mut body, "to", &abroad).unwrap_err().code(),
            "ghn.country_unsupported"
        );
    }
}
