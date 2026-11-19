//! Lô, hạn dùng và số serial (bước 42 mục 4) — bật theo sản phẩm (`lot_tracked`, `serial_tracked`
//! của `catalog`).
//!
//! - Lô: mỗi dòng tồn (biến thể × kho) có nhiều lô; tổng tồn của các lô ≤ `on_hand` (phần không
//!   gắn lô — vd chênh lệch kiểm kê dương — vẫn bán được). Giữ chỗ ở mức dòng tồn; chọn lô lúc xuất:
//!   lô nêu tên, hay tự chọn lô hết hạn trước (FEFO) trong các lô chưa hết hạn. Job hằng ngày trừ lô
//!   quá hạn khỏi `on_hand` và `available` (dòng sổ `expiry`) để không bán hàng hết hạn.
//! - `expiry_state` (`ok` / `expiring` / `expired`) tính lúc đọc từ `expires_on` và cài đặt kế thừa
//!   theo nút `inventory.expiring_days` (mặc định 30).
//! - Serial: nhập kho kèm đủ số serial; xuất kho phải nêu đủ serial đang `in_stock` ở kho đó; đổi
//!   trạng thái bằng câu `UPDATE … WHERE status = 'in_stock'` nên một serial không bán hai lần dù
//!   hai lần xuất chạy cùng lúc.

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::tenancy::{OrgSetting, ScopeExt, TenantContext};
use chrono::NaiveDate;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder,
};
use uuid::Uuid;

use crate::entity::{lot, serial};
use crate::service::now;
use crate::stock;

/// Cài đặt kế thừa theo nút: số ngày trước hạn dùng thì lô là "sắp hết hạn".
pub(crate) const EXPIRING_DAYS: OrgSetting = OrgSetting::new(
    "inventory.expiring_days",
    "Days before its expiry date a lot counts as expiring (0 to 3650).",
)
.with_default("30")
.with_schema(r#"{"type":"integer","minimum":0,"maximum":3650}"#)
.checked(check_days);

fn check_days(value: &serde_json::Value) -> Result<serde_json::Value, String> {
    match value.as_i64() {
        Some(days) if (0..=3650).contains(&days) => Ok(value.clone()),
        _ => Err("must be a whole number of days from 0 to 3650".to_owned()),
    }
}

/// Trạng thái hạn của lô lúc đọc.
pub(crate) fn expiry_state(
    expires_on: Option<NaiveDate>,
    today: NaiveDate,
    days: i64,
) -> &'static str {
    match expires_on {
        Some(day) if day < today => "expired",
        Some(day) if day <= today + chrono::Duration::days(days) => "expiring",
        _ => "ok",
    }
}

/// Mã lô: 1–64 ký tự in được.
pub(crate) fn check_lot_code(problems: &mut Vec<FieldError>, field: &str, code: &str) {
    let count = code.chars().count();
    if !(1..=64).contains(&count) || code.chars().any(char::is_control) {
        problems.push(FieldError::new(field, "length", "must be 1 to 64 characters"));
    }
}

/// Lỗi `inventory.lot_unavailable`.
pub(crate) fn lot_unavailable(code: &str) -> Error {
    Error::conflict(
        "inventory.lot_unavailable",
        "Lot `{lot_code}` does not hold enough units or has expired.",
    )
    .with_param("lot_code", code.to_owned())
}

/// Lỗi `inventory.serial_unavailable`.
pub(crate) fn serial_unavailable(serial: &str) -> Error {
    Error::conflict(
        "inventory.serial_unavailable",
        "Serial number `{serial}` is not in stock in this warehouse.",
    )
    .with_param("serial", serial.to_owned())
}

/// Lỗi `inventory.serials_required`.
pub(crate) fn serials_required(line_key: &str, count: i64) -> Error {
    Error::new(
        autoxide_kernel::error::ErrorKind::Validation,
        "inventory.serials_required",
        "Line `{line_key}` needs exactly {count} serial numbers.",
    )
    .with_param("line_key", line_key.to_owned())
    .with_param("count", count)
}

