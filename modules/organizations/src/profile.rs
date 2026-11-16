//! Hồ sơ công khai của nút (bước 34 mục 7, D-200, D-315): website hiện cửa hàng (tên hiển thị, địa
//! chỉ, giờ mở cửa, ngày đặc biệt, nhận hàng tại quầy, ảnh bìa) — chỗ duy nhất giữ địa chỉ và giờ
//! mở cửa của nút. Thương hiệu (logo, liên hệ) là cài đặt kế thừa `storefront.*` (bước 55).

use autoxide_address::api::{Address, AddressInput};
use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use chrono::{DateTime, NaiveDate, NaiveTime, SubsecRound, Utc};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::unit;
use crate::events::{self, Change};
use crate::permissions::PROFILE_MANAGE;
use crate::service::OrgService;

pub(crate) mod entity {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "organizations_profile")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub org_id: Uuid,
        pub public: bool,
        pub position: i32,
        pub display_name: Option<String>,
        pub address_country: Option<String>,
        pub address_province_code: Option<String>,
        pub address_province_name: Option<String>,
        pub address_commune_code: Option<String>,
        pub address_commune_name: Option<String>,
        pub address_line: Option<String>,
        /// JSON: `[OpeningHours]`.
        pub opening_hours: String,
        /// JSON: `[SpecialDay]`.
        pub special_days: String,
        pub pickup: bool,
        pub cover_media_id: Option<Uuid>,
        pub updated_by: Option<Uuid>,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}

/// Opening hours on one day of the week; a day can have several ranges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub(crate) struct OpeningHours {
    /// ISO day of the week: 1 Monday … 7 Sunday.
    #[schema(minimum = 1, maximum = 7, example = 1)]
    pub day: u8,
    /// Local time `HH:MM` (the unit's time zone, setting `organizations.timezone`).
    #[schema(example = "08:00")]
    pub opens: String,
    /// Local time `HH:MM`, after `opens`.
    #[schema(example = "21:30")]
    pub closes: String,
}

/// A day with other hours than usual (holiday, event).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub(crate) struct SpecialDay {
    /// `YYYY-MM-DD`.
    #[schema(example = "2026-02-17")]
    pub date: String,
    /// Closed all day.
    #[serde(default)]
    pub closed: bool,
    /// When open: local time `HH:MM`.
    pub opens: Option<String>,
    pub closes: Option<String>,
    /// Shown to visitors, up to 200 characters.
    pub note: Option<String>,
}

