//! Danh sách lịch đặt: cửa hàng thấy lịch của nút trong phạm vi `bookings.manage` (hay `bookings.read`); nhân viên chỉ có
//! `bookings.staff` thấy lịch của tài nguyên gắn với tài khoản mình; khách thấy lịch của mình.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QuerySelect};
use uuid::Uuid;

use crate::booking::{Booking, STATUSES};
use crate::entity::{booking, resource};
use crate::permissions::{BOOKINGS_MANAGE, BOOKINGS_READ, BOOKINGS_STAFF};
use crate::service::{BookingsService, finish};

/// Bộ lọc danh sách.
#[derive(Debug, Clone, Default)]
pub(crate) struct BookingFilter {
    pub status: Option<String>,
    pub resource_id: Option<Uuid>,
    /// Bắt đầu từ lúc này.
    pub from: Option<DateTime<Utc>>,
    /// Bắt đầu trước lúc này.
    pub to: Option<DateTime<Utc>>,
}

impl BookingFilter {
    fn apply(
        &self,
        select: sea_orm::Select<booking::Entity>,
    ) -> Result<sea_orm::Select<booking::Entity>, Error> {
        let mut problems = Vec::new();
        if self.status.as_deref().is_some_and(|status| !STATUSES.contains(&status)) {
            problems.push(FieldError::new("status", "invalid", "is not a booking status"));
        }
        finish(problems)?;
        let mut select = select;
        if let Some(status) = &self.status {
            select = select.filter(booking::Column::Status.eq(status.clone()));
        }
        if let Some(resource) = self.resource_id {
            select = select.filter(booking::Column::ResourceId.eq(resource));
        }
        if let Some(from) = self.from {
            select = select.filter(booking::Column::StartsAt.gte(from));
        }
        if let Some(to) = self.to {
            select = select.filter(booking::Column::StartsAt.lt(to));
        }
        Ok(select)
    }
}

impl BookingsService {
    /// Lịch đặt trong phạm vi của người gọi.
    pub(crate) async fn list_bookings(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: &BookingFilter,
        page: &PageRequest,
    ) -> Result<Page<Booking>, Error> {
        let read = tenant.database().read();
        // Cửa hàng (`manage`) hay người xem (`read`): mọi lịch trong phạm vi của một trong hai.
        let mut wide = Condition::any();
        let mut viewer = false;
        for permission in [&BOOKINGS_MANAGE, &BOOKINGS_READ] {
            if !principal.grants(permission).is_empty() {
                wide = wide.add(principal.scope_condition(
                    permission,
                    booking::Column::OrgId,
                    None,
                )?);
                viewer = true;
            }
        }
        let select = if !viewer {
            let scope = principal.scope_condition(&BOOKINGS_STAFF, booking::Column::OrgId, None)?;
            let mine: Vec<Uuid> = resource::Entity::find()
                .select_only()
                .column(resource::Column::Id)
                .filter(resource::Column::UserId.eq(principal.user_id()))
                .scoped(tenant)
                .into_tuple()
                .all(read)
                .await?;
            booking::Entity::find().filter(scope).filter(booking::Column::ResourceId.is_in(mine))
        } else {
            booking::Entity::find().filter(wide)
        };
        let select = filter.apply(select.scoped(tenant))?;
        fetch_page(select, booking::Column::Id, |row| row.id, page, read).await
    }
}
