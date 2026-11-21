//! Phía người mua của đổi trả (mục 7 của bước 45, D-201, D-202, D-20261119-step45-returns): kiểm
//! yêu cầu trả (hạn, lý do, cách gửi trả, ảnh kèm), xem trước tiền hoàn — không ghi gì, gọi được
//! bao nhiêu lần cũng được — và yêu cầu hãng tới lấy hàng trả.

use autoxide_kernel::time::{Date, Timestamp};
use jiff::SignedDuration;
use std::collections::HashMap;

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::money::Money;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use autoxide_shipping::api::{
    Address, Direction, QuoteItem, QuoteRequest, ShipmentItem, ShipmentRequest,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::Value;
use uuid::Uuid;

use crate::api::OrderStatus;
use crate::carts::ShipTo;
use crate::entity::{line, order, return_request};
use crate::errors;
use crate::payments::Method;
use crate::reasons::RETURN_REASON;
use crate::return_refund::{self, Breakdown, Earlier};
use crate::returns::{self, MAX_PHOTOS, METHODS, RESOLUTIONS, ReturnInput, ReturnLine};
use crate::service::OrdersService;

const MAX_LINES: usize = 100;

/// Kết quả kiểm phần không phụ thuộc dòng của yêu cầu trả.
pub(crate) struct Checked {
    pub deadline_at: Timestamp,
    pub window_days: i64,
    pub photos_required: bool,
    /// Lỗi của đầu vào (gom cùng lỗi của dòng rồi báo một lần, D-083).
    pub problems: Vec<FieldError>,
}

/// Thời gian hoàn tiền dự kiến theo cách người mua đã trả (D-201): dữ liệu có ngày hiệu lực, dòng
/// có `effective_from` mới nhất không sau hôm nay thắng. Đây là ước lượng cho người mua, không phải
/// cam kết của đối tác thanh toán.
struct EtaRow {
    effective_from: (i16, i8, i8),
    /// `gateway` (thẻ, ví…), `transfer` (chuyển khoản, VietQR), `cash` (COD, tiền mặt).
    group: &'static str,
    min_days: u16,
    max_days: u16,
    /// Tiền về đâu: `original_payment` (về thẻ / ví đã trả), `bank_transfer`.
    channel: &'static str,
}

const REFUND_ETA: [EtaRow; 3] = [
    EtaRow {
        effective_from: (2026, 1, 1),
        group: "gateway",
        min_days: 5,
        max_days: 15,
        channel: "original_payment",
    },
    EtaRow {
        effective_from: (2026, 1, 1),
        group: "transfer",
        min_days: 1,
        max_days: 3,
        channel: "bank_transfer",
    },
    EtaRow {
        effective_from: (2026, 1, 1),
        group: "cash",
        min_days: 1,
        max_days: 3,
        channel: "bank_transfer",
    },
];

/// Thời gian hoàn dự kiến.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RefundEta {
    pub min_days: u16,
    pub max_days: u16,
    pub channel: &'static str,
}

/// Thời gian hoàn dự kiến cho đơn trả bằng `payment_method`, vào ngày `today`.
pub(crate) fn refund_eta(payment_method: &str, today: Date) -> Option<RefundEta> {
    let group = match Method::parse(payment_method) {
        Method::Cod => "cash",
        Method::Transfer => "transfer",
        Method::Gateway => "gateway",
    };
    REFUND_ETA
        .iter()
        .filter(|row| row.group == group)
        .filter(|row| {
            let (year, month, day) = row.effective_from;
            Date::new(year, month, day).is_some_and(|from| from <= today)
        })
        .max_by_key(|row| row.effective_from)
        .map(|row| RefundEta {
            min_days: row.min_days,
            max_days: row.max_days,
            channel: row.channel,
        })
}

/// Một cách gửi trả trong bản xem trước.
#[derive(Debug, Clone)]
pub(crate) struct MethodOption {
    pub method: &'static str,
    /// Người mua trả (0: cửa hàng chịu); `None` khi không báo được giá.
    pub fee: Option<Money>,
    pub available: bool,
}

/// Bản xem trước của một yêu cầu trả.
pub(crate) struct Preview {
    pub breakdown: Breakdown,
    pub currency: String,
    pub lines: Vec<ReturnLine>,
    pub methods: Vec<MethodOption>,
    pub refund_eta: Option<RefundEta>,
    pub deadline_at: Timestamp,
    pub photos_required: bool,
}

