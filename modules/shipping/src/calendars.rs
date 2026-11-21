//! Lịch giao (bước 44 mục 6, D-201, D-223…D-227): múi giờ, ngày làm việc trong tuần, ngày riêng
//! (`closed` hay `special_hours` với giờ chốt riêng) — dữ liệu có ngày hiệu lực, khách nhập khi có
//! thông báo chính thức, không viết cứng ngày lễ. Mỗi lịch thuộc một nút; quản lý cần
//! `shipping.settings.manage` ở nút đó. Phương thức gắn lịch qua `calendar_id`.

use autoxide_kernel::time::Date;
use std::collections::BTreeMap;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter,
    QueryOrder, Set,
};
use uuid::Uuid;

use crate::entity::{calendar, calendar_day};
use crate::errors;
use crate::eta::{Calendar, DEFAULT_TIME_ZONE, DayRule, WEEKDAYS};
use crate::permissions::SETTINGS_MANAGE;
use crate::service::{ShippingService, check_length, now};
use crate::settings::{default_org, fail_if_any, readable, writable};

/// Lịch kèm các ngày riêng.
#[derive(Debug, Clone)]
pub(crate) struct CalendarView {
    pub calendar: calendar::Model,
    pub days: Vec<calendar_day::Model>,
}

/// Nội dung lịch gửi lên (tạo: `name` bắt buộc).
#[derive(Debug, Clone, Default)]
pub(crate) struct CalendarFields {
    pub org_id: Option<Uuid>,
    pub name: Option<String>,
    pub time_zone: Option<String>,
    /// Thứ trong tuần làm việc, 1 = thứ Hai … 7 = Chủ nhật.
    pub working_days: Option<Vec<u8>>,
}

/// Một ngày riêng gửi lên.
#[derive(Debug, Clone)]
pub(crate) struct DayInput {
    /// `closed` hay `special_hours`.
    pub kind: String,
    pub cutoff_minute: Option<i32>,
    pub note: String,
}

fn bitmask(days: &[u8]) -> i32 {
    days.iter().filter(|day| (1..=7).contains(*day)).fold(0, |mask, day| mask | (1 << (day - 1)))
}

/// Thứ trong tuần (1 = thứ Hai) từ bitmask.
pub(crate) fn weekdays(mask: i32) -> Vec<u8> {
    (1..=7u8).filter(|day| mask & (1 << (day - 1)) != 0).collect()
}

fn check_fields(problems: &mut Vec<FieldError>, fields: &CalendarFields) {
    if let Some(name) = &fields.name {
        check_length(problems, "name", name.trim(), 1, 200);
    }
    if let Some(zone) = &fields.time_zone
        && autoxide_kernel::time::zone(zone).is_none()
    {
        problems.push(FieldError::new(
            "time_zone",
            "time_zone",
            "must be an IANA time zone like `Asia/Ho_Chi_Minh`",
        ));
    }
    if let Some(days) = &fields.working_days
        && days.iter().any(|day| !(1..=7).contains(day))
    {
        problems.push(FieldError::new(
            "working_days",
            "range",
            "must list days 1 (Monday) to 7 (Sunday)",
        ));
    }
}