/// Cộng `quantity` vào lô `code` của dòng tồn `level` (tạo lô nếu chưa có); trả ID lô.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn lot_in<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    level: Uuid,
    org: Uuid,
    variant: Uuid,
    code: &str,
    expires_on: Option<NaiveDate>,
    quantity: i64,
) -> Result<Uuid, Error> {
    lot::Entity::insert(lot::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant.id().as_uuid()),
        level_id: Set(level),
        org_id: Set(org),
        variant_id: Set(variant),
        lot_code: Set(code.to_owned()),
        expires_on: Set(expires_on),
        on_hand: Set(0),
        received_at: Set(now()),
    })
    .on_conflict(
        OnConflict::columns([lot::Column::TenantId, lot::Column::LevelId, lot::Column::LotCode])
            .do_nothing()
            .to_owned(),
    )
    .exec_without_returning(transaction)
    .await?;
    let row = lot::Entity::find()
        .filter(lot::Column::LevelId.eq(level))
        .filter(lot::Column::LotCode.eq(code))
        .scoped(tenant)
        .one(transaction)
        .await?
        .ok_or_else(|| Error::internal("lot vanished right after it was created"))?;
    if expires_on.is_some() && row.expires_on.is_none() {
        lot::Entity::update_many()
            .col_expr(lot::Column::ExpiresOn, Expr::value(expires_on))
            .filter(lot::Column::Id.eq(row.id))
            .scoped(tenant)
            .exec(transaction)
            .await?;
    }
    stock::move_lot(transaction, tenant, row.id, quantity).await?;
    Ok(row.id)
}

/// Lấy `quantity` khỏi lô của dòng tồn `level`. Có `code`: chỉ lô đó (không đủ hay đã hết hạn →
/// `inventory.lot_unavailable`). Không có: các lô chưa hết hạn theo hạn dùng sớm nhất trước (FEFO),
/// phần còn lại lấy từ hàng không gắn lô (`on_hand` − tổng lô) — không bao giờ lấy vào lô đã hết
/// hạn. Trả các phần (ID lô hay `None` cho hàng không gắn lô, số lấy).
pub(crate) async fn lots_out<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    level: Uuid,
    code: Option<&str>,
    quantity: i64,
) -> Result<Vec<(Option<Uuid>, i64)>, Error> {
    let today = now().date_naive();
    let all = lot::Entity::find()
        .filter(lot::Column::LevelId.eq(level))
        .scoped(tenant)
        .order_by_asc(lot::Column::Id)
        .all(transaction)
        .await?;
    let mut usable: Vec<&lot::Model> = all
        .iter()
        .filter(|row| row.on_hand > 0 && row.expires_on.is_none_or(|day| day >= today))
        .filter(|row| code.is_none_or(|code| row.lot_code == code))
        .collect();
    // Lô không hạn xếp sau lô có hạn (SQLite và Postgres xếp NULL khác nhau: sắp ở đây).
    usable.sort_by_key(|row| (row.expires_on.is_none(), row.expires_on, row.id));
    let mut left = quantity;
    let mut picks = Vec::new();
    for row in usable {
        if left == 0 {
            break;
        }
        let take = left.min(row.on_hand);
        stock::move_lot(transaction, tenant, row.id, -take).await?;
        picks.push((Some(row.id), take));
        left -= take;
    }
    if left > 0 {
        if let Some(code) = code {
            return Err(lot_unavailable(code));
        }
        let on_hand = crate::entity::level::Entity::find_by_id(level)
            .scoped(tenant)
            .one(transaction)
            .await?
            .map_or(0, |row| row.on_hand);
        let in_lots: i64 = all.iter().map(|row| row.on_hand).sum();
        if on_hand - in_lots < left {
            return Err(lot_unavailable("FEFO"));
        }
        picks.push((None, left));
    }
    Ok(picks)
}

/// Lô và serial của một dòng hàng ra.
#[derive(Debug, Clone, Default)]
pub(crate) struct Tracking<'a> {
    pub lot_tracked: bool,
    pub lot_code: Option<&'a str>,
    /// Serial ra (đủ số) khi biến thể theo dõi serial.
    pub serials: Option<&'a [String]>,
    /// Trạng thái mới của serial: `sold`, `written_off`, `in_transit`.
    pub serial_status: &'a str,
    pub order: Option<Uuid>,
}

