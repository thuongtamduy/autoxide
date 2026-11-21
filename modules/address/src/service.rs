//! Tra và kiểm đơn vị hành chính. Đọc ở database điều khiển (bảng mặt điều khiển, D-253).

use autoxide_kernel::time::{Date, Timestamp};
use std::collections::HashMap;

use autoxide_kernel::database::Database;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Outbox;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::DatabaseRegistry;
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder};

use crate::api::{Address, AddressInput, AdministrativeUnit};
use crate::entity::{dataset, unit};

/// Quốc gia có danh mục (đặc tả 34: chỉ Việt Nam).
pub(crate) const COUNTRIES: [&str; 1] = ["VN"];
const LINE_MAX: usize = 300;

pub(crate) struct AddressService {
    pub databases: DatabaseRegistry,
    pub outbox: Outbox,
}

impl AddressService {
    pub(crate) fn control(&self) -> &Database {
        self.databases.control()
    }

    /// Bộ dữ liệu mới nhất của `country` (bản đang dùng), nếu đã nhập.
    pub(crate) async fn latest(&self, country: &str) -> Result<Option<dataset::Model>, Error> {
        Ok(dataset::Entity::find()
            .filter(dataset::Column::Country.eq(country))
            .order_by_desc(dataset::Column::ImportedAt)
            .one(self.control().read())
            .await?)
    }

    /// Phiên bản dữ liệu của `country` cho `ETag`: đổi mỗi lần nhập bộ mới.
    pub(crate) async fn version(&self, country: &str) -> Result<Option<String>, Error> {
        Ok(self
            .latest(country)
            .await?
            .map(|row| format!("\"{}\"", &row.sha256[..16.min(row.sha256.len())])))
    }

    /// Mã quốc gia đã chuẩn hoá, có danh mục; không thì `address.country_unsupported`.
    pub(crate) fn country(&self, raw: &str) -> Result<String, Error> {
        let country = raw.trim().to_ascii_uppercase();
        if COUNTRIES.contains(&country.as_str()) {
            Ok(country)
        } else {
            Err(country_unsupported(&country))
        }
    }

    pub(crate) async fn provinces(
        &self,
        country: &str,
        as_of: Option<&str>,
    ) -> Result<Vec<AdministrativeUnit>, Error> {
        let country = self.country(country)?;
        let as_of = as_of_date(as_of)?;
        let rows = unit::Entity::find()
            .filter(unit::Column::Country.eq(&country))
            .filter(unit::Column::Level.eq(1))
            .filter(live(&as_of))
            .order_by_asc(unit::Column::Code)
            .all(self.control().read())
            .await?;
        Ok(rows.into_iter().map(describe).collect())
    }

    pub(crate) async fn communes(
        &self,
        country: &str,
        province: &str,
        as_of: Option<&str>,
    ) -> Result<Vec<AdministrativeUnit>, Error> {
        let country = self.country(country)?;
        let as_of = as_of_date(as_of)?;
        let read = self.control().read();
        let found = unit::Entity::find()
            .filter(unit::Column::Country.eq(&country))
            .filter(unit::Column::Level.eq(1))
            .filter(unit::Column::Code.eq(province))
            .filter(live(&as_of))
            .one(read)
            .await?;
        if found.is_none() {
            return Err(unit_not_found(province));
        }
        let rows = unit::Entity::find()
            .filter(unit::Column::Country.eq(&country))
            .filter(unit::Column::Level.eq(2))
            .filter(unit::Column::ParentCode.eq(province))
            .filter(live(&as_of))
            .order_by_asc(unit::Column::Code)
            .all(read)
            .await?;
        Ok(rows.into_iter().map(describe).collect())
    }

