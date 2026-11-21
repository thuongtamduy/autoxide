//! Ví voucher của người mua (D-222): voucher là quyền DÙNG MỘT LẦN trong ví của một tài khoản.
//! Vào ví bằng cách bấm "Lưu" (chương trình `claimable`), nhập mã nhận (`purpose = claim`), hay
//! được cấp qua `grant_voucher` (`customers`, `support`, `automations`, nhân viên cấp tay). Gọi lại
//! cùng `source` + `source_ref` trả voucher cũ, không cấp thêm (unique index quyết định).
//!
//! Số voucher phát (`voucher_quota`) và số lần mỗi người nhận (`per_user_claim_limit`) là bộ đếm
//! `rows::adjust` có trần trong transaction cấp: hai người nhận cùng lúc không vượt. Dữ liệu cá
//! nhân: ví có trong bản xuất; xoá tài khoản thì voucher chưa dùng bị thu hồi, voucher đã dùng giữ
//! cho chứng từ nhưng bỏ liên kết người.

use jiff::SignedDuration;
use std::time::Duration;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::{Actor, Delivered, EventHandler};
use autoxide_kernel::pagination::{Page, PageRequest, SortValue, fetch_page_by};
use autoxide_kernel::privacy::{PersonalData, deleted_user};
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::rows::{self, Bounds};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::throttle::Throttle;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, ExprTrait, QueryFilter, QueryOrder, QuerySelect,
    Set,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{GrantVoucher, Voucher, VoucherSource, VoucherStatus};
use crate::entity::{code, customer_usage, promotion, voucher};
use crate::evaluate::code_key;
use crate::events::{self, Change};
use crate::model::{Purpose, Status, Trigger};
use crate::service::{PromotionsService, is_unique_violation, now};

pub(crate) fn voucher_not_found() -> Error {
    Error::not_found("promotions.voucher_not_found", "There is no such voucher.")
}

fn source_str(source: VoucherSource) -> &'static str {
    match source {
        VoucherSource::Claim => "claim",
        VoucherSource::ClaimCode => "claim_code",
        VoucherSource::Grant => "grant",
        VoucherSource::Loyalty => "loyalty",
        VoucherSource::PurchaseReward => "purchase_reward",
        VoucherSource::Birthday => "birthday",
        VoucherSource::Referral => "referral",
        VoucherSource::Compensation => "compensation",
        VoucherSource::Automation => "automation",
    }
}

fn parse_source(text: &str) -> VoucherSource {
    match text {
        "claim" => VoucherSource::Claim,
        "claim_code" => VoucherSource::ClaimCode,
        "loyalty" => VoucherSource::Loyalty,
        "purchase_reward" => VoucherSource::PurchaseReward,
        "birthday" => VoucherSource::Birthday,
        "referral" => VoucherSource::Referral,
        "compensation" => VoucherSource::Compensation,
        "automation" => VoucherSource::Automation,
        _ => VoucherSource::Grant,
    }
}

pub(crate) fn status_str(status: VoucherStatus) -> &'static str {
    match status {
        VoucherStatus::Available => "available",
        VoucherStatus::Held => "held",
        VoucherStatus::Used => "used",
        VoucherStatus::Expired => "expired",
        VoucherStatus::Revoked => "revoked",
    }
}

fn parse_status(text: &str) -> VoucherStatus {
    match text {
        "held" => VoucherStatus::Held,
        "used" => VoucherStatus::Used,
        "expired" => VoucherStatus::Expired,
        "revoked" => VoucherStatus::Revoked,
        _ => VoucherStatus::Available,
    }
}

pub(crate) fn present(row: &voucher::Model) -> Voucher {
    Voucher {
        id: row.id,
        promotion_id: row.promotion_id,
        holder_user_id: row.holder_user_id,
        status: parse_status(&row.status),
        source: parse_source(&row.source),
        source_ref: row.source_ref.clone(),
        valid_from: row.valid_from,
        expires_at: row.expires_at,
        issued_at: row.issued_at,
        used_order_id: row.used_order_id,
    }
}