/// Hồ sơ công khai đã ghép với nút.
#[derive(Debug, Clone)]
pub(crate) struct Profile {
    pub unit_id: Uuid,
    pub unit_type: String,
    pub public: bool,
    pub position: i32,
    pub display_name: String,
    pub address: Option<Address>,
    pub opening_hours: Vec<OpeningHours>,
    pub special_days: Vec<SpecialDay>,
    pub pickup: bool,
    pub cover_media_id: Option<Uuid>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// Thay đổi; `None` giữ nguyên, `Some(None)` bỏ đặt.
#[derive(Debug, Default)]
pub(crate) struct ProfileChanges {
    pub public: Option<bool>,
    pub position: Option<i32>,
    pub display_name: Option<Option<String>>,
    pub address: Option<Option<AddressInput>>,
    pub opening_hours: Option<Vec<OpeningHours>>,
    pub special_days: Option<Vec<SpecialDay>>,
    pub pickup: Option<bool>,
    pub cover_media_id: Option<Option<Uuid>>,
}

fn time(raw: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(raw, "%H:%M").ok()
}

/// Kiểm giờ mở cửa và ngày đặc biệt; báo mọi lỗi một lần.
fn check_hours(hours: &[OpeningHours], days: &[SpecialDay]) -> Vec<FieldError> {
    let mut problems = Vec::new();
    if hours.len() > 50 {
        problems.push(FieldError::new("opening_hours", "length", "must list at most 50 ranges"));
    }
    for (index, range) in hours.iter().enumerate() {
        if !(1..=7).contains(&range.day) {
            problems.push(FieldError::new(
                format!("opening_hours[{index}].day"),
                "range",
                "must be 1 (Monday) to 7 (Sunday)",
            ));
        }
        match (time(&range.opens), time(&range.closes)) {
            (Some(opens), Some(closes)) if closes > opens => {}
            _ => problems.push(FieldError::new(
                format!("opening_hours[{index}].closes"),
                "invalid",
                "times must be written HH:MM and closing must be after opening",
            )),
        }
    }
    if days.len() > 366 {
        problems.push(FieldError::new("special_days", "length", "must list at most 366 days"));
    }
    let mut seen = BTreeSet::new();
    for (index, day) in days.iter().enumerate() {
        if NaiveDate::parse_from_str(&day.date, "%Y-%m-%d").is_err() {
            problems.push(FieldError::new(
                format!("special_days[{index}].date"),
                "invalid",
                "must be a date written YYYY-MM-DD",
            ));
        } else if !seen.insert(day.date.as_str()) {
            problems.push(FieldError::new(
                format!("special_days[{index}].date"),
                "duplicate",
                "each day may appear once",
            ));
        }
        if !day.closed {
            let valid =
                match (day.opens.as_deref().and_then(time), day.closes.as_deref().and_then(time)) {
                    (Some(opens), Some(closes)) => closes > opens,
                    _ => false,
                };
            if !valid {
                problems.push(FieldError::new(
                    format!("special_days[{index}].closes"),
                    "invalid",
                    "an open day needs opens and closes written HH:MM, closing after opening",
                ));
            }
        }
        if day.note.as_deref().is_some_and(|note| note.chars().count() > 200) {
            problems.push(FieldError::new(
                format!("special_days[{index}].note"),
                "length",
                "must be at most 200 characters",
            ));
        }
    }
    problems
}

fn invalid_hours(problems: Vec<FieldError>) -> Error {
    let mut error = Error::new(
        ErrorKind::Validation,
        "organizations.invalid_opening_hours",
        "The opening hours are invalid: closing must be after opening, and a special day must not repeat.",
    );
    for problem in problems {
        error = error.with_field(problem);
    }
    error
}

fn address_of(row: &entity::Model) -> Option<Address> {
    let (Some(country), Some(province_code), Some(commune_code)) =
        (&row.address_country, &row.address_province_code, &row.address_commune_code)
    else {
        return None;
    };
    let input = AddressInput::new(
        country,
        province_code,
        commune_code,
        row.address_line.clone().unwrap_or_default(),
    );
    // Tên giữ như lúc ghi (đơn vị đổi tên sau đó: địa chỉ cũ vẫn đúng với lúc ghi).
    serde_json::from_value(json!({
        "country": input.country,
        "province_code": input.province_code,
        "province_name": row.address_province_name.clone().unwrap_or_default(),
        "commune_code": input.commune_code,
        "commune_name": row.address_commune_name.clone().unwrap_or_default(),
        "line": input.line,
    }))
    .ok()
}

/// Danh sách JSON đã kiểm lúc ghi; hỏng thì rỗng (không làm hỏng trang công khai).
fn parse_list<T: serde::de::DeserializeOwned>(raw: &str) -> Vec<T> {
    serde_json::from_str(raw).unwrap_or_default()
}

fn profile_of(unit: &unit::Model, row: Option<&entity::Model>) -> Profile {
    Profile {
        unit_id: unit.id,
        unit_type: unit.unit_type.clone(),
        public: row.is_some_and(|row| row.public),
        position: row.map_or(0, |row| row.position),
        display_name: row
            .and_then(|row| row.display_name.clone())
            .unwrap_or_else(|| unit.name.clone()),
        address: row.and_then(address_of),
        opening_hours: row.map(|row| parse_list(&row.opening_hours)).unwrap_or_default(),
        special_days: row.map(|row| parse_list(&row.special_days)).unwrap_or_default(),
        pickup: row.is_some_and(|row| row.pickup),
        cover_media_id: row.and_then(|row| row.cover_media_id),
        updated_at: row.map(|row| row.updated_at),
    }
}

impl OrgService {
    /// Hồ sơ công khai của nút `unit_id` (cần `organizations.units.read` phủ nút).
    pub(crate) async fn profile(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        unit_id: Uuid,
    ) -> Result<Profile, Error> {
        let unit = self.get(tenant, principal, unit_id).await?;
        let row = entity::Entity::find_by_id((tenant.id().as_uuid(), unit_id))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        Ok(profile_of(&unit, row.as_ref()))
    }