/// Ghi hàng RA (`new.quantity` âm): serial đổi trạng thái, lô trừ theo [`lots_out`], một dòng sổ
/// mỗi lô. Trả các dòng sổ và các phần lô đã lấy.
pub(crate) async fn record_out<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    touched: &mut stock::Touched,
    new: stock::NewMovement,
    tracking: &Tracking<'_>,
) -> Result<(Vec<crate::entity::movement::Model>, Vec<(Option<Uuid>, i64)>), Error> {
    if let Some(serials) = tracking.serials {
        serials_out(
            transaction,
            tenant,
            new.variant,
            new.org,
            serials,
            tracking.serial_status,
            tracking.order,
        )
        .await?;
    }
    if !tracking.lot_tracked {
        let row = stock::record(transaction, tenant, touched, new).await?;
        return Ok((vec![row], Vec::new()));
    }
    let level = stock::level_id(transaction, tenant, new.org, new.variant).await?;
    let picks = lots_out(transaction, tenant, level, tracking.lot_code, -new.quantity).await?;
    let mut rows = Vec::with_capacity(picks.len());
    for (lot, quantity) in &picks {
        let mut part = new.clone();
        part.quantity = -quantity;
        part.lot_id = *lot;
        rows.push(stock::record(transaction, tenant, touched, part).await?);
    }
    Ok((rows, picks))
}

/// Ghi hàng VÀO (`new.quantity` dương) kèm lô (`code`, hạn dùng) và serial khi có.
pub(crate) async fn record_in<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    touched: &mut stock::Touched,
    mut new: stock::NewMovement,
    lot: Option<(&str, Option<NaiveDate>)>,
    serials: &[String],
) -> Result<crate::entity::movement::Model, Error> {
    if let Some((code, expires_on)) = lot {
        let level = stock::level_id(transaction, tenant, new.org, new.variant).await?;
        new.lot_id = Some(
            lot_in(
                transaction,
                tenant,
                level,
                new.org,
                new.variant,
                code,
                expires_on,
                new.quantity,
            )
            .await?,
        );
    }
    if !serials.is_empty() {
        serials_in(transaction, tenant, new.variant, new.org, serials).await?;
    }
    stock::record(transaction, tenant, touched, new).await
}

/// Ghi các serial vào kho `org` (`in_stock`): mới thì thêm, đã có (trả về, chuyển đến) thì đổi
/// kho và trạng thái. Serial đang `in_stock` ở kho khác → `inventory.serial_unavailable`.
pub(crate) async fn serials_in<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    variant: Uuid,
    org: Uuid,
    serials: &[String],
) -> Result<(), Error> {
    for code in serials {
        let existing = serial::Entity::find()
            .filter(serial::Column::VariantId.eq(variant))
            .filter(serial::Column::Serial.eq(code.as_str()))
            .scoped(tenant)
            .one(transaction)
            .await?;
        match existing {
            None => {
                serial::Entity::insert(serial::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    variant_id: Set(variant),
                    serial: Set(code.clone()),
                    org_id: Set(Some(org)),
                    status: Set("in_stock".to_owned()),
                    order_id: Set(None),
                    updated_at: Set(now()),
                })
                .exec_without_returning(transaction)
                .await?;
            }
            Some(row) => {
                let changed = serial::Entity::update_many()
                    .col_expr(serial::Column::Status, Expr::value("in_stock"))
                    .col_expr(serial::Column::OrgId, Expr::value(Some(org)))
                    .col_expr(serial::Column::UpdatedAt, Expr::value(now()))
                    .filter(serial::Column::Id.eq(row.id))
                    .filter(serial::Column::Status.ne("in_stock"))
                    .scoped(tenant)
                    .exec(transaction)
                    .await?
                    .rows_affected;
                if changed == 0 {
                    return Err(serial_unavailable(code));
                }
            }
        }
    }
    Ok(())
}