fn ship_to_of(row: &order::Model) -> Option<(ShipTo, Address)> {
    let ship_to: ShipTo = row.ship_to.as_deref().and_then(|raw| serde_json::from_str(raw).ok())?;
    let address = ship_to.address.clone()?;
    Some((ship_to, address))
}

fn method_id_of(row: &order::Model) -> Option<Uuid> {
    let option: Value =
        row.shipping_option.as_deref().and_then(|raw| serde_json::from_str(raw).ok())?;
    option["method_id"].as_str().and_then(|id| id.parse().ok())
}

/// Cách gửi trả `method` dùng được cho đơn `row` không: lấy tận nơi cần địa chỉ giao và phương thức
/// giao của đơn; trả tại cửa hàng chỉ khi đơn có cửa hàng nhận.
fn method_available(row: &order::Model, method: &str) -> bool {
    match method {
        "pickup" => ship_to_of(row).is_some() && method_id_of(row).is_some(),
        "in_store" => row.pickup_org_id.is_some(),
        "drop_off" => true,
        _ => false,
    }
}

/// Yêu cầu hãng tới lấy hàng trả (`direction = return`) từ địa chỉ giao của đơn về kho
/// `warehouse` — payload `after.shipment` của `orders.return_pickup_requested`.
pub(crate) fn pickup_request(
    row: &order::Model,
    stored: &return_request::Model,
    lines: &[line::Model],
    warehouse: OrgId,
) -> Result<ShipmentRequest, Error> {
    let (ship_to, address) = ship_to_of(row).ok_or_else(errors::delivery_required)?;
    let method_id = method_id_of(row).ok_or_else(errors::delivery_required)?;
    let mut items = Vec::new();
    let mut declared = 0;
    for returned in returns::lines_of(stored)? {
        let Some(line) = lines.iter().find(|line| line.id == returned.line_id) else { continue };
        let snapshot: Value = serde_json::from_str(&line.snapshot).unwrap_or(Value::Null);
        items.push(
            ShipmentItem::new(
                line.id.to_string(),
                line.variant_id,
                &line.sku,
                &line.name,
                returned.quantity,
            )
            .weight(snapshot["weight_grams"].as_i64()),
        );
        declared += return_refund::unit_value(line) * returned.quantity;
    }
    let recipient = (
        ship_to.recipient_name.clone().unwrap_or_default(),
        ship_to.phone.clone().unwrap_or_default(),
    );
    let mut request = ShipmentRequest::new(
        row.id,
        row.number.clone(),
        format!("return:{}", stored.id),
        OrgId::from_uuid(row.org_id),
        warehouse,
        method_id,
        recipient,
        address,
        items,
        Money::from_columns(declared, &row.currency)?,
    )
    .direction(Direction::Return)
    .note(format!("Return {}", stored.number));
    if let Some(buyer) = row.buyer_user_id {
        request = request.buyer(buyer);
    }
    Ok(request)
}

/// Kho nhận hàng trả mặc định: kho đã giao dòng đầu tiên của yêu cầu, không thì nút bán.
pub(crate) fn default_warehouse(
    row: &order::Model,
    lines: &[line::Model],
    wanted: &[Uuid],
) -> Uuid {
    lines
        .iter()
        .find(|line| wanted.contains(&line.id))
        .and_then(|line| line.warehouse_org_id)
        .unwrap_or(row.org_id)
}

