//! Nạp lịch từ database rồi gọi hàm thuần [`crate::availability`]: giờ trống công khai và kiểm
//! "trong giờ làm" khi đặt chỗ.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::collections::{BTreeMap, HashMap};

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use jiff::tz::TimeZone;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::availability::{self, Opening, Plan, Shape, Span, Weekly};
use crate::entity::{resource, session, slot};
use crate::service::{BookingsService, finish, now};
use crate::settings::Policy;
use crate::setup::ServiceView;

/// Khoảng giờ trống hỏi tối đa một lần.
pub(crate) const MAX_RANGE_DAYS: i64 = 31;

/// Câu hỏi giờ trống.
#[derive(Debug, Clone)]
pub(crate) struct AvailabilityQuery {
    pub service_id: Uuid,
    /// Chỉ nhân viên / phòng của nút này; múi giờ của nút này.
    pub org_id: Option<Uuid>,
    pub resource_id: Option<Uuid>,
    pub from: Timestamp,
    pub to: Timestamp,
}

/// Giờ trống cùng những gì khách cần biết trước khi đặt (giá, cọc, chính sách huỷ).
#[derive(Debug, Clone)]
pub(crate) struct Availability {
    pub service: ServiceView,
    pub zone: TimeZone,
    pub policy: Policy,
    pub openings: Vec<Opening>,
}

pub(crate) fn shape_of(service: &ServiceView) -> Shape {
    Shape {
        duration: i64::from(service.row.duration_minutes),
        buffer_before: i64::from(service.row.buffer_before_minutes),
        buffer_after: i64::from(service.row.buffer_after_minutes),
    }
}

/// Giờ trống của một (hay mọi) nhân viên — dùng chung cho kiểm khi đặt.
pub(crate) struct Loaded {
    pub performers: Vec<Plan>,
    pub required: Vec<Plan>,
    pub sessions: BTreeMap<(Uuid, Timestamp), i32>,
}

impl BookingsService {
    /// Lịch của các tài nguyên `ids` trong [from, to) (đã nới thêm hai đầu cho đệm và ca qua đêm).
    pub(crate) async fn plans<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        resources: &[resource::Model],
        from: Timestamp,
        to: Timestamp,
    ) -> Result<HashMap<Uuid, Plan>, Error> {
        let ids: Vec<Uuid> = resources.iter().map(|row| row.id).collect();
        let (wide_from, wide_to) =
            (from - SignedDuration::from_hours(24 * 2), to + SignedDuration::from_hours(24 * 2));
        let weekly = Self::schedule_rows(connection, tenant, &ids).await?;
        let exceptions = Self::exception_rows(connection, tenant, &ids, wide_from, wide_to).await?;
        let busy = slot::Entity::find()
            .filter(slot::Column::ResourceId.is_in(ids.iter().copied()))
            .filter(slot::Column::SlotStart.gte(wide_from))
            .filter(slot::Column::SlotStart.lt(wide_to))
            .scoped(tenant)
            .all(connection)
            .await?;
        let mut plans: HashMap<Uuid, Plan> = resources
            .iter()
            .map(|row| {
                let mut plan = Plan { resource_id: row.id, ..Plan::default() };
                plan.weekly = weekly
                    .iter()
                    .filter(|range| range.resource_id == row.id)
                    .map(|range| Weekly {
                        weekday: u32::try_from(range.weekday).unwrap_or(0),
                        start_minute: i64::from(range.start_minute),
                        end_minute: i64::from(range.end_minute),
                    })
                    .collect();
                // Phòng / thiết bị không có lịch thì luôn mở (D-20261118-step60-bookings mục 5).
                plan.always_open = row.kind != "staff" && plan.weekly.is_empty();
                (row.id, plan)
            })
            .collect();
        for found in exceptions {
            if let Some(plan) = plans.get_mut(&found.resource_id) {
                let span = Span::new(found.starts_at, found.ends_at);
                if found.kind == "open" { plan.open.push(span) } else { plan.closed.push(span) }
            }
        }
        for cell in busy {
            if let Some(plan) = plans.get_mut(&cell.resource_id) {
                plan.busy.insert(cell.slot_start);
            }
        }
        Ok(plans)
    }

    /// Nạp lịch của nhân viên làm được `service` (tuỳ chọn chỉ `resource_id`, chỉ nút `org`) và
    /// tài nguyên dịch vụ cần.
    pub(crate) async fn load<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        service: &ServiceView,
        org: Option<Uuid>,
        resource_id: Option<Uuid>,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Loaded, Error> {
        let ids: Vec<Uuid> = service.performers.iter().chain(&service.required).copied().collect();
        let rows = resource::Entity::find()
            .filter(resource::Column::Id.is_in(ids.iter().copied()))
            .filter(resource::Column::Active.eq(true))
            .scoped(tenant)
            .all(connection)
            .await?;
        let mut plans = Self::plans(connection, tenant, &rows, from, to).await?;
        let by_id: HashMap<Uuid, &resource::Model> = rows.iter().map(|row| (row.id, row)).collect();
        let mut performers: Vec<Plan> = service
            .performers
            .iter()
            .filter(|id| resource_id.is_none_or(|wanted| wanted == **id))
            .filter(|id| by_id.get(id).is_some_and(|row| org.is_none_or(|org| row.org_id == org)))
            .filter_map(|id| plans.get(id).cloned())
            .collect();
        performers.sort_by_key(|plan| plan.resource_id);
        let required = service.required.iter().filter_map(|id| plans.remove(id)).collect();
        let open = session::Entity::find()
            .filter(session::Column::ServiceId.eq(service.row.id))
            .filter(session::Column::StartsAt.gte(from - SignedDuration::from_hours(24)))
            .filter(session::Column::StartsAt.lt(to + SignedDuration::from_hours(24)))
            .scoped(tenant)
            .all(connection)
            .await?;
        let sessions = open
            .into_iter()
            .map(|row| ((row.resource_id, row.starts_at), row.capacity - row.booked))
            .collect();
        Ok(Loaded { performers, required, sessions })
    }

    /// Giờ trống công khai của một dịch vụ đang nhận đặt.
    pub(crate) async fn availability(
        &self,
        tenant: &TenantContext,
        query: AvailabilityQuery,
    ) -> Result<Availability, Error> {
        let mut problems = Vec::new();
        if query.to <= query.from {
            problems.push(FieldError::new("to", "range", "must be after `from`"));
        } else if query.to - query.from > SignedDuration::from_hours(24 * MAX_RANGE_DAYS) {
            problems.push(
                FieldError::new("to", "range", "ask for at most {days} days at a time")
                    .with_param("days", MAX_RANGE_DAYS),
            );
        }
        finish(problems)?;
        let read = tenant.database().read();
        let service = Self::service_view(read, tenant, query.service_id).await?;
        if !service.row.active {
            return Err(crate::setup::service_not_found(query.service_id));
        }
        let org = OrgId::from_uuid(query.org_id.unwrap_or(service.row.org_id));
        let zone = self.settings.timezone(tenant, org).await?;
        let policy = self.settings.policy(tenant, org).await?;
        let loaded = Self::load(
            read,
            tenant,
            &service,
            query.org_id,
            query.resource_id,
            query.from,
            query.to,
        )
        .await?;
        let openings = availability::openings(&availability::Query {
            zone: zone.clone(),
            shape: shape_of(&service),
            capacity: service.row.capacity,
            from: query.from,
            to: query.to,
            now: now(),
            performers: &loaded.performers,
            required: &loaded.required,
            sessions: &loaded.sessions,
        });
        Ok(Availability { service, zone, policy, openings })
    }
}