/// Đánh dấu các serial đã bán cho đơn `order`; mỗi serial phải đang `in_stock` ở kho `org`. Một câu
/// `UPDATE` có điều kiện mỗi serial: hai lần xuất cùng serial chỉ một lần thành công.
pub(crate) async fn serials_out<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    variant: Uuid,
    org: Uuid,
    serials: &[String],
    status: &str,
    order: Option<Uuid>,
) -> Result<(), Error> {
    for code in serials {
        let changed = serial::Entity::update_many()
            .col_expr(serial::Column::Status, Expr::value(status))
            .col_expr(serial::Column::OrderId, Expr::value(order))
            .col_expr(serial::Column::UpdatedAt, Expr::value(now()))
            .filter(serial::Column::VariantId.eq(variant))
            .filter(serial::Column::Serial.eq(code.as_str()))
            .filter(serial::Column::OrgId.eq(org))
            .filter(serial::Column::Status.eq("in_stock"))
            .scoped(tenant)
            .exec(transaction)
            .await?
            .rows_affected;
        if changed == 0 {
            return Err(serial_unavailable(code));
        }
    }
    Ok(())
}

/// Kiểm danh sách serial của một dòng: đúng `quantity` số, mỗi số 1–100 ký tự, không trùng.
pub(crate) fn check_serials(
    problems: &mut Vec<FieldError>,
    field: &str,
    serials: &[String],
    quantity: i64,
) {
    let unique: std::collections::BTreeSet<&String> = serials.iter().collect();
    let fits = i64::try_from(serials.len()).is_ok_and(|count| count == quantity)
        && unique.len() == serials.len()
        && serials.iter().all(|code| (1..=100).contains(&code.chars().count()));
    if !fits {
        problems.push(
            FieldError::new(field, "count", "must list exactly {count} different serial numbers")
                .with_param("count", quantity),
        );
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::expiry_state;

    #[test]
    fn trang_thai_han_cua_lo() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 18).unwrap();
        let day = |d| NaiveDate::from_ymd_opt(2026, 11, d);
        assert_eq!(expiry_state(None, today, 30), "ok");
        assert_eq!(expiry_state(day(17), today, 30), "expired");
        assert_eq!(expiry_state(day(18), today, 30), "expiring");
        assert_eq!(expiry_state(NaiveDate::from_ymd_opt(2026, 12, 18), today, 30), "expiring");
        assert_eq!(expiry_state(NaiveDate::from_ymd_opt(2026, 12, 19), today, 30), "ok");
        assert_eq!(expiry_state(day(30), today, 0), "ok");
    }
}

/// Bộ lọc danh sách lô.
#[derive(Debug, Clone, Default)]
pub(crate) struct LotFilter {
    pub org_id: Option<Uuid>,
    pub variant_id: Option<Uuid>,
    /// `ok`, `expiring` hay `expired`.
    pub expiry_state: Option<String>,
}

/// Một lô kèm trạng thái hạn tính lúc đọc.
#[derive(Debug, Clone)]
pub(crate) struct LotView {
    pub row: lot::Model,
    pub expiry_state: &'static str,
}

impl crate::service::InventoryService {
    /// Số ngày "sắp hết hạn" ở nút `org` (cài đặt kế thừa `inventory.expiring_days`).
    pub(crate) async fn expiring_days(
        &self,
        tenant: &TenantContext,
        org: Uuid,
    ) -> Result<i64, Error> {
        let Some(orgs) = &self.orgs else { return Ok(30) };
        let found = orgs
            .setting(tenant, autoxide_kernel::tenancy::OrgId::from_uuid(org), EXPIRING_DAYS.key())
            .await?;
        Ok(found.and_then(|setting| setting.value).and_then(|value| value.as_i64()).unwrap_or(30))
    }