impl OrdersService {
    /// Kiểm phần không phụ thuộc dòng của yêu cầu trả của người mua `buyer` cho đơn `row`: trạng
    /// thái và hạn (409), lý do, cách trả, ảnh kèm (gom vào `problems`).
    pub(crate) async fn check_return(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
        row: &order::Model,
        input: &ReturnInput,
    ) -> Result<Checked, Error> {
        let status = OrderStatus::parse(&row.status).unwrap_or(OrderStatus::Cancelled);
        let policy = self.policy(tenant, OrgId::from_uuid(row.org_id)).await?;
        if status == OrderStatus::Completed {
            return Err(errors::return_window_closed(policy.return_window_days));
        }
        if status != OrderStatus::Delivered {
            return Err(errors::return_not_allowed(status));
        }
        let delivered_at = row.delivered_at.unwrap_or(row.updated_at);
        let deadline_at = delivered_at + SignedDuration::from_hours(24 * policy.return_window_days);
        if crate::service::now() > deadline_at {
            return Err(errors::return_window_closed(policy.return_window_days));
        }
        let mut problems = Vec::new();
        let reason_allowed = policy.allowed_return_reasons.contains(&input.reason_code)
            && self.reasons.ensure(tenant, RETURN_REASON, &input.reason_code).await.is_ok();
        if !reason_allowed {
            problems.push(
                FieldError::new(
                    "reason_code",
                    "not_allowed",
                    "is not a return reason this shop accepts; allowed: {allowed}",
                )
                .with_param("allowed", policy.allowed_return_reasons.join(", ")),
            );
        }
        if !RESOLUTIONS.contains(&input.resolution.as_str()) {
            problems.push(FieldError::new(
                "resolution",
                "invalid",
                "must be `refund` or `exchange`",
            ));
        }
        if !METHODS.contains(&input.return_method.as_str()) {
            problems.push(FieldError::new(
                "return_method",
                "invalid",
                "must be `pickup`, `drop_off` or `in_store`",
            ));
        } else if !method_available(row, &input.return_method) {
            problems.push(FieldError::new(
                "return_method",
                "unavailable",
                "cannot be used for this order; see `methods` of the return preview",
            ));
        }
        if input.lines.is_empty() || input.lines.len() > MAX_LINES {
            problems.push(
                FieldError::new("lines", "length", "must have 1 to {max} lines")
                    .with_param("max", MAX_LINES),
            );
        }
        let photos_required = policy.return_photo_reasons.contains(&input.reason_code);
        self.check_photos(tenant, buyer, &input.media_ids, photos_required, &mut problems).await?;
        Ok(Checked {
            deadline_at,
            window_days: policy.return_window_days,
            photos_required,
            problems,
        })
    }

    /// Ảnh kèm (D-202): tối đa [`MAX_PHOTOS`], bắt buộc khi lý do nằm trong
    /// `orders.return_photo_reasons`, mỗi ảnh phải do chính người mua tải lên và không trong thùng
    /// rác (hỏi `media` qua trait, không đọc bảng của nó).
    pub(crate) async fn check_photos(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
        media_ids: &[Uuid],
        required: bool,
        problems: &mut Vec<FieldError>,
    ) -> Result<(), Error> {
        if media_ids.len() > MAX_PHOTOS {
            problems.push(
                FieldError::new("media_ids", "length", "must have at most {max} photos")
                    .with_param("max", MAX_PHOTOS),
            );
            return Ok(());
        }
        if media_ids.is_empty() {
            if required {
                problems.push(FieldError::new(
                    "media_ids",
                    "photos_required",
                    "this return reason needs photos of the items",
                ));
            }
            return Ok(());
        }
        let owners: HashMap<Uuid, Option<Uuid>> = match &self.media {
            Some(media) => media.uploaders(tenant, media_ids).await?.into_iter().collect(),
            None => HashMap::new(),
        };
        for (index, id) in media_ids.iter().enumerate() {
            if owners.get(id).copied().flatten() != Some(buyer) {
                problems.push(FieldError::new(
                    format!("media_ids[{index}]"),
                    "not_owned",
                    "is not a photo you uploaded",
                ));
            }
        }
        Ok(())
    }

