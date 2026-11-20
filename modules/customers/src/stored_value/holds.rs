//! Giữ / xác nhận / trả giá trị của thẻ và số dư cho một đơn, và job dọn chỗ giữ quá hạn, thẻ hết hạn.

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::Money;
use autoxide_kernel::rows::{self, Bounds};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use chrono::{DateTime, Utc};
use sea_orm::sea_query::{Expr, ExprTrait as _, OnConflict};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

use crate::entity::{gift_card, store_credit, stored_value_hold};
use crate::loyalty::ledger::{Line, PREPAYMENTS};
use crate::service::{CustomersService, now};

use super::{BREAKAGE, CREDIT_LIABILITY, GIFT_CARD, GIFT_LIABILITY, HELD, insufficient};

impl CustomersService {
    /// Giữ giá trị của một công cụ cho đơn `reference`: ghi chỗ giữ trước ("trùng thì bỏ qua" —
    /// gọi lại hay hai lần cùng lúc chỉ giữ một lần), rồi trừ số dư bằng một câu có điều kiện.
    pub(super) async fn hold_value(
        &self,
        tenant: &TenantContext,
        instrument: &'static str,
        id: Uuid,
        reference: &str,
        wanted: Money,
        expires_at: DateTime<Utc>,
    ) -> Result<Money, Error> {
        let key = (tenant.id().as_uuid(), reference.to_owned(), instrument.to_owned());
        let transaction = tenant.database().begin_write().await?;
        let inserted = stored_value_hold::Entity::insert(stored_value_hold::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            reference: Set(reference.to_owned()),
            instrument: Set(instrument.to_owned()),
            instrument_id: Set(id),
            amount_minor: Set(wanted.minor()),
            currency: Set(wanted.currency().code().to_owned()),
            status: Set(HELD.to_owned()),
            expires_at: Set(expires_at),
            created_at: Set(now()),
        })
        .on_conflict(OnConflict::new().do_nothing().to_owned())
        .exec_without_returning(&transaction)
        .await?;
        if inserted == 0 {
            transaction.rollback().await?;
            let existing = stored_value_hold::Entity::find_by_id(key)
                .one(tenant.database().write())
                .await?
                .ok_or_else(|| Error::internal("stored value hold vanished"))?;
            return Money::from_columns(existing.amount_minor, &existing.currency);
        }
        let changed = if instrument == GIFT_CARD {
            let filter = Condition::all()
                .add(gift_card::Column::Id.eq(id))
                .add(gift_card::Column::TenantId.eq(tenant.id().as_uuid()))
                .add(gift_card::Column::Status.eq("active"));
            rows::adjust::<gift_card::Entity, _>(
                &transaction,
                gift_card::Column::BalanceMinor,
                filter,
                -wanted.minor(),
                Bounds::NON_NEGATIVE,
            )
            .await?
        } else {
            let filter = Condition::all()
                .add(store_credit::Column::ProfileId.eq(id))
                .add(store_credit::Column::TenantId.eq(tenant.id().as_uuid()));
            rows::adjust::<store_credit::Entity, _>(
                &transaction,
                store_credit::Column::BalanceMinor,
                filter,
                -wanted.minor(),
                Bounds::NON_NEGATIVE,
            )
            .await?
        };
        if changed == 0 {
            transaction.rollback().await?;
            let read = tenant.database().read();
            let available = if instrument == GIFT_CARD {
                gift_card::Entity::find_by_id(id)
                    .one(read)
                    .await?
                    .map_or(0, |row| row.balance_minor)
            } else {
                self.credit_balance(read, tenant, id).await?
            };
            return Err(insufficient(available));
        }
        self.sv_entry(
            &transaction,
            tenant,
            instrument,
            id,
            "hold",
            Money::new(-wanted.minor(), wanted.currency()),
            reference,
            Actor::system(),
        )
        .await?;
        transaction.commit().await?;
        Ok(wanted)
    }

    /// Xác nhận (`confirm`) hay trả (`!confirm`) mọi chỗ giữ thẻ / số dư của đơn `reference`.
    pub(crate) async fn settle_value_holds(
        &self,
        tenant: &TenantContext,
        reference: &str,
        confirm: bool,
    ) -> Result<(), Error> {
        let holds = stored_value_hold::Entity::find()
            .filter(stored_value_hold::Column::Reference.eq(reference))
            .filter(stored_value_hold::Column::Status.eq(HELD))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        for hold in holds {
            let transaction = tenant.database().begin_write().await?;
            let changed = stored_value_hold::Entity::update_many()
                .col_expr(
                    stored_value_hold::Column::Status,
                    Expr::value(if confirm { "confirmed" } else { "released" }),
                )
                .filter(stored_value_hold::Column::Reference.eq(reference))
                .filter(stored_value_hold::Column::Instrument.eq(hold.instrument.as_str()))
                .filter(stored_value_hold::Column::Status.eq(HELD))
                .scoped(tenant)
                .exec(&transaction)
                .await?
                .rows_affected;
            if changed == 0 {
                continue;
            }
            let money = Money::from_columns(hold.amount_minor, &hold.currency)?;
            if confirm {
                self.sv_entry(
                    &transaction,
                    tenant,
                    &hold.instrument,
                    hold.instrument_id,
                    "redeem",
                    money,
                    reference,
                    Actor::system(),
                )
                .await?;
                let liability =
                    if hold.instrument == GIFT_CARD { GIFT_LIABILITY } else { CREDIT_LIABILITY };
                self.queue_posting(
                    &transaction,
                    tenant,
                    format!("customers:{}:redeem:{reference}", hold.instrument),
                    "Gift card or store credit used for an order",
                    &hold.currency,
                    vec![
                        Line::debit(liability, hold.amount_minor),
                        Line::credit(PREPAYMENTS, hold.amount_minor),
                    ],
                )
                .await?;
            } else {
                if hold.instrument == GIFT_CARD {
                    let returned = gift_card::Entity::update_many()
                        .col_expr(
                            gift_card::Column::BalanceMinor,
                            Expr::col(gift_card::Column::BalanceMinor).add(hold.amount_minor),
                        )
                        .filter(gift_card::Column::Id.eq(hold.instrument_id))
                        .filter(gift_card::Column::Status.eq("active"))
                        .scoped(tenant)
                        .exec(&transaction)
                        .await?
                        .rows_affected;
                    // Thẻ đã huỷ / hết hạn trong lúc đang giữ: phần giữ không về thẻ mà thành thu
                    // nhập khác, như số dư lúc huỷ.
                    if returned == 0 {
                        self.queue_posting(
                            &transaction,
                            tenant,
                            format!("customers:gift_card:release:{reference}"),
                            "Gift card hold released after the card ended",
                            &hold.currency,
                            vec![
                                Line::debit(GIFT_LIABILITY, hold.amount_minor),
                                Line::credit(BREAKAGE, hold.amount_minor),
                            ],
                        )
                        .await?;
                    }
                } else {
                    store_credit::Entity::update_many()
                        .col_expr(
                            store_credit::Column::BalanceMinor,
                            Expr::col(store_credit::Column::BalanceMinor).add(hold.amount_minor),
                        )
                        .filter(store_credit::Column::ProfileId.eq(hold.instrument_id))
                        .scoped(tenant)
                        .exec(&transaction)
                        .await?;
                }
                self.sv_entry(
                    &transaction,
                    tenant,
                    &hold.instrument,
                    hold.instrument_id,
                    "release",
                    money,
                    reference,
                    Actor::system(),
                )
                .await?;
            }
            transaction.commit().await?;
        }
        Ok(())
    }

    /// Trả chỗ giữ quá hạn và cho thẻ quá hạn hết hiệu lực (job, chỉ khi đã bật).
    pub(crate) async fn stored_value_sweep(&self, tenant: &TenantContext) -> Result<u64, Error> {
        if !self.config.stored_value_enabled {
            return Ok(0);
        }
        let due = stored_value_hold::Entity::find()
            .filter(stored_value_hold::Column::Status.eq(HELD))
            .filter(stored_value_hold::Column::ExpiresAt.lte(now()))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        let mut done = due.len() as u64;
        for hold in due {
            self.settle_value_holds(tenant, &hold.reference, false).await?;
        }
        let expired = gift_card::Entity::find()
            .filter(gift_card::Column::Status.eq("active"))
            .filter(gift_card::Column::ExpiresAt.lte(now()))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        for card in expired {
            let transaction = tenant.database().begin_write().await?;
            let changed = gift_card::Entity::update_many()
                .col_expr(gift_card::Column::Status, Expr::value("expired"))
                .col_expr(gift_card::Column::BalanceMinor, Expr::value(0_i64))
                .filter(gift_card::Column::Id.eq(card.id))
                .filter(gift_card::Column::Status.eq("active"))
                .filter(gift_card::Column::BalanceMinor.eq(card.balance_minor))
                .scoped(tenant)
                .exec(&transaction)
                .await?
                .rows_affected;
            if changed == 1 && card.balance_minor > 0 {
                self.queue_posting(
                    &transaction,
                    tenant,
                    format!("customers:gift_card:{}:expire", card.id),
                    "Gift card expired",
                    &card.currency,
                    vec![
                        Line::debit(GIFT_LIABILITY, card.balance_minor),
                        Line::credit(BREAKAGE, card.balance_minor),
                    ],
                )
                .await?;
                done += 1;
            }
            transaction.commit().await?;
        }
        Ok(done)
    }
}