fn title_param(row: &promotion::Model) -> String {
    row.id.to_string()
}

impl PromotionsService {
    /// Cấp voucher vào ví (D-222).
    pub(crate) async fn grant_voucher(
        &self,
        tenant: &TenantContext,
        request: &GrantVoucher,
        actor: Actor,
    ) -> Result<Submitted<Voucher>, Error> {
        if request.source_ref.trim().is_empty() || request.source_ref.chars().count() > 200 {
            return Err(Error::validation(vec![FieldError::new(
                "source_ref",
                "length",
                "must be 1 to 200 characters long",
            )]));
        }
        self.issue(
            tenant,
            request.promotion_id,
            request.user_id,
            request.source,
            &request.source_ref,
            actor,
            None,
        )
        .await
    }

    /// Ghi một voucher: trong một transaction cộng số đã phát (trần `voucher_quota`), số lần
    /// người này nhận (trần `per_user_claim_limit`, chỉ với lưu / mã nhận), rồi chèn.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn issue(
        &self,
        tenant: &TenantContext,
        promotion_id: Uuid,
        user_id: Uuid,
        source: VoucherSource,
        source_ref: &str,
        actor: Actor,
        reason: Option<&str>,
    ) -> Result<Submitted<Voucher>, Error> {
        let existing = || async {
            Ok::<_, Error>(
                voucher::Entity::find()
                    .filter(voucher::Column::PromotionId.eq(promotion_id))
                    .filter(voucher::Column::HolderUserId.eq(user_id))
                    .filter(voucher::Column::Source.eq(source_str(source)))
                    .filter(voucher::Column::SourceRef.eq(source_ref))
                    .scoped(tenant)
                    .one(tenant.database().write())
                    .await?,
            )
        };
        if let Some(found) = existing().await? {
            return Ok(Submitted::Replayed(present(&found)));
        }
        let at = now();
        let transaction = tenant.database().begin_write().await?;
        let program = promotion::Entity::find_by_id(promotion_id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| crate::programs::not_found(promotion_id))?;
        let status = Status::parse(&program.status).unwrap_or(Status::Draft);
        if program.trigger != Trigger::Voucher.as_str()
            || !matches!(status, Status::Scheduled | Status::Active)
            || program.ends_at <= at
        {
            return Err(crate::programs::transitions::invalid_transition(status, Status::Active));
        }
        let tenant_id = tenant.id().as_uuid();
        let issued = rows::adjust::<promotion::Entity, _>(
            &transaction,
            promotion::Column::IssuedCount,
            promotion::Column::Id.eq(promotion_id).and(promotion::Column::TenantId.eq(tenant_id)),
            1,
            Bounds { floor: 0, ceiling: program.voucher_quota.unwrap_or(i64::MAX) },
        )
        .await?;
        if issued == 0 {
            return Err(Error::conflict(
                "promotions.voucher_quota_exhausted",
                "Promotion `{title}` has given out all its vouchers.",
            )
            .with_param("title", title_param(&program)));
        }
        if matches!(source, VoucherSource::Claim | VoucherSource::ClaimCode)
            && let Some(limit) = program.per_user_claim_limit
        {
            let key = format!("user:{user_id}");
            customer_usage::Entity::insert(customer_usage::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant_id),
                promotion_id: Set(promotion_id),
                subject: Set("claim".to_owned()),
                buyer_key: Set(key.clone()),
                period_key: Set("lifetime".to_owned()),
                used: Set(0),
            })
            .on_conflict(
                OnConflict::columns([
                    customer_usage::Column::TenantId,
                    customer_usage::Column::PromotionId,
                    customer_usage::Column::Subject,
                    customer_usage::Column::BuyerKey,
                    customer_usage::Column::PeriodKey,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(&transaction)
            .await?;
            let claimed = rows::adjust::<customer_usage::Entity, _>(
                &transaction,
                customer_usage::Column::Used,
                customer_usage::Column::TenantId
                    .eq(tenant_id)
                    .and(customer_usage::Column::PromotionId.eq(promotion_id))
                    .and(customer_usage::Column::Subject.eq("claim"))
                    .and(customer_usage::Column::BuyerKey.eq(key))
                    .and(customer_usage::Column::PeriodKey.eq("lifetime")),
                1,
                Bounds { floor: 0, ceiling: i64::from(limit) },
            )
            .await?;
            if claimed == 0 {
                return Err(Error::conflict(
                    "promotions.claim_limit_reached",
                    "You have received all the vouchers allowed for promotion `{title}`.",
                )
                .with_param("title", title_param(&program)));
            }
        }
        let valid_from = std::cmp::max(program.starts_at, at);
        let expires_at = program.voucher_valid_days.map_or(program.ends_at, |days| {
            std::cmp::min(at + SignedDuration::from_hours(24 * (i64::from(days))), program.ends_at)
        });
        let id = Uuid::now_v7();
        let inserted = voucher::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id),
            promotion_id: Set(promotion_id),
            holder_user_id: Set(Some(user_id)),
            status: Set("available".to_owned()),
            source: Set(source_str(source).to_owned()),
            source_ref: Set(source_ref.to_owned()),
            valid_from: Set(valid_from),
            expires_at: Set(expires_at),
            issued_at: Set(at),
            used_order_id: Set(None),
            updated_at: Set(at),
        }
        .insert(&transaction)
        .await;
        let row = match inserted {
            Ok(row) => row,
            Err(error) if is_unique_violation(&error) => {
                drop(transaction);
                return existing()
                    .await?
                    .map(|found| Submitted::Replayed(present(&found)))
                    .ok_or_else(voucher_not_found);
            }
            Err(error) => return Err(error.into()),
        };
        self.outbox
            .record(
                &transaction,
                tenant,
                actor,
                Some(format!("promotions.voucher:{id}")),
                &events::VoucherIssued(Change::after(json!({
                    "id": id, "promotion_id": promotion_id, "user_id": user_id,
                    "source": source_str(source), "source_ref": source_ref, "reason": reason,
                }))),
            )
            .await?;
        transaction.commit().await?;
        Ok(Submitted::Created(present(&row)))
    }

    /// Người mua bấm "Lưu" voucher của chương trình `claimable` (D-222, D-225).
    pub(crate) async fn claim(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        promotion_id: Uuid,
    ) -> Result<Submitted<Voucher>, Error> {
        let program = promotion::Entity::find_by_id(promotion_id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .filter(|row| row.claimable && row.public)
            .ok_or_else(|| crate::programs::not_found(promotion_id))?;
        if program.claim_requires_verified_phone {
            // Xác thực số điện thoại thuộc `auth` / `customers` (bước 47); chưa có cách hỏi.
            return Err(Error::new(
                ErrorKind::Forbidden,
                "promotions.claim_requires_verified_phone",
                "A verified phone number is needed to receive this voucher.",
            ));
        }
        let user = principal.user_id();
        // Mỗi lần lưu một `source_ref` mới; số lần do `per_user_claim_limit` chặn.
        let taken = voucher::Entity::find()
            .filter(voucher::Column::PromotionId.eq(promotion_id))
            .filter(voucher::Column::HolderUserId.eq(user))
            .filter(voucher::Column::Source.eq("claim"))
            .scoped(tenant)
            .all(tenant.database().write())
            .await?
            .len();
        let source_ref = if program.per_user_claim_limit.unwrap_or(1) <= 1 {
            "claim".to_owned()
        } else {
            format!("claim:{}", taken + 1)
        };
        self.issue(
            tenant,
            promotion_id,
            user,
            VoucherSource::Claim,
            &source_ref,
            Actor::from(principal),
            None,
        )
        .await
    }

    /// Người mua nhập mã nhận (`purpose = claim`): cấp một voucher vào ví. Giới hạn số lần thử
    /// theo người mua (rate limiting của kernel).
    pub(crate) async fn claim_code(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        entered: &str,
    ) -> Result<Submitted<Voucher>, Error> {
        let throttle = Throttle::new("promotions", 10, Duration::from_secs(600))?;
        let user = principal.user_id();
        if !throttle.hit(tenant, &format!("claim_code:{user}")).await?.is_allowed() {
            return Err(Error::new(
                ErrorKind::TooManyRequests,
                "promotions.too_many_attempts",
                "Too many codes tried; wait a few minutes and try again.",
            ));
        }
        let key = code_key(entered);
        let found = code::Entity::find()
            .filter(code::Column::CodeKey.eq(key))
            .filter(code::Column::Purpose.eq(Purpose::Claim.as_str()))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::Validation,
                    "promotions.code_invalid",
                    "Code `{code}` does not exist.",
                )
                .with_param("code", entered)
            })?;
        self.issue(
            tenant,
            found.promotion_id,
            user,
            VoucherSource::ClaimCode,
            &found.code_key,
            Actor::from(principal),
            None,
        )
        .await
    }

    /// Ví voucher của một tài khoản, hết hạn gần nhất trước.
    pub(crate) async fn vouchers_of(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        status: Option<VoucherStatus>,
    ) -> Result<Vec<Voucher>, Error> {
        let mut select = voucher::Entity::find()
            .filter(voucher::Column::HolderUserId.eq(user_id))
            .scoped(tenant);
        if let Some(status) = status {
            select = select.filter(voucher::Column::Status.eq(status_str(status)));
        }
        Ok(select
            .order_by_asc(voucher::Column::ExpiresAt)
            .order_by_asc(voucher::Column::Id)
            .all(tenant.database().read())
            .await?
            .iter()
            .map(present)
            .collect())
    }

    /// Ví voucher của tài khoản `user_id` cho nhân viên (tool MCP): chỉ voucher của chương trình
    /// thuộc nút trong phạm vi `promotions.read` của `principal` — như REST, không thấy dữ liệu
    /// của nhánh khác.
    pub(crate) async fn vouchers_of_in_scope(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        user_id: Uuid,
        status: Option<VoucherStatus>,
    ) -> Result<Vec<Voucher>, Error> {
        principal.require(&crate::permissions::READ)?;
        let vouchers = self.vouchers_of(tenant, user_id, status).await?;
        let mut ids: Vec<Uuid> = vouchers.iter().map(|voucher| voucher.promotion_id).collect();
        ids.sort();
        ids.dedup();
        if ids.is_empty() {
            return Ok(vouchers);
        }
        let visible: Vec<Uuid> = promotion::Entity::find()
            .select_only()
            .column(promotion::Column::Id)
            .filter(promotion::Column::Id.is_in(ids))
            .filter(principal.scope_condition(
                &crate::permissions::READ,
                promotion::Column::OrgId,
                None,
            )?)
            .scoped(tenant)
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        Ok(vouchers.into_iter().filter(|voucher| visible.contains(&voucher.promotion_id)).collect())
    }

    /// Một trang ví voucher (hết hạn gần nhất trước).
    pub(crate) async fn wallet(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        status: Option<VoucherStatus>,
        page: &PageRequest,
    ) -> Result<Page<Voucher>, Error> {
        let mut select = voucher::Entity::find()
            .filter(voucher::Column::HolderUserId.eq(user_id))
            .scoped(tenant);
        if let Some(status) = status {
            select = select.filter(voucher::Column::Status.eq(status_str(status)));
        }
        let page = fetch_page_by(
            select,
            voucher::Column::ExpiresAt,
            |row| SortValue::Time(row.expires_at),
            voucher::Column::Id,
            |row| row.id,
            page,
            tenant.database().read(),
        )
        .await?;
        Ok(page.map(|row| present(&row)))
    }

    /// Job `promotions.expire_vouchers` (mỗi giờ): voucher còn trong ví đã quá hạn → `expired`,
    /// theo lô, không phát event cho từng voucher.
    pub(crate) async fn expire_vouchers(&self, tenant: &TenantContext) -> Result<u64, Error> {
        let at = now();
        Ok(voucher::Entity::update_many()
            .col_expr(voucher::Column::Status, Expr::value("expired"))
            .col_expr(voucher::Column::UpdatedAt, Expr::value(at))
            .filter(voucher::Column::Status.eq("available"))
            .filter(voucher::Column::ExpiresAt.lte(at))
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?
            .rows_affected)
    }
}

