//! Trạng thái chống đoán mã đăng nhập SMS theo số (PT-7, D-20261126-otp-nist): client quen của số và bộ đếm lần sai
//! liên tiếp. Mọi hàm nhận khoá của số (`SecretKey::phone_key`, HMAC theo khách), không nhận số.

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use sea_orm::sea_query::{Expr, ExprTrait as _, OnConflict};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

use crate::entity::{phone_challenge, phone_failure, phone_known_client};

/// Số lần sai liên tiếp của số `phone_key` kể từ lần đúng gần nhất.
pub(crate) async fn failures<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    phone_key: &str,
) -> Result<i32, Error> {
    Ok(phone_failure::Entity::find()
        .filter(phone_failure::Column::PhoneKey.eq(phone_key))
        .scoped(tenant)
        .one(connection)
        .await?
        .map_or(0, |row| row.failures))
}

/// Cộng một lần sai cho số (trong transaction của lần kiểm mã).
pub(crate) async fn record_failure<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    phone_key: &str,
    at: Timestamp,
) -> Result<(), Error> {
    phone_failure::Entity::insert(phone_failure::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant.id().as_uuid()),
        phone_key: Set(phone_key.to_owned()),
        failures: Set(1),
        updated_at: Set(at),
        unlocked_at: Set(None),
    })
    .on_conflict(
        OnConflict::columns([phone_failure::Column::TenantId, phone_failure::Column::PhoneKey])
            .value(
                phone_failure::Column::Failures,
                Expr::col((phone_failure::Entity, phone_failure::Column::Failures)).add(1),
            )
            .value(phone_failure::Column::UpdatedAt, Expr::value(at))
            .to_owned(),
    )
    .exec_without_returning(connection)
    .await?;
    Ok(())
}

/// Quản trị viên mở khoá số: chỉ khi số đang khoá (`locked_from` lần sai liên tiếp trở lên) — đưa bộ đếm về 0 và ghi
/// mốc `unlocked_at` (giới hạn mã sai mỗi ngày coi như một lần đúng). `true` nếu số đang khoá và đã được mở.
pub(crate) async fn unlock<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    phone_key: &str,
    locked_from: i32,
    at: Timestamp,
) -> Result<bool, Error> {
    let updated = phone_failure::Entity::update_many()
        .col_expr(phone_failure::Column::Failures, Expr::value(0))
        .col_expr(phone_failure::Column::UpdatedAt, Expr::value(at))
        .col_expr(phone_failure::Column::UnlockedAt, Expr::value(Some(at)))
        .filter(phone_failure::Column::PhoneKey.eq(phone_key))
        .filter(phone_failure::Column::Failures.gte(locked_from))
        .scoped(tenant)
        .exec(connection)
        .await?;
    Ok(updated.rows_affected > 0)
}

/// Lúc quản trị viên mở khoá số gần nhất (nếu có).
pub(crate) async fn unlocked_at<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    phone_key: &str,
) -> Result<Option<Timestamp>, Error> {
    Ok(phone_failure::Entity::find()
        .filter(phone_failure::Column::PhoneKey.eq(phone_key))
        .scoped(tenant)
        .one(connection)
        .await?
        .and_then(|row| row.unlocked_at))
}

/// Lần đúng: xoá bộ đếm lần sai của số (NIST SP 800-63B-4 §3.2.2) và cho hết hạn mọi mã đăng nhập còn mở khác của số
/// — mã tạo trước lần đúng không còn nhận lượt đoán nào nữa.
pub(crate) async fn succeeded<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    phone: &str,
    phone_key: &str,
    used: Uuid,
    at: Timestamp,
) -> Result<(), Error> {
    phone_failure::Entity::delete_many()
        .filter(phone_failure::Column::PhoneKey.eq(phone_key))
        .scoped(tenant)
        .exec(connection)
        .await?;
    phone_challenge::Entity::update_many()
        .col_expr(phone_challenge::Column::ExpiresAt, Expr::value(at))
        .filter(phone_challenge::Column::Phone.eq(phone))
        // Chỉ mã đăng nhập (chưa có tài khoản): mã xác thực số của người đã đăng nhập thuộc tài khoản của họ.
        .filter(phone_challenge::Column::UserId.eq(Uuid::nil()))
        .filter(phone_challenge::Column::Id.ne(used))
        .filter(phone_challenge::Column::ConsumedAt.is_null())
        .filter(phone_challenge::Column::ExpiresAt.gt(at))
        .scoped(tenant)
        .exec(connection)
        .await?;
    Ok(())
}

/// Nhớ client `block` là client quen của số (đăng nhập đúng lúc `at`).
pub(crate) async fn remember_client<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    phone_key: &str,
    block: &str,
    at: Timestamp,
) -> Result<(), Error> {
    phone_known_client::Entity::insert(phone_known_client::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant.id().as_uuid()),
        phone_key: Set(phone_key.to_owned()),
        requester_block: Set(block.to_owned()),
        last_success_at: Set(at),
    })
    .on_conflict(
        OnConflict::columns([
            phone_known_client::Column::TenantId,
            phone_known_client::Column::PhoneKey,
            phone_known_client::Column::RequesterBlock,
        ])
        .update_column(phone_known_client::Column::LastSuccessAt)
        .to_owned(),
    )
    .exec_without_returning(connection)
    .await?;
    Ok(())
}

/// Client `block` đã đăng nhập đúng bằng mã của số sau `since`.
pub(crate) async fn is_known<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    phone_key: &str,
    block: Option<&str>,
    since: Timestamp,
) -> Result<bool, Error> {
    let Some(block) = block else { return Ok(false) };
    Ok(phone_known_client::Entity::find()
        .filter(phone_known_client::Column::PhoneKey.eq(phone_key))
        .filter(phone_known_client::Column::RequesterBlock.eq(block))
        .filter(phone_known_client::Column::LastSuccessAt.gt(since))
        .scoped(tenant)
        .one(connection)
        .await?
        .is_some())
}