    /// Tìm đơn vị còn hiệu lực theo tên, không dấu ("ha noi" ra "Hà Nội").
    pub(crate) async fn search(
        &self,
        country: &str,
        q: Option<&str>,
        province: Option<&str>,
        level: Option<i32>,
        page: &PageRequest,
    ) -> Result<Page<AdministrativeUnit>, Error> {
        let country = self.country(country)?;
        let today = as_of_date(None)?;
        let mut select =
            unit::Entity::find().filter(unit::Column::Country.eq(&country)).filter(live(&today));
        if let Some(q) = q.map(str::trim).filter(|q| !q.is_empty()) {
            select = select.filter(autoxide_kernel::search::contains(unit::Column::NameFolded, q));
        }
        if let Some(province) = province {
            select = select.filter(
                Condition::any()
                    .add(unit::Column::ParentCode.eq(province))
                    .add(unit::Column::Code.eq(province)),
            );
        }
        if let Some(level) = level {
            select = select.filter(unit::Column::Level.eq(level));
        }
        let page =
            fetch_page(select, unit::Column::Id, |row| row.id, page, self.control().read()).await?;
        Ok(page.map(describe))
    }

    /// Đơn vị `code` (còn hiệu lực hôm nay) kèm đơn vị cấp trên.
    pub(crate) async fn get(
        &self,
        country: &str,
        code: &str,
    ) -> Result<(AdministrativeUnit, Option<AdministrativeUnit>), Error> {
        let country = self.country(country)?;
        let today = as_of_date(None)?;
        let read = self.control().read();
        let found = unit::Entity::find()
            .filter(unit::Column::Country.eq(&country))
            .filter(unit::Column::Code.eq(code))
            .filter(live(&today))
            .one(read)
            .await?
            .ok_or_else(|| unit_not_found(code))?;
        let parent = match &found.parent_code {
            Some(parent) => unit::Entity::find()
                .filter(unit::Column::Country.eq(&country))
                .filter(unit::Column::Code.eq(parent))
                .filter(live(&today))
                .one(read)
                .await?
                .map(describe),
            None => None,
        };
        Ok((describe(found), parent))
    }

    /// Kiểm địa chỉ (xã còn hiệu lực, thuộc đúng tỉnh) và điền tên. Lỗi gắn tiền tố `field`.
    pub(crate) async fn check(
        &self,
        input: AddressInput,
        field: &str,
    ) -> Result<Address, Vec<FieldError>> {
        let name = |suffix: &str| format!("{field}{suffix}");
        let mut problems = Vec::new();
        let line = input.line.trim().to_owned();
        if line.is_empty() || line.chars().count() > LINE_MAX {
            problems.push(
                FieldError::new(name("line"), "length", "must be 1 to {max} characters")
                    .with_param("max", LINE_MAX),
            );
        }
        let country = match self.country(&input.country) {
            Ok(country) => country,
            Err(_) => {
                problems.push(
                    FieldError::new(
                        name("country"),
                        "address.country_unsupported",
                        "there is no list of administrative units for `{country}`",
                    )
                    .with_param("country", input.country.trim()),
                );
                return Err(problems);
            }
        };
        let today = match as_of_date(None) {
            Ok(today) => today,
            Err(_) => return Err(problems),
        };
        let codes = [input.province_code.trim().to_owned(), input.commune_code.trim().to_owned()];
        let found: HashMap<String, unit::Model> = match unit::Entity::find()
            .filter(unit::Column::Country.eq(&country))
            .filter(unit::Column::Code.is_in(codes.clone()))
            .filter(live(&today))
            .all(self.control().read())
            .await
        {
            Ok(rows) => rows.into_iter().map(|row| (row.code.clone(), row)).collect(),
            Err(error) => {
                tracing::warn!(error = %error, "cannot read administrative units");
                problems.push(FieldError::new(
                    name("province_code"),
                    "unavailable",
                    "cannot be checked now; retry",
                ));
                return Err(problems);
            }
        };
        let province = found.get(&codes[0]).filter(|row| row.level == 1);
        let commune = found.get(&codes[1]).filter(|row| row.level == 2);
        if province.is_none() {
            problems.push(
                FieldError::new(
                    name("province_code"),
                    "address.unit_not_found",
                    "there is no province `{code}`",
                )
                .with_param("code", &codes[0]),
            );
        }
        match commune {
            None => problems.push(
                FieldError::new(
                    name("commune_code"),
                    "address.unit_not_found",
                    "there is no commune `{code}`",
                )
                .with_param("code", &codes[1]),
            ),
            Some(commune)
                if commune.parent_code.as_deref() != Some(codes[0].as_str())
                    && province.is_some() =>
            {
                problems.push(
                    FieldError::new(
                        name("commune_code"),
                        "address.commune_not_in_province",
                        "commune `{commune}` is not in province `{province}`",
                    )
                    .with_param("commune", &codes[1])
                    .with_param("province", &codes[0]),
                );
            }
            Some(_) => {}
        }
        match (province, commune) {
            (Some(province), Some(commune)) if problems.is_empty() => Ok(Address {
                country,
                province_code: province.code.clone(),
                province_name: province.name.clone(),
                commune_code: commune.code.clone(),
                commune_name: commune.name.clone(),
                line,
            }),
            _ => Err(problems),
        }
    }
}