    /// Sửa hồ sơ công khai (cần `organizations.profile.manage` phủ nút; nút phải hoạt động).
    pub(crate) async fn update_profile(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        unit_id: Uuid,
        changes: ProfileChanges,
    ) -> Result<Profile, Error> {
        let units = self.units.as_ref();
        principal.require(&PROFILE_MANAGE)?;
        let current = self.profile(tenant, principal, unit_id).await?;
        let mut problems = Vec::new();
        let hours = changes.opening_hours.clone().unwrap_or_else(|| current.opening_hours.clone());
        let days = changes.special_days.clone().unwrap_or_else(|| current.special_days.clone());
        let hour_problems = check_hours(&hours, &days);
        let display_name = match &changes.display_name {
            Some(Some(name)) => {
                let name = name.trim().to_owned();
                if name.is_empty() || name.chars().count() > 200 {
                    problems.push(
                        FieldError::new("display_name", "length", "must be 1 to {max} characters")
                            .with_param("max", 200),
                    );
                }
                Some(Some(name))
            }
            other => other.clone(),
        };
        // Kiểm địa chỉ (đọc database điều khiển) TRƯỚC transaction ghi.
        let address = match changes.address.clone() {
            Some(Some(input)) => match units {
                Some(units) => match units.validate(input, "address.").await {
                    Ok(address) => Some(Some(address)),
                    Err(fields) => {
                        problems.extend(fields);
                        None
                    }
                },
                None => {
                    problems.push(FieldError::new(
                        "address",
                        "unavailable",
                        "addresses cannot be checked on this server",
                    ));
                    None
                }
            },
            Some(None) => Some(None),
            None => None,
        };
        if !hour_problems.is_empty() {
            let mut error = invalid_hours(hour_problems);
            for problem in problems {
                error = error.with_field(problem);
            }
            return Err(error);
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let now = Utc::now().trunc_subsecs(6);
        let transaction = tenant.database().begin_write().await?;
        principal
            .require_write_at(
                &transaction,
                &PROFILE_MANAGE,
                OrgTarget::new(OrgId::from_uuid(unit_id), None),
            )
            .await?;
        let before = entity::Entity::find_by_id((tenant.id().as_uuid(), unit_id))
            .scoped(tenant)
            .one(&transaction)
            .await?;
        let address = match address {
            Some(found) => found,
            None => before.as_ref().and_then(address_of),
        };
        let row = entity::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(unit_id),
            public: Set(changes.public.unwrap_or(current.public)),
            position: Set(changes.position.unwrap_or(current.position)),
            display_name: Set(match display_name {
                Some(name) => name,
                None => before.as_ref().and_then(|row| row.display_name.clone()),
            }),
            address_country: Set(address.as_ref().map(|a| a.country.clone())),
            address_province_code: Set(address.as_ref().map(|a| a.province_code.clone())),
            address_province_name: Set(address.as_ref().map(|a| a.province_name.clone())),
            address_commune_code: Set(address.as_ref().map(|a| a.commune_code.clone())),
            address_commune_name: Set(address.as_ref().map(|a| a.commune_name.clone())),
            address_line: Set(address.as_ref().map(|a| a.line.clone())),
            opening_hours: Set(serde_json::to_string(&hours).map_err(Error::internal)?),
            special_days: Set(serde_json::to_string(&days).map_err(Error::internal)?),
            pickup: Set(changes.pickup.unwrap_or(current.pickup)),
            cover_media_id: Set(match changes.cover_media_id {
                Some(id) => id,
                None => current.cover_media_id,
            }),
            updated_by: Set(Some(principal.user_id())),
            updated_at: Set(now),
        };
        let conflict = OnConflict::columns([entity::Column::TenantId, entity::Column::OrgId])
            .update_columns([
                entity::Column::Public,
                entity::Column::Position,
                entity::Column::DisplayName,
                entity::Column::AddressCountry,
                entity::Column::AddressProvinceCode,
                entity::Column::AddressProvinceName,
                entity::Column::AddressCommuneCode,
                entity::Column::AddressCommuneName,
                entity::Column::AddressLine,
                entity::Column::OpeningHours,
                entity::Column::SpecialDays,
                entity::Column::Pickup,
                entity::Column::CoverMediaId,
                entity::Column::UpdatedBy,
                entity::Column::UpdatedAt,
            ])
            .to_owned();
        entity::Entity::insert(row)
            .on_conflict(conflict)
            .exec_without_returning(&transaction)
            .await?;
        let after = entity::Entity::find_by_id((tenant.id().as_uuid(), unit_id))
            .scoped(tenant)
            .one(&transaction)
            .await?;
        let describe = |row: Option<&entity::Model>| {
            row.map(|row| {
                json!({
                    "public": row.public,
                    "display_name": row.display_name,
                    "address": address_of(row),
                    "pickup": row.pickup,
                })
            })
        };
        let change = Change { before: describe(before.as_ref()), after: describe(after.as_ref()) };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::unit_subject(unit_id),
                &events::PublicProfileUpdated(change),
            )
            .await?;
        transaction.commit().await?;
        self.profile(tenant, principal, unit_id).await
    }

