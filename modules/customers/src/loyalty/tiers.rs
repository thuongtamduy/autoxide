//! Hạng thành viên (bước 47 mục 3): hạng cao nhất mà chi tiêu 12 tháng trượt (tiền hàng của đơn hoàn
//! tất trong 365 ngày, trừ hoàn) đạt ngưỡng. Xét lại ngay khi tổng chi đổi (trong transaction của
//! thay đổi) và hằng ngày (đơn cũ rơi khỏi cửa sổ 12 tháng). Đổi hạng phát `customers.tier_changed`
//! và đánh giá lại nhóm (quy tắc `tier_in`).

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use chrono::Duration;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect};
use serde_json::json;
use uuid::Uuid;

use super::current_tiers;
use crate::entity::{order_fact, profile};
use crate::events::{Change, TierChanged};
use crate::profiles::subject;
use crate::service::{CustomersService, now};

impl CustomersService {
    /// Chi tiêu 12 tháng trượt của hồ sơ theo tiền tệ `currency`.
    async fn spent_last_year<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        profile_id: Uuid,
        currency: &str,
    ) -> Result<i64, Error> {
        let rows: Vec<(i64, i64)> = order_fact::Entity::find()
            .filter(order_fact::Column::ProfileId.eq(profile_id))
            .filter(order_fact::Column::Status.eq("completed"))
            .filter(order_fact::Column::Currency.eq(currency))
            .filter(order_fact::Column::CompletedAt.gte(now() - Duration::days(365)))
            .scoped(tenant)
            .select_only()
            .column(order_fact::Column::NetMinor)
            .column(order_fact::Column::RefundedMinor)
            .into_tuple()
            .all(connection)
            .await?;
        Ok(rows
            .into_iter()
            .map(|(net, refunded)| (net - refunded).max(0))
            .fold(0_i64, i64::saturating_add))
    }

    /// Xét lại hạng của hồ sơ; trả `true` khi hạng đổi.
    pub(crate) async fn evaluate_tier<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        profile_id: Uuid,
    ) -> Result<bool, Error> {
        let tiers = current_tiers(connection, tenant, now()).await?;
        let Some(row) =
            profile::Entity::find_by_id(profile_id).scoped(tenant).one(connection).await?
        else {
            return Ok(false);
        };
        let mut chosen = None;
        for tier in &tiers {
            let spent =
                self.spent_last_year(connection, tenant, profile_id, &tier.currency).await?;
            if spent >= tier.threshold_minor {
                chosen = Some(tier.code.clone());
                break;
            }
        }
        if chosen == row.tier_code {
            return Ok(false);
        }
        profile::Entity::update_many()
            .col_expr(profile::Column::TierCode, Expr::value(chosen.clone()))
            .col_expr(profile::Column::UpdatedAt, Expr::value(now()))
            .filter(profile::Column::Id.eq(profile_id))
            .scoped(tenant)
            .exec(connection)
            .await?;
        self.outbox
            .record(
                connection,
                tenant,
                Actor::system(),
                Some(subject(profile_id)),
                &TierChanged(Change {
                    before: Some(json!({"profile_id": profile_id, "tier": row.tier_code})),
                    after: Some(json!({"profile_id": profile_id, "tier": chosen})),
                }),
            )
            .await?;
        Ok(true)
    }

    /// Xét lại hạng mọi hồ sơ có hạng hay có đơn trong 13 tháng (job hằng ngày).
    pub(crate) async fn evaluate_tiers(&self, tenant: &TenantContext) -> Result<u64, Error> {
        let ids: Vec<Uuid> = profile::Entity::find()
            .filter(
                sea_orm::Condition::any()
                    .add(profile::Column::TierCode.is_not_null())
                    .add(profile::Column::LastOrderAt.gte(now() - Duration::days(400))),
            )
            .scoped(tenant)
            .select_only()
            .column(profile::Column::Id)
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        let mut changed = 0;
        for id in ids {
            let transaction = tenant.database().begin_write().await?;
            if self.evaluate_tier(&transaction, tenant, id).await? {
                self.schedule_evaluation(&transaction, tenant, id).await?;
                changed += 1;
            }
            transaction.commit().await?;
        }
        Ok(changed)
    }
}