#[async_trait::async_trait]
impl PersonalData for PromotionsService {
    async fn export(&self, tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
        let vouchers = self.vouchers_of(tenant, user_id, None).await?;
        let reminders = crate::entity::reminder::Entity::find()
            .filter(crate::entity::reminder::Column::UserId.eq(user_id))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        Ok(json!({
            "vouchers": vouchers,
            "reminders": reminders.iter().map(|found| json!({
                "promotion_id": found.promotion_id, "remind_at": found.remind_at, "sent_at": found.sent_at,
            })).collect::<Vec<_>>(),
        }))
    }
}

/// Xoá tài khoản (Luật 91/2025): voucher chưa dùng → `revoked`; mọi voucher bỏ liên kết người
/// (voucher đã dùng giữ cho chứng từ); lời nhắc bị xoá. Nhận lại event không sao.
pub(crate) struct ErasedUsers;

#[async_trait::async_trait]
impl EventHandler for ErasedUsers {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(user_id) = deleted_user(event) else {
            return Ok(());
        };
        erase_user(&event.tenant, user_id).await?;
        tracing::info!(user_id = %user_id, "vouchers of a deleted user revoked");
        Ok(())
    }
}

/// Thu hồi voucher chưa dùng và bỏ liên kết người của tài khoản đã xoá.
pub(crate) async fn erase_user(tenant: &TenantContext, user_id: Uuid) -> Result<(), Error> {
    let at = now();
    let transaction = tenant.database().begin_write().await?;
    voucher::Entity::update_many()
        .col_expr(voucher::Column::Status, Expr::value("revoked"))
        .col_expr(voucher::Column::UpdatedAt, Expr::value(at))
        .filter(voucher::Column::HolderUserId.eq(user_id))
        .filter(voucher::Column::Status.eq("available"))
        .scoped(tenant)
        .exec(&transaction)
        .await?;
    voucher::Entity::update_many()
        .col_expr(voucher::Column::HolderUserId, Expr::value(Option::<Uuid>::None))
        .col_expr(voucher::Column::UpdatedAt, Expr::value(at))
        .filter(voucher::Column::HolderUserId.eq(user_id))
        .scoped(tenant)
        .exec(&transaction)
        .await?;
    crate::entity::reminder::Entity::delete_many()
        .filter(crate::entity::reminder::Column::UserId.eq(user_id))
        .scoped(tenant)
        .exec(&transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

impl PromotionsService {
    /// Cấp tay một voucher (lý do bắt buộc, có trong event → audit).
    pub(crate) async fn grant_by_hand(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        reason: &str,
        user_id: Uuid,
        reference: Option<String>,
    ) -> Result<autoxide_kernel::reference::Submitted<Voucher>, Error> {
        if !(3..=500).contains(&reason.trim().chars().count()) {
            return Err(Error::validation(vec![autoxide_kernel::error::FieldError::new(
                "reason",
                "length",
                "must be 3 to 500 characters long",
            )]));
        }
        let program = self.promotion(tenant, principal, id).await?;
        crate::service::readable(
            principal,
            tenant.database().read(),
            &crate::permissions::VOUCHERS_GRANT,
            program.row.org_id,
            || crate::programs::not_found(id),
        )
        .await?;
        let source_ref = reference.unwrap_or_else(|| format!("grant:{}", Uuid::now_v7()));
        tracing::info!(promotion_id = %id, user_id = %user_id, "voucher granted by hand");
        let source_ref: String = source_ref.chars().take(200).collect();
        self.issue(
            tenant,
            id,
            user_id,
            VoucherSource::Grant,
            &source_ref,
            Actor::from(principal),
            Some(reason.trim()),
        )
        .await
    }
}
