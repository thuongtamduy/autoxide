//! Hãng vận chuyển giả, cho test (testkit, bước 45) và để thử trên máy dev
//! (`AUTOXIDE_SHIPPING_FAKE_CARRIER=true`). Không gọi mạng: mọi thứ nằm trong bộ nhớ, trạng thái
//! điều khiển bằng tay. Webhook ký HMAC-SHA256 trên thân thô bằng trường `webhook_secret` của
//! thông tin đăng nhập, header [`SIGNATURE_HEADER`] (hex chữ thường).
//!
//! Đây cũng là ví dụ cho người viết bộ nối thật (`docs/SHIPPING.md`).

use autoxide_kernel::time::{Date, Timestamp};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use autoxide_kernel::error::Error;
use autoxide_kernel::money::Money;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use super::errors;
use super::{
    CarrierAccount, CarrierCapabilities, CarrierQuoteRequest, CarrierRate, CarrierShipment,
    CarrierShipmentRequest, CarrierUpdate, CodStatement, Label, ShipmentStatus, ShippingCarrier,
};

/// Header mang chữ ký của webhook giả.
pub const SIGNATURE_HEADER: &str = "x-fake-signature";

/// Mã của hãng giả.
pub const CODE: &str = "fake";

type HmacSha256 = Hmac<Sha256>;

/// Chữ ký (hex) của `body` với khoá `secret`, như hãng giả gửi.
pub fn sign(secret: &str, body: &[u8]) -> String {
    let mut out = String::new();
    // `new_from_slice` chỉ lỗi với độ dài khoá không hợp lệ — HMAC nhận mọi độ dài.
    if let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) {
        mac.update(body);
        for byte in mac.finalize().into_bytes() {
            out.push_str(&format!("{byte:02x}"));
        }
    }
    out
}

/// Một dòng của thân webhook giả (thân là mảng JSON các dòng này).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FakeWebhookUpdate {
    pub event_id: String,
    pub tracking_number: String,
    /// Trạng thái chung (`in_transit`, `delivered`…) — hãng giả dùng luôn trạng thái chung.
    pub status: ShipmentStatus,
    pub occurred_at: Timestamp,
    #[serde(default)]
    pub cod_collected: Option<Money>,
}

/// Hãng giả. Dùng chung qua `Arc`: test giữ một bản để điều khiển, sổ hãng giữ bản kia.
#[derive(Default)]
pub struct FakeCarrier {
    /// Báo giá / tạo vận đơn trả "hãng không trả lời".
    unavailable: AtomicBool,
    /// Lần tạo vận đơn kế tiếp: hãng tạo xong nhưng trả lỗi (mất kết nối giữa chừng).
    lose_next_create_response: AtomicBool,
    /// Số lần `create_shipment` thật sự tạo đơn mới.
    created: AtomicU64,
    next_number: AtomicU64,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// Theo `client_reference`.
    shipments: HashMap<String, CarrierShipment>,
    /// Hành trình theo mã vận đơn.
    tracks: HashMap<String, Vec<CarrierUpdate>>,
    statement: Option<CodStatement>,
}

impl FakeCarrier {
    pub fn new() -> Self {
        FakeCarrier::default()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        // Khoá bị "nhiễm độc" chỉ khi một luồng panic lúc giữ khoá: dữ liệu vẫn dùng được.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Bật / tắt "hãng không trả lời".
    pub fn set_unavailable(&self, unavailable: bool) {
        self.unavailable.store(unavailable, Ordering::SeqCst);
    }

    /// Lần tạo vận đơn kế tiếp tạo xong nhưng trả lỗi `carrier_unavailable`.
    pub fn lose_next_create_response(&self) {
        self.lose_next_create_response.store(true, Ordering::SeqCst);
    }

    /// Số vận đơn đã tạo thật (gọi lại cùng mã không tính).
    pub fn created_count(&self) -> u64 {
        self.created.load(Ordering::SeqCst)
    }

    /// Thêm một bước hành trình cho vận đơn `tracking_number` (như hãng tự cập nhật).
    pub fn push_update(&self, update: CarrierUpdate) {
        let mut state = self.state();
        let status = update.status;
        let tracking = update.tracking_number.clone();
        state.tracks.entry(tracking.clone()).or_default().push(update);
        for shipment in state.shipments.values_mut() {
            if shipment.tracking_number == tracking {
                shipment.status = status;
            }
        }
    }

    /// Bảng kê COD `cod_statement` sẽ trả.
    pub fn set_statement(&self, statement: Option<CodStatement>) {
        self.state().statement = statement;
    }

    /// Phí giả: 15 000 đơn vị nhỏ nhất cho kg đầu, 5 000 mỗi kg (hay phần kg) sau.
    fn fee_for(&self, weight_grams: i64, like: &Money) -> Money {
        let extra_kg = (weight_grams.max(1) - 1) / 1000;
        Money::new(15_000 + 5_000 * extra_kg, like.currency())
    }

    fn check_available(&self) -> Result<(), Error> {
        if self.unavailable.load(Ordering::SeqCst) {
            Err(errors::carrier_unavailable())
        } else {
            Ok(())
        }
    }
}

#[async_trait::async_trait]
impl ShippingCarrier for FakeCarrier {
    fn code(&self) -> &'static str {
        CODE
    }