    pub(crate) async fn list_lots(
        &self,
        tenant: &TenantContext,
        principal: &autoxide_kernel::access::Principal,
        filter: &LotFilter,
        page: &autoxide_kernel::pagination::PageRequest,
    ) -> Result<autoxide_kernel::pagination::Page<LotView>, Error> {
        let scope =
            principal.scope_condition(&crate::permissions::READ, lot::Column::OrgId, None)?;
        let mut select = lot::Entity::find().scoped(tenant).filter(scope);
        if let Some(org) = filter.org_id {
            select = select.filter(lot::Column::OrgId.eq(org));
        }
        if let Some(variant) = filter.variant_id {
            select = select.filter(lot::Column::VariantId.eq(variant));
        }
        let root = autoxide_kernel::tenancy::root_of(tenant.id()).as_uuid();
        let days = self.expiring_days(tenant, filter.org_id.unwrap_or(root)).await?;
        let today = now().date_naive();
        let soon = today + chrono::Duration::days(days);
        match filter.expiry_state.as_deref() {
            None => {}
            Some("expired") => select = select.filter(lot::Column::ExpiresOn.lt(today)),
            Some("expiring") => {
                select = select
                    .filter(lot::Column::ExpiresOn.gte(today))
                    .filter(lot::Column::ExpiresOn.lte(soon));
            }
            Some("ok") => {
                select = select.filter(
                    sea_orm::Condition::any()
                        .add(lot::Column::ExpiresOn.is_null())
                        .add(lot::Column::ExpiresOn.gt(soon)),
                );
            }
            Some(_) => {
                return Err(Error::validation(vec![FieldError::new(
                    "expiry_state",
                    "invalid",
                    "must be `ok`, `expiring` or `expired`",
                )]));
            }
        }
        let read = tenant.database().read();
        let found = autoxide_kernel::pagination::fetch_page(
            select,
            lot::Column::Id,
            |row| row.id,
            page,
            read,
        )
        .await?;
        Ok(found.map(|row| {
            let state = expiry_state(row.expires_on, today, days);
            LotView { row, expiry_state: state }
        }))
    }

    /// Trừ các lô đã quá hạn dùng khỏi tồn (dòng sổ `expiry`, event `inventory.lot_expired`) trong
    /// `database` (mọi khách), tối đa `EXPIRE_BATCH` lô một lượt; mỗi lô một transaction.
    pub(crate) async fn expire_lots(
        &self,
        database: &autoxide_kernel::database::Database,
    ) -> Result<u64, Error> {
        use sea_orm::QuerySelect;
        let today = now().date_naive();
        let due: Vec<(Uuid, Uuid)> = lot::Entity::find()
            .select_only()
            .column(lot::Column::TenantId)
            .column(lot::Column::Id)
            .filter(lot::Column::ExpiresOn.lt(today))
            .filter(lot::Column::OnHand.gt(0))
            .limit(self.config.expire_batch)
            .into_tuple()
            .all(database.read())
            .await?;
        let mut expired = 0;
        for (tenant, id) in due {
            let tenant = self
                .databases
                .tenant(autoxide_kernel::tenancy::TenantId::from_uuid(tenant))
                .await?;
            match self.expire_lot(&tenant, id).await {
                Ok(true) => expired += 1,
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(lot_id = %id, error = %error, "cannot write off an expired lot")
                }
            }
        }
        Ok(expired)
    }

    async fn expire_lot(&self, tenant: &TenantContext, id: Uuid) -> Result<bool, Error> {
        let database = tenant.database();
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let Some(row) = autoxide_kernel::rows::lock_one(
                    &transaction,
                    lot::Entity::find_by_id(id).scoped(tenant),
                )
                .await?
                else {
                    return Ok(false);
                };
                if row.on_hand <= 0 {
                    return Ok(false);
                }
                let mut touched = stock::Touched::default();
                stock::move_lot(&transaction, tenant, row.id, -row.on_hand).await?;
                stock::record(
                    &transaction,
                    tenant,
                    &mut touched,
                    stock::NewMovement {
                        org: row.org_id,
                        variant: row.variant_id,
                        kind: stock::MovementKind::Expiry,
                        quantity: -row.on_hand,
                        touches_available: true,
                        unit_cost: None,
                        lot_id: Some(row.id),
                        reason_code: Some("expired".to_owned()),
                        note: row.lot_code.clone(),
                        source_type: "lot",
                        source_id: row.id,
                        actor: None,
                    },
                )
                .await?;
                let after = serde_json::json!({
                    "lot_id": row.id,
                    "lot_code": row.lot_code,
                    "org_id": row.org_id,
                    "variant_id": row.variant_id,
                    "expires_on": row.expires_on,
                    "quantity": row.on_hand,
                });
                let actor = autoxide_kernel::event::Actor::system();
                self.outbox
                    .record(
                        &transaction,
                        tenant,
                        actor,
                        Some(format!("inventory.lot:{}", row.id)),
                        &crate::events::LotExpired(crate::events::Change::after(after)),
                    )
                    .await?;
                stock::publish(&transaction, tenant, &self.outbox, &actor, touched).await?;
                transaction.commit().await?;
                Ok(true)
            })
            .await
    }
}