#[async_trait::async_trait]
impl crate::api::AdministrativeUnits for AddressService {
    async fn provinces(
        &self,
        country: &str,
        as_of: Option<&str>,
    ) -> Result<Vec<AdministrativeUnit>, Error> {
        AddressService::provinces(self, country, as_of).await
    }

    async fn communes(
        &self,
        country: &str,
        province: &str,
        as_of: Option<&str>,
    ) -> Result<Vec<AdministrativeUnit>, Error> {
        AddressService::communes(self, country, province, as_of).await
    }

    async fn validate(&self, input: AddressInput, field: &str) -> Result<Address, Vec<FieldError>> {
        self.check(input, field).await
    }
}

/// Đơn vị còn hiệu lực ngày `as_of` (so chuỗi `YYYY-MM-DD`: đúng thứ tự, không lệch múi giờ).
pub(crate) fn live(as_of: &str) -> Condition {
    Condition::all().add(unit::Column::ValidFrom.lte(as_of)).add(
        Condition::any().add(unit::Column::ValidTo.is_null()).add(unit::Column::ValidTo.gt(as_of)),
    )
}

/// `as_of` đã kiểm (`YYYY-MM-DD`); không có thì hôm nay (giờ Việt Nam không khác ngày UTC quá
/// một ngày; danh mục đổi theo ngày, lệch vài giờ quanh nửa đêm chấp nhận được).
pub(crate) fn as_of_date(raw: Option<&str>) -> Result<String, Error> {
    match raw.map(str::trim).filter(|raw| !raw.is_empty()) {
        None => Ok(Timestamp::now().date_utc().format("%Y-%m-%d").to_string()),
        Some(raw) => Date::parse_from_str(raw, "%Y-%m-%d")
            .map(|date| date.format("%Y-%m-%d").to_string())
            .map_err(|_| {
                Error::validation(vec![FieldError::new(
                    "as_of",
                    "invalid",
                    "must be a date written YYYY-MM-DD",
                )])
            }),
    }
}

pub(crate) fn describe(row: unit::Model) -> AdministrativeUnit {
    AdministrativeUnit {
        code: row.code,
        name: row.name,
        kind: row.kind,
        level: row.level,
        parent_code: row.parent_code,
        valid_from: row.valid_from,
        valid_to: row.valid_to,
    }
}

pub(crate) fn unit_not_found(code: &str) -> Error {
    Error::not_found(
        "address.unit_not_found",
        "There is no administrative unit with code `{code}`.",
    )
    .with_param("code", code)
}

pub(crate) fn country_unsupported(country: &str) -> Error {
    Error::new(
        autoxide_kernel::error::ErrorKind::Validation,
        "address.country_unsupported",
        "There is no list of administrative units for country `{country}` yet.",
    )
    .with_param("country", country)
}