    /// Các nút công khai của khách đang hoạt động, theo thứ tự khách đặt rồi theo tên (đọc công khai).
    pub(crate) async fn public_stores(
        &self,
        tenant: &TenantContext,
    ) -> Result<Vec<Profile>, Error> {
        let read = tenant.database().read();
        let rows = entity::Entity::find()
            .filter(entity::Column::Public.eq(true))
            .scoped(tenant)
            .order_by_asc(entity::Column::Position)
            .all(read)
            .await?;
        let units = unit::Entity::find()
            .filter(unit::Column::Id.is_in(rows.iter().map(|row| row.org_id)))
            .filter(unit::Column::Status.eq("active"))
            .scoped(tenant)
            .all(read)
            .await?;
        let mut found: Vec<Profile> = rows
            .iter()
            .filter_map(|row| {
                units
                    .iter()
                    .find(|unit| unit.id == row.org_id)
                    .map(|unit| profile_of(unit, Some(row)))
            })
            .collect();
        found.sort_by(|a, b| {
            a.position.cmp(&b.position).then_with(|| a.display_name.cmp(&b.display_name))
        });
        Ok(found)
    }
}

/// Loại đối tượng `organizations.store` trong sổ đối tượng của kernel (D-238): menu của website trỏ
/// tới một cửa hàng; chỉ nút có hồ sơ công khai là công khai.
pub(crate) const STORE: &str = "organizations.store";

pub(crate) struct StoreResolver;

#[async_trait::async_trait]
impl autoxide_kernel::resource::ResourceResolver for StoreResolver {
    async fn resolve(
        &self,
        tenant: &TenantContext,
        id: &str,
    ) -> Result<Option<autoxide_kernel::resource::Resource>, Error> {
        let Ok(id) = id.parse::<Uuid>() else { return Ok(None) };
        let read = tenant.database().read();
        let Some(unit) = unit::Entity::find_by_id(id).scoped(tenant).one(read).await? else {
            return Ok(None);
        };
        let row = entity::Entity::find_by_id((tenant.id().as_uuid(), id))
            .scoped(tenant)
            .one(read)
            .await?;
        let profile = profile_of(&unit, row.as_ref());
        let shown = profile.public && unit.status == "active";
        let mut resource = autoxide_kernel::resource::Resource::new(if shown {
            profile.display_name.clone()
        } else {
            String::new()
        });
        resource.public = shown;
        if shown {
            resource.summary = profile
                .address
                .as_ref()
                .map(|a| format!("{}, {}, {}", a.line, a.commune_name, a.province_name))
                .unwrap_or_default();
            resource.image = profile.cover_media_id;
        }
        Ok(Some(resource))
    }
}