impl ShippingService {
    pub(crate) async fn list_calendars(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<calendar::Model>, Error> {
        let condition = Condition::all().add(principal.scope_condition(
            &SETTINGS_MANAGE,
            calendar::Column::OrgId,
            None,
        )?);
        let select = calendar::Entity::find().filter(condition).scoped(tenant);
        fetch_page(select, calendar::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    pub(crate) async fn get_calendar(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<CalendarView, Error> {
        let read = tenant.database().read();
        let row = find_calendar(read, tenant, id).await?;
        readable(principal, read, &SETTINGS_MANAGE, row.org_id, || errors::calendar_not_found(id))
            .await?;
        view_of(read, tenant, row).await
    }

    pub(crate) async fn create_calendar(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        fields: CalendarFields,
    ) -> Result<CalendarView, Error> {
        let mut problems = Vec::new();
        if fields.name.is_none() {
            problems.push(FieldError::new("name", "required", "is required"));
        }
        check_fields(&mut problems, &fields);
        fail_if_any(problems)?;
        let org = match fields.org_id {
            Some(org) => org,
            None => default_org(tenant, principal, &SETTINGS_MANAGE)?.as_uuid(),
        };
        let at = now();
        let transaction = tenant.database().begin_write().await?;
        principal
            .require_write_at(
                &transaction,
                &SETTINGS_MANAGE,
                OrgTarget::new(OrgId::from_uuid(org), None),
            )
            .await?;
        let row = calendar::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(org),
            name: Set(fields.name.unwrap_or_default().trim().to_owned()),
            time_zone: Set(fields.time_zone.unwrap_or_else(|| DEFAULT_TIME_ZONE.to_owned())),
            working_weekdays: Set(fields.working_days.as_deref().map_or(WEEKDAYS, bitmask)),
            created_at: Set(at),
            updated_at: Set(at),
        }
        .insert(&transaction)
        .await?;
        transaction.commit().await?;
        Ok(CalendarView { calendar: row, days: Vec::new() })
    }

    pub(crate) async fn update_calendar(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        fields: CalendarFields,
    ) -> Result<CalendarView, Error> {
        let mut problems = Vec::new();
        if fields.org_id.is_some() {
            problems.push(FieldError::new(
                "org_id",
                "immutable",
                "cannot be changed; create another calendar",
            ));
        }
        check_fields(&mut problems, &fields);
        fail_if_any(problems)?;
        let transaction = tenant.database().begin_write().await?;
        let row = find_calendar(&transaction, tenant, id).await?;
        writable(principal, &transaction, &SETTINGS_MANAGE, row.org_id, || {
            errors::calendar_not_found(id)
        })
        .await?;
        let mut active: calendar::ActiveModel = row.into();
        if let Some(name) = fields.name {
            active.name = Set(name.trim().to_owned());
        }
        if let Some(zone) = fields.time_zone {
            active.time_zone = Set(zone);
        }
        if let Some(days) = fields.working_days {
            active.working_weekdays = Set(bitmask(&days));
        }
        active.updated_at = Set(now());
        let row = active.update(&transaction).await?;
        let view = view_of(&transaction, tenant, row).await?;
        transaction.commit().await?;
        Ok(view)
    }

    /// Đặt (thay) ngày riêng `day` của lịch.
    pub(crate) async fn put_calendar_day(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        day: Date,
        input: DayInput,
    ) -> Result<CalendarView, Error> {
        let mut problems = Vec::new();
        if !matches!(input.kind.as_str(), "closed" | "special_hours") {
            problems.push(FieldError::new("kind", "kind", "must be `closed` or `special_hours`"));
        }
        if input.cutoff_minute.is_some_and(|minute| !(0..1440).contains(&minute)) {
            problems.push(FieldError::new(
                "cutoff_minute",
                "range",
                "must be 0 to 1439 (minutes after midnight)",
            ));
        }
        check_length(&mut problems, "note", &input.note, 0, 300);
        fail_if_any(problems)?;
        let transaction = tenant.database().begin_write().await?;
        let row = find_calendar(&transaction, tenant, id).await?;
        writable(principal, &transaction, &SETTINGS_MANAGE, row.org_id, || {
            errors::calendar_not_found(id)
        })
        .await?;
        calendar_day::Entity::delete_many()
            .filter(calendar_day::Column::CalendarId.eq(id))
            .filter(calendar_day::Column::Day.eq(day))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        calendar_day::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            calendar_id: Set(id),
            day: Set(day),
            kind: Set(input.kind),
            cutoff_minute: Set(input.cutoff_minute),
            note: Set(input.note.trim().to_owned()),
        }
        .insert(&transaction)
        .await?;
        let view = view_of(&transaction, tenant, row).await?;
        transaction.commit().await?;
        Ok(view)
    }

    /// Bỏ ngày riêng `day` (về theo ngày làm việc trong tuần).
    pub(crate) async fn delete_calendar_day(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        day: Date,
    ) -> Result<CalendarView, Error> {
        let transaction = tenant.database().begin_write().await?;
        let row = find_calendar(&transaction, tenant, id).await?;
        writable(principal, &transaction, &SETTINGS_MANAGE, row.org_id, || {
            errors::calendar_not_found(id)
        })
        .await?;
        calendar_day::Entity::delete_many()
            .filter(calendar_day::Column::CalendarId.eq(id))
            .filter(calendar_day::Column::Day.eq(day))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let view = view_of(&transaction, tenant, row).await?;
        transaction.commit().await?;
        Ok(view)
    }

    /// Lịch dùng để tính ngày giao của phương thức (`None` → lịch chuẩn), ngày riêng trong 400 ngày tới.
    pub(crate) async fn calendar_for(
        &self,
        tenant: &TenantContext,
        calendar_id: Option<Uuid>,
        from: Date,
    ) -> Result<Calendar, Error> {
        let Some(id) = calendar_id else { return Ok(Calendar::standard()) };
        let read = tenant.database().read();
        let Some(row) = calendar::Entity::find_by_id(id).scoped(tenant).one(read).await? else {
            return Ok(Calendar::standard());
        };
        let until = from.checked_add_days(400).unwrap_or(from);
        let days = calendar_day::Entity::find()
            .filter(calendar_day::Column::CalendarId.eq(id))
            .filter(calendar_day::Column::Day.gte(from))
            .filter(calendar_day::Column::Day.lte(until))
            .scoped(tenant)
            .all(read)
            .await?;
        let mut rules = BTreeMap::new();
        for day in days {
            let rule = if day.kind == "closed" {
                DayRule::Closed
            } else {
                DayRule::Special(day.cutoff_minute)
            };
            rules.insert(day.day, rule);
        }
        Ok(Calendar {
            time_zone: autoxide_kernel::time::zone(&row.time_zone)
                .or_else(|| autoxide_kernel::time::zone(DEFAULT_TIME_ZONE))
                .unwrap_or(jiff::tz::TimeZone::UTC),
            working_weekdays: row.working_weekdays,
            days: rules,
        })
    }
}

async fn find_calendar<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<calendar::Model, Error> {
    calendar::Entity::find_by_id(id)
        .scoped(tenant)
        .one(connection)
        .await?
        .ok_or_else(|| errors::calendar_not_found(id))
}

async fn view_of<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    calendar: calendar::Model,
) -> Result<CalendarView, Error> {
    let days = calendar_day::Entity::find()
        .filter(calendar_day::Column::CalendarId.eq(calendar.id))
        .scoped(tenant)
        .order_by_asc(calendar_day::Column::Day)
        .all(connection)
        .await?;
    Ok(CalendarView { calendar, days })
}
