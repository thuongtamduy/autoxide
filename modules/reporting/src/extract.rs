//! Chặng 1 — rút số liệu ở database NGUỒN: bộ nhận event gọi `FactSource::extract`, tính ngày theo
//! múi giờ của nút, ghi `reporting_local_facts` (trùng thì bỏ qua — nhận lại event không đếm hai
//! lần, D-054). Không chạm database khác: database của khách gốc đang bảo trì không làm hỏng event.

use autoxide_kernel::time::{Date, Timestamp};
use jiff::tz::TimeZone;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::report::{Fact, FactSource};
use autoxide_kernel::tenancy::{OrgId, TenantContext, root_of};
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
        let mut zones: HashMap<OrgId, Option<TimeZone>> = HashMap::new();
        let mut rows = Vec::with_capacity(facts.len());
        let at = Timestamp::now();
        for (index, fact) in facts.into_iter().enumerate() {
            let org = fact.org_id.unwrap_or_else(|| root_of(tenant.id()));
            let zone = match zones.get(&org) {
                Some(zone) => zone.clone(),
                None => {
                    let zone = self.timezone(tenant, org).await;
                    zones.insert(org, zone.clone());
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
                occurred_at: Set(fact.occurred_at.truncate(jiff::Unit::Microsecond)),
                local_day: Set(local_day(fact.occurred_at, zone.as_ref())),
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
    async fn timezone(&self, tenant: &TenantContext, org: OrgId) -> Option<TimeZone> {
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
        autoxide_kernel::time::zone(found?.value?.as_str()?)
    }
}

/// Ngày của `at` theo múi giờ `zone` (không có: UTC).
pub(crate) fn local_day(at: Timestamp, zone: Option<&TimeZone>) -> Date {
    match zone {
        Some(zone) => at.date_in(zone),
        None => at.date_utc(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ngay_theo_mui_gio_cua_nut() {
        // 23:30 ngày 31/10 giờ Việt Nam là 16:30 UTC cùng ngày; 00:30 ngày 1/11 giờ Việt Nam là
        // 17:30 UTC ngày 31/10.
        let late = Timestamp::parse_rfc3339("2026-10-31T16:30:00Z").unwrap();
        let after = Timestamp::parse_rfc3339("2026-10-31T17:30:00Z").unwrap();
        let vietnam = autoxide_kernel::time::zone("Asia/Ho_Chi_Minh");
        let vietnam = vietnam.as_ref();
        assert_eq!(local_day(late, vietnam), Date::new(2026, 10, 31).unwrap());
        assert_eq!(local_day(after, vietnam), Date::new(2026, 11, 1).unwrap());
        assert_eq!(local_day(after, None), Date::new(2026, 10, 31).unwrap());
    }
}
