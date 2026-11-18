//! Chặng 1 — rút số liệu ở database NGUỒN: bộ nhận event gọi `FactSource::extract`, tính ngày theo
//! múi giờ của nút, ghi `reporting_local_facts` (trùng thì bỏ qua — nhận lại event không đếm hai
//! lần, D-054). Không chạm database khác: database của khách gốc đang bảo trì không làm hỏng event.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::report::{Fact, FactSource};
use autoxide_kernel::tenancy::{OrgId, TenantContext, root_of};
use chrono::{DateTime, NaiveDate, SubsecRound, Utc};
use sea_orm::sea_query::OnConflict;
use sea_orm::{EntityTrait, Set};
use uuid::Uuid;

use crate::entity::local_fact;
use crate::service::Reporting;

/// Bộ nhận event của một nguồn số liệu.
pub(crate) struct FactHandler {
    pub source: FactSource,
    pub service: Arc<Reporting>,
}

#[async_trait::async_trait]
impl EventHandler for FactHandler {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let facts = self.source.extract(event)?;
        self.service.record_local(&event.tenant, event.id, facts).await
    }
}

impl Reporting {
    /// Ghi các số liệu của event `event_id` vào kho cục bộ của khách `tenant`. Gọi lại với cùng
    /// event không thêm dòng nào.
    pub(crate) async fn record_local(
        &self,
        tenant: &TenantContext,
        event_id: Uuid,
        facts: Vec<Fact>,
    ) -> Result<(), Error> {
        if facts.is_empty() {
            return Ok(());
        }
        // Múi giờ đọc TRƯỚC transaction (đọc cài đặt của nút là một lời gọi sang module khác).
        let mut zones: HashMap<OrgId, Option<chrono_tz::Tz>> = HashMap::new();
        let mut rows = Vec::with_capacity(facts.len());
        let at = Utc::now().trunc_subsecs(6);
        for (index, fact) in facts.into_iter().enumerate() {
            let org = fact.org_id.unwrap_or_else(|| root_of(tenant.id()));
            let zone = match zones.get(&org) {
                Some(zone) => *zone,
                None => {
                    let zone = self.timezone(tenant, org).await;
                    zones.insert(org, zone);
                    zone
                }
            };
            let dimensions: BTreeMap<&str, &str> =
                fact.dimensions.iter().map(|(name, value)| (*name, value.as_str())).collect();
            rows.push(local_fact::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                source_event_id: Set(event_id),
                fact_index: Set(i32::try_from(index).unwrap_or(i32::MAX)),
                kind: Set(fact.kind.to_owned()),
                org_id: Set(org.as_uuid()),
                occurred_at: Set(fact.occurred_at.trunc_subsecs(6)),
                local_day: Set(local_day(fact.occurred_at, zone)),
                amount_minor: Set(fact.amount.map(|amount| amount.minor())),
                amount_currency: Set(fact.amount.map(|amount| amount.currency().code().to_owned())),
                quantity: Set(fact.quantity),
                dimensions: Set(serde_json::to_string(&dimensions).map_err(Error::internal)?),
                created_at: Set(at),
                forwarded_at: Set(None),
            });
        }
        local_fact::Entity::insert_many(rows)
            .on_conflict(
                OnConflict::columns([
                    local_fact::Column::TenantId,
                    local_fact::Column::SourceEventId,
                    local_fact::Column::FactIndex,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(tenant.database().write())
            .await?;
        Ok(())
    }

    /// Múi giờ của nút `org` (`organizations.timezone`, kế thừa theo cây); không đọc được thì
    /// `None` (UTC).
    async fn timezone(&self, tenant: &TenantContext, org: OrgId) -> Option<chrono_tz::Tz> {
        let directory = self.directory.as_ref()?;
        let found = match directory.setting(tenant, org, "organizations.timezone").await {
            Ok(found) => found,
            // Nút đã bị xoá hay không thuộc khách: dùng múi giờ của nút gốc.
            Err(_) if org != root_of(tenant.id()) => {
                match directory
                    .setting(tenant, root_of(tenant.id()), "organizations.timezone")
                    .await
                {
                    Ok(found) => found,
                    Err(error) => {
                        tracing::warn!(%error, "cannot read the time zone of a unit; using UTC");
                        None
                    }
                }
            }
            Err(error) => {
                tracing::warn!(%error, "cannot read the time zone of a unit; using UTC");
                None
            }
        };
        found?.value?.as_str()?.parse::<chrono_tz::Tz>().ok()
    }
}

/// Ngày của `at` theo múi giờ `zone` (không có: UTC).
pub(crate) fn local_day(at: DateTime<Utc>, zone: Option<chrono_tz::Tz>) -> NaiveDate {
    match zone {
        Some(zone) => at.with_timezone(&zone).date_naive(),
        None => at.date_naive(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ngay_theo_mui_gio_cua_nut() {
        // 23:30 ngày 31/10 giờ Việt Nam là 16:30 UTC cùng ngày; 00:30 ngày 1/11 giờ Việt Nam là
        // 17:30 UTC ngày 31/10.
        let late = DateTime::parse_from_rfc3339("2026-10-31T16:30:00Z").unwrap().to_utc();
        let after = DateTime::parse_from_rfc3339("2026-10-31T17:30:00Z").unwrap().to_utc();
        let vietnam = Some(chrono_tz::Asia::Ho_Chi_Minh);
        assert_eq!(local_day(late, vietnam), NaiveDate::from_ymd_opt(2026, 10, 31).unwrap());
        assert_eq!(local_day(after, vietnam), NaiveDate::from_ymd_opt(2026, 11, 1).unwrap());
        assert_eq!(local_day(after, None), NaiveDate::from_ymd_opt(2026, 10, 31).unwrap());
    }
}