    fn capabilities(&self) -> CarrierCapabilities {
        CarrierCapabilities::all()
    }

    async fn check(&self, account: &CarrierAccount) -> Result<(), Error> {
        self.check_available()?;
        match account.credentials.get("token") {
            Some(token) if !token.is_empty() && token != "wrong" => Ok(()),
            _ => Err(errors::carrier_unauthorized()),
        }
    }

    async fn quote(
        &self,
        _account: &CarrierAccount,
        request: &CarrierQuoteRequest,
    ) -> Result<Vec<CarrierRate>, Error> {
        self.check_available()?;
        let weight: i64 = request.parcels.iter().map(|parcel| parcel.weight_grams).sum();
        let standard = self.fee_for(weight, &request.declared_value);
        let express = standard.checked_mul(2)?;
        let rates = vec![
            CarrierRate::new("standard", standard).days(2, 4),
            CarrierRate::new("express", express).days(1, 2),
        ];
        Ok(match &request.service_code {
            Some(code) => rates.into_iter().filter(|rate| &rate.service_code == code).collect(),
            None => rates,
        })
    }

    async fn create_shipment(
        &self,
        _account: &CarrierAccount,
        request: &CarrierShipmentRequest,
    ) -> Result<CarrierShipment, Error> {
        self.check_available()?;
        let shipment = {
            let mut state = self.state();
            if let Some(existing) = state.shipments.get(&request.client_reference) {
                existing.clone()
            } else {
                let number = self.next_number.fetch_add(1, Ordering::SeqCst) + 1;
                let tracking = format!("FAKE{number:08}");
                let parcels = (1..=request.parcels.len())
                    .map(|position| format!("{tracking}-{position}"))
                    .collect();
                let weight: i64 = request.parcels.iter().map(|parcel| parcel.weight_grams).sum();
                let fee = self.fee_for(weight, &request.declared_value);
                let shipment = CarrierShipment::new(request.client_reference.clone(), tracking)
                    .parcels(parcels)
                    .fee(Some(fee));
                state.shipments.insert(request.client_reference.clone(), shipment.clone());
                self.created.fetch_add(1, Ordering::SeqCst);
                shipment
            }
        };
        if self.lose_next_create_response.swap(false, Ordering::SeqCst) {
            return Err(errors::carrier_unavailable());
        }
        Ok(shipment)
    }

    async fn find_by_reference(
        &self,
        _account: &CarrierAccount,
        client_reference: &str,
    ) -> Result<Option<CarrierShipment>, Error> {
        self.check_available()?;
        Ok(self.state().shipments.get(client_reference).cloned())
    }

    async fn cancel(&self, _account: &CarrierAccount, tracking_number: &str) -> Result<(), Error> {
        self.check_available()?;
        let mut state = self.state();
        let Some(shipment) = state
            .shipments
            .values_mut()
            .find(|shipment| shipment.tracking_number == tracking_number)
        else {
            return Err(errors::carrier_error("unknown tracking number"));
        };
        match shipment.status {
            ShipmentStatus::Pending | ShipmentStatus::Booking | ShipmentStatus::ReadyToShip => {
                shipment.status = ShipmentStatus::Cancelled;
                Ok(())
            }
            ShipmentStatus::Cancelled => Ok(()),
            _ => Err(errors::cancel_too_late()),
        }
    }

    async fn label(
        &self,
        _account: &CarrierAccount,
        tracking_number: &str,
    ) -> Result<Label, Error> {
        self.check_available()?;
        Ok(Label::new("application/pdf", format!("%PDF-1.4 fake label {tracking_number}").into()))
    }

    async fn track(
        &self,
        _account: &CarrierAccount,
        tracking_number: &str,
    ) -> Result<Vec<CarrierUpdate>, Error> {
        self.check_available()?;
        Ok(self.state().tracks.get(tracking_number).cloned().unwrap_or_default())
    }

    fn parse_webhook(
        &self,
        account: &CarrierAccount,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> Result<Vec<CarrierUpdate>, Error> {
        let secret = account.credentials.get("webhook_secret").unwrap_or_default();
        let claimed = headers
            .get(SIGNATURE_HEADER)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let expected = sign(secret, body);
        if secret.is_empty() || !constant_time_eq(expected.as_bytes(), claimed.as_bytes()) {
            return Err(errors::webhook_signature());
        }
        let lines: Vec<FakeWebhookUpdate> = serde_json::from_slice(body)
            .map_err(|_| errors::carrier_error("the webhook body is not valid JSON"))?;
        Ok(lines
            .into_iter()
            .map(|line| {
                CarrierUpdate::new(
                    line.event_id,
                    line.tracking_number,
                    line.status,
                    line.occurred_at,
                )
                .cod_collected(line.cod_collected)
            })
            .collect())
    }

    async fn cod_statement(
        &self,
        _account: &CarrierAccount,
        _from: Date,
        _to: Date,
    ) -> Result<Option<CodStatement>, Error> {
        self.check_available()?;
        Ok(self.state().statement.clone())
    }
}

/// So hai chuỗi byte trong thời gian không phụ thuộc chỗ khác nhau đầu tiên.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}
