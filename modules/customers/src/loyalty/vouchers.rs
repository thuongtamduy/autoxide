//! Tặng voucher cho người mua qua `Promotions::grant_voucher` (bước 47 mục 6, D-222) — không tự tạo
//! mã. Gọi lại cùng `source_ref` không cấp thêm (bên `promotions` chống trùng):
//!
//! - sinh nhật: job hằng ngày theo ngày giờ Việt Nam, `birthday:<năm>`;
//! - sau khi mua: khi đơn hoàn tất, `purchase:<order_id>`;
//! - đổi điểm lấy voucher: trừ điểm rồi cấp với `points:<id bút toán điểm>`; cấp lỗi thì hoàn điểm
//!   bằng lô mới cùng giá trị (không để mất điểm mà không có voucher).
//!
//! `grant_voucher` gọi NGOÀI transaction ghi của `customers`.

use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_promotions::api::{GrantVoucher, Voucher, VoucherSource};
use chrono::{Datelike, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use uuid::Uuid;

use super::ledger::{EXPENSE, LIABILITY, Line};
use super::points::NewLot;
use super::{current_rule, local_offset};
use crate::entity::{order_fact, profile, voucher_program};
use crate::service::{CustomersService, now};

pub(crate) const BIRTHDAY: &str = "birthday";
pub(crate) const PURCHASE: &str = "purchase";
pub(crate) const POINTS: &str = "points";

fn program_not_found() -> Error {
    Error::not_found(
        "customers.voucher_program_not_found",
        "No active voucher reward with this id.",
    )
}

fn promotions_missing() -> Error {
    Error::new(
        ErrorKind::Unavailable,
        "customers.vouchers_unavailable",
        "Vouchers are not available: the promotions module is not installed.",
    )
}

impl CustomersService {
    async fn programs(
        &self,
        tenant: &TenantContext,
        kind: &str,
    ) -> Result<Vec<voucher_program::Model>, Error> {
        Ok(voucher_program::Entity::find()
            .filter(voucher_program::Column::Kind.eq(kind))
            .filter(voucher_program::Column::Active.eq(true))
            .scoped(tenant)
            .order_by_asc(voucher_program::Column::Id)
            .all(tenant.database().read())
            .await?)
    }

    /// Cấp voucher; lỗi nghiệp vụ (hết lượt, chương trình đã kết thúc) chỉ ghi log — việc tặng không
    /// được chặn việc khác. Lỗi hạ tầng trả lên để event / job chạy lại.
    async fn grant(
        &self,
        tenant: &TenantContext,
        request: GrantVoucher,
    ) -> Result<Option<Voucher>, Error> {
        let Some(promotions) = &self.promotions else { return Ok(None) };
        match promotions.grant_voucher(tenant, &request).await {
            Ok(granted) => Ok(Some(granted.into_inner())),
            Err(error) if error.kind().status() < 500 => {
                tracing::warn!(code = error.code(), source_ref = %request.source_ref, "customer voucher not granted");
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Voucher sau khi mua (đơn vừa hoàn tất, người mua có tài khoản).
    pub(crate) async fn purchase_vouchers(
        &self,
        tenant: &TenantContext,
        fact: &order_fact::Model,
    ) -> Result<(), Error> {
        let Some(user) = profile::Entity::find_by_id(fact.profile_id)
            .one(tenant.database().read())
            .await?
            .and_then(|row| row.user_id)
        else {
            return Ok(());
        };
        let net = (fact.net_minor - fact.refunded_minor).max(0);
        for program in self.programs(tenant, PURCHASE).await? {
            if program.min_order_minor.is_some_and(|min| net < min) {
                continue;
            }
            let request = GrantVoucher::new(
                program.promotion_id,
                user,
                VoucherSource::PurchaseReward,
                format!("purchase:{}", fact.order_id),
            );
            self.grant(tenant, request).await?;
        }
        Ok(())
    }

    /// Voucher sinh nhật cho người có sinh nhật hôm nay (giờ Việt Nam). Trả số người đã xét.
    pub(crate) async fn birthday_vouchers(&self, tenant: &TenantContext) -> Result<u64, Error> {
        let programs = self.programs(tenant, BIRTHDAY).await?;
        if programs.is_empty() {
            return Ok(0);
        }
        let today = Utc::now().with_timezone(&local_offset()).date_naive();
        let rows = profile::Entity::find()
            .filter(profile::Column::BirthdayMd.eq(today.format("%m-%d").to_string()))
            .filter(profile::Column::UserId.is_not_null())
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        let count = rows.len() as u64;
        for row in rows {
            let Some(user) = row.user_id else { continue };
            for program in &programs {
                let request = GrantVoucher::new(
                    program.promotion_id,
                    user,
                    VoucherSource::Birthday,
                    format!("birthday:{}", today.year()),
                );
                self.grant(tenant, request).await?;
            }
        }
        Ok(count)
    }

    /// Người mua đổi điểm lấy voucher của chương trình `program_id` (loại `points`).
    pub(crate) async fn redeem_for_voucher(
        &self,
        tenant: &TenantContext,
        user: Uuid,
        program_id: Uuid,
    ) -> Result<Voucher, Error> {
        if self.promotions.is_none() {
            return Err(promotions_missing());
        }
        let program = voucher_program::Entity::find_by_id(program_id)
            .filter(voucher_program::Column::Kind.eq(POINTS))
            .filter(voucher_program::Column::Active.eq(true))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(program_not_found)?;
        let cost = program.points_cost.unwrap_or(0).max(1);
        let profile = self.me(tenant, user).await?;
        let reference = format!("voucher:{}", Uuid::now_v7());
        // 1. Trừ điểm (một transaction ngắn).
        let transaction = tenant.database().begin_write().await?;
        let taken = self
            .spend_now(
                &transaction,
                tenant,
                profile.id,
                cost,
                "redeem",
                &reference,
                None,
                Actor::user(user),
            )
            .await?;
        let currency = current_rule(&transaction, tenant, now())
            .await?
            .map_or_else(|| "VND".to_owned(), |rule| rule.currency);
        let value: i64 = taken.iter().map(|part| part.points.saturating_mul(part.value)).sum();
        self.queue_posting(
            &transaction,
            tenant,
            format!("customers:points:{reference}"),
            "Loyalty points exchanged for a voucher",
            &currency,
            vec![Line::debit(LIABILITY, value), Line::credit(EXPENSE, value)],
        )
        .await?;
        transaction.commit().await?;
        // 2. Cấp voucher ngoài transaction, cùng mã tham chiếu với bút toán điểm.
        let request = GrantVoucher::new(
            program.promotion_id,
            user,
            VoucherSource::Loyalty,
            format!("points:{reference}"),
        );
        let granted = match &self.promotions {
            Some(promotions) => promotions.grant_voucher(tenant, &request).await,
            None => Err(promotions_missing()),
        };
        match granted {
            Ok(voucher) => Ok(voucher.into_inner()),
            Err(error) => {
                // 3. Cấp lỗi: hoàn điểm thành lô mới cùng giá trị (bút toán đảo), rồi báo lỗi.
                let transaction = tenant.database().begin_write().await?;
                for (index, part) in taken.iter().enumerate() {
                    let lot = NewLot {
                        profile_id: profile.id,
                        points: part.points,
                        value: part.value,
                        currency: &currency,
                        valid_months: 1,
                        source_ref: format!("{reference}:restore:{index}"),
                        kind: "restore",
                        actor: Actor::system(),
                        reason: Some(format!("voucher not granted: {}", error.code())),
                    };
                    self.add_lot(&transaction, tenant, lot).await?;
                }
                transaction.commit().await?;
                tracing::warn!(code = error.code(), reference = %reference, "points restored after a voucher failed");
                Err(error)
            }
        }
    }
}