    /// Xem trước một yêu cầu trả của người mua `buyer` (D-202): tiền hoàn, cách gửi trả và phí,
    /// thời gian hoàn, hạn, có cần ảnh không. Không ghi, không event, không giữ chỗ; đầu vào sai thì
    /// cùng lỗi với `POST …/returns`.
    pub(crate) async fn preview_return(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
        order_id: Uuid,
        input: &ReturnInput,
    ) -> Result<Preview, Error> {
        let row = self.order(tenant, order_id).await?;
        if row.buyer_user_id != Some(buyer) {
            return Err(errors::order_not_found());
        }
        let checked = self.check_return(tenant, buyer, &row, input).await?;
        let read = tenant.database().read();
        let lines = line::Entity::find()
            .filter(line::Column::OrderId.eq(order_id))
            .scoped(tenant)
            .all(read)
            .await?;
        let others = return_request::Entity::find()
            .filter(return_request::Column::OrderId.eq(order_id))
            .scoped(tenant)
            .all(read)
            .await?;
        let open = returns::open_quantities(&others)?;
        let evaluation = returns::evaluation_of(&row);
        let plan = returns::plan_lines(&lines, &open, &input.lines, evaluation.as_ref())?;
        let mut problems = checked.problems;
        problems.extend(plan.problems);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let mut earlier = Earlier::default();
        for other in &others {
            if let Some(breakdown) = returns::breakdown_of(other) {
                earlier.add(&breakdown);
            }
        }
        let returning: HashMap<Uuid, i64> =
            plan.lines.iter().map(|line| (line.line_id, line.quantity)).collect();
        let seller_fault = return_refund::SELLER_FAULT.contains(&input.reason_code.as_str());
        let breakdown = if input.resolution == "refund" {
            return_refund::refund_for(
                &row,
                &lines,
                &returning,
                seller_fault,
                evaluation.as_ref(),
                &earlier,
            )?
        } else {
            Breakdown::default()
        };
        let wanted: Vec<Uuid> = plan.wanted.keys().copied().collect();
        let warehouse = default_warehouse(&row, &lines, &wanted);
        let mut methods = Vec::new();
        for method in METHODS {
            let available = method_available(&row, method);
            let fee = match method {
                "pickup" if available && !seller_fault => {
                    self.pickup_fee(tenant, &row, &lines, &returning, warehouse).await
                }
                _ => Some(Money::from_columns(0, &row.currency)?),
            };
            methods.push(MethodOption { method, fee, available: available && fee.is_some() });
        }
        Ok(Preview {
            breakdown,
            currency: row.currency.clone(),
            lines: plan.lines,
            methods,
            refund_eta: refund_eta(&row.payment_method, Timestamp::now().date_utc()),
            deadline_at: checked.deadline_at,
            photos_required: checked.photos_required,
        })
    }

    /// Phí hãng lấy hàng trả (rẻ nhất), người mua trả khi không phải lỗi của người bán. Không báo
    /// được giá (chưa cài `shipping`, hãng lỗi) → `None`.
    async fn pickup_fee(
        &self,
        tenant: &TenantContext,
        row: &order::Model,
        lines: &[line::Model],
        returning: &HashMap<Uuid, i64>,
        warehouse: Uuid,
    ) -> Option<Money> {
        let rates = self.sources.shipping.as_ref()?;
        let (_, address) = ship_to_of(row)?;
        let mut value = 0;
        let items = lines
            .iter()
            .filter_map(|line| returning.get(&line.id).map(|quantity| (line, *quantity)))
            .map(|(line, quantity)| {
                value += return_refund::unit_value(line) * quantity;
                let snapshot: Value = serde_json::from_str(&line.snapshot).unwrap_or(Value::Null);
                QuoteItem::new(line.variant_id, quantity)
                    .sku(line.sku.clone())
                    .weight(snapshot["weight_grams"].as_i64())
            })
            .collect();
        let value = Money::from_columns(value, &row.currency).ok()?;
        let request =
            QuoteRequest::new(OrgId::from_uuid(row.org_id), items, value, Timestamp::now())
                .origin(OrgId::from_uuid(warehouse))
                .destination(address)
                .locale(row.locale.clone());
        match rates.quote(tenant, &request).await {
            Ok(quote) => quote
                .options
                .into_iter()
                .filter(|option| option.pickup_org_id.is_none())
                .map(|option| option.amount)
                .next(),
            Err(error) => {
                tracing::warn!(code = error.code(), "return pickup fee could not be quoted");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thoi_gian_hoan_theo_cach_tra_va_ngay_hieu_luc() {
        let today = Date::new(2026, 11, 19).unwrap();
        let card = refund_eta("card", today).unwrap();
        assert_eq!((card.min_days, card.max_days, card.channel), (5, 15, "original_payment"));
        assert_eq!(refund_eta("cod", today).unwrap().channel, "bank_transfer");
        // Trước ngày hiệu lực của mọi dòng: không có ước lượng.
        assert!(refund_eta("card", Date::new(2025, 1, 1).unwrap()).is_none());
    }
}
