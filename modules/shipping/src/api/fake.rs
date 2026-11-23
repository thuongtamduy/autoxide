//! A fake carrier, for tests (testkit, step 45) and for trying things on a dev machine
//! (`AUTOXIDE_SHIPPING_FAKE_CARRIER=true`). Makes no network calls: everything lives in memory and the status
//! is driven by hand. Webhooks are signed with HMAC-SHA256 over the raw body using the `webhook_secret` field
//! of the credentials, in the [`SIGNATURE_HEADER`] header (lowercase hex).
//!
//! This is also an example for people writing a real connector (`docs/SHIPPING.md`).

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

/// Header carrying the signature of the fake webhook.
pub const SIGNATURE_HEADER: &str = "x-fake-signature";

/// Code of the fake carrier.
pub const CODE: &str = "fake";

type HmacSha256 = Hmac<Sha256>;

/// Signature (hex) of `body` under key `secret`, as the fake carrier sends it.
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

/// A line of the fake webhook body (the body is a JSON array of these lines).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FakeWebhookUpdate {
    pub event_id: String,
    pub tracking_number: String,
    /// Common status (`in_transit`, `delivered`…) — the fake carrier uses the common status directly.
    pub status: ShipmentStatus,
    pub occurred_at: Timestamp,
    #[serde(default)]
    pub cod_collected: Option<Money>,
}

/// The fake carrier. Shared through `Arc`: the test keeps one handle to drive it, the carrier registry keeps the other.
#[derive(Default)]
pub struct FakeCarrier {
    /// Quote / create-shipment returns "the carrier does not respond".
    unavailable: AtomicBool,
    /// The next shipment creation: the carrier finishes creating it but returns an error (connection lost midway).
    lose_next_create_response: AtomicBool,
    /// Number of times `create_shipment` really created a new shipment.
    created: AtomicU64,
    next_number: AtomicU64,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// By `client_reference`.
    shipments: HashMap<String, CarrierShipment>,
    /// Journey by tracking number.
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

    /// Turns "the carrier does not respond" on / off.
    pub fn set_unavailable(&self, unavailable: bool) {
        self.unavailable.store(unavailable, Ordering::SeqCst);
    }

    /// The next shipment creation finishes but returns the error `carrier_unavailable`.
    pub fn lose_next_create_response(&self) {
        self.lose_next_create_response.store(true, Ordering::SeqCst);
    }

    /// Number of shipments really created (calling again with the same code does not count).
    pub fn created_count(&self) -> u64 {
        self.created.load(Ordering::SeqCst)
    }

    /// Adds a journey step to shipment `tracking_number` (as if the carrier updated it itself).
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

    /// The COD statement `cod_statement` will return.
    pub fn set_statement(&self, statement: Option<CodStatement>) {
        self.state().statement = statement;
    }

    /// Fake fee: 15 000 in the smallest unit for the first kg, 5 000 for each following kg (or part of a kg).
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

/// Compares two byte strings in time that does not depend on where they first differ.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}
