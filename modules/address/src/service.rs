//! Tra và kiểm đơn vị hành chính. Đọc ở database điều khiển (bảng mặt điều khiển, D-253).

use autoxide_kernel::time::{Date, Timestamp};
use std::collections::HashMap;

use autoxide_kernel::db::{Cond, ControlPlane, Find, GlobalRead, Queries as _};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Outbox;
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageRequest};
use autoxide_kernel::tenancy::DatabaseRegistry;

use crate::api::{Address, AddressInput, AdministrativeUnit};
use crate::entity::{Dataset, Unit};

/// Quốc gia có danh mục (đặc tả 34: chỉ Việt Nam).
pub(crate) const COUNTRIES: [&str; 1] = ["VN"];
const LINE_MAX: usize = 300;

pub(crate) struct AddressService {
    /// Handle của database điều khiển: bảng của module là bảng mặt điều khiển.
    pub control: ControlPlane,
    /// Chỉ để lấy khách mặc định (nền tảng) cho event nhập dữ liệu.
    pub databases: DatabaseRegistry,
    pub outbox: Outbox,
}

impl AddressService {
    /// Bộ dữ liệu mới nhất của `country` (bản đang dùng), nếu đã nhập.
    pub(crate) async fn latest(&self, country: &str) -> Result<Option<Dataset>, Error> {
        self.control
            .read()
            .find::<Dataset>()
            .filter(Dataset::COUNTRY.eq(country.to_owned()))
            .order_desc(Dataset::IMPORTED_AT)
            .one()
            .await
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

    /// Một trang tỉnh còn hiệu lực ngày `as_of`, theo mã (thứ tự của văn bản); keyset trong database theo
    /// (mã, id), index `address_unit_page_idx`.
    pub(crate) async fn provinces(
        &self,
        country: &str,
        as_of: Option<&str>,
        page: &PageRequest,
    ) -> Result<Page<AdministrativeUnit>, Error> {
        let country = self.country(country)?;
        let as_of = as_of_date(as_of)?;
        let read = self.control.read();
        let select = read
            .find::<Unit>()
            .filter(Unit::COUNTRY.eq(country))
            .filter(Unit::LEVEL.eq(1))
            .filter(live(&as_of));
        Ok(by_code(select, page).await?.map(describe))
    }

    /// Một trang xã của tỉnh `province`, theo mã; index `address_unit_parent_page_idx`.
    pub(crate) async fn communes(
        &self,
        country: &str,
        province: &str,
        as_of: Option<&str>,
        page: &PageRequest,
    ) -> Result<Page<AdministrativeUnit>, Error> {
        let country = self.country(country)?;
        let as_of = as_of_date(as_of)?;
        let read = self.control.read();
        let found = read
            .find::<Unit>()
            .filter(Unit::COUNTRY.eq(country.clone()))
            .filter(Unit::LEVEL.eq(1))
            .filter(Unit::CODE.eq(province.to_owned()))
            .filter(live(&as_of))
            .exists()
            .await?;
        if !found {
            return Err(unit_not_found(province));
        }
        let select = read
            .find::<Unit>()
            .filter(Unit::COUNTRY.eq(country))
            .filter(Unit::LEVEL.eq(2))
            .filter(Unit::PARENT_CODE.eq(Some(province.to_owned())))
            .filter(live(&as_of));
        Ok(by_code(select, page).await?.map(describe))
    }

    // Hợp đồng `AdministrativeUnits` (cho module khác) trả cả danh sách: gom các trang theo `next_cursor`.
    // Danh mục có cỡ cố định (34 tỉnh, vài trăm xã một tỉnh), nên vài trang 200 dòng là hết.
    async fn every_page(
        &self,
        country: &str,
        province: Option<&str>,
        as_of: Option<&str>,
    ) -> Result<Vec<AdministrativeUnit>, Error> {
        let mut units = Vec::new();
        let mut cursor = None;
        loop {
            let limit = Some(autoxide_kernel::pagination::MAX_LIMIT.to_string());
            let page = PageRequest::try_from(NaturalOrderPageQuery::new(limit, cursor, None))?;
            let page = match province {
                None => self.provinces(country, as_of, &page).await?,
                Some(province) => self.communes(country, province, as_of, &page).await?,
            };
            units.extend(page.data);
            match page.pagination.next_cursor {
                Some(next) => cursor = Some(next),
                None => return Ok(units),
            }
        }
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
        let read = self.control.read();
        let mut select = read.find::<Unit>().filter(Unit::COUNTRY.eq(country)).filter(live(&today));
        if let Some(q) = q.map(str::trim).filter(|q| !q.is_empty()) {
            // Cột lưu dạng đã bỏ dấu: bỏ dấu cả chữ tìm để "ha noi" ra "Hà Nội".
            select = select.filter(Unit::NAME_FOLDED.contains(&autoxide_kernel::search::fold(q)));
        }
        if let Some(province) = province {
            select = select.filter(
                Cond::any()
                    .with(Unit::PARENT_CODE.eq(Some(province.to_owned())))
                    .with(Unit::CODE.eq(province.to_owned())),
            );
        }
        if let Some(level) = level {
            select = select.filter(Unit::LEVEL.eq(level));
        }
        Ok(select.page(Unit::ID, page).await?.map(describe))
    }

    /// Đơn vị `code` (còn hiệu lực hôm nay) kèm đơn vị cấp trên.
    pub(crate) async fn get(
        &self,
        country: &str,
        code: &str,
    ) -> Result<(AdministrativeUnit, Option<AdministrativeUnit>), Error> {
        let country = self.country(country)?;
        let today = as_of_date(None)?;
        let read = self.control.read();
        let found = read
            .find::<Unit>()
            .filter(Unit::COUNTRY.eq(country.clone()))
            .filter(Unit::CODE.eq(code.to_owned()))
            .filter(live(&today))
            .one()
            .await?
            .ok_or_else(|| unit_not_found(code))?;
        let parent = match &found.parent_code {
            Some(parent) => read
                .find::<Unit>()
                .filter(Unit::COUNTRY.eq(country))
                .filter(Unit::CODE.eq(parent.clone()))
                .filter(live(&today))
                .one()
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
        let found: HashMap<String, Unit> = match self
            .control
            .read()
            .find::<Unit>()
            .filter(Unit::COUNTRY.eq(country.clone()))
            .filter(Unit::CODE.is_in(codes.clone()))
            .filter(live(&today))
            .all()
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

#[autoxide_kernel::async_trait]
impl crate::api::AdministrativeUnits for AddressService {
    async fn provinces(
        &self,
        country: &str,
        as_of: Option<&str>,
    ) -> Result<Vec<AdministrativeUnit>, Error> {
        self.every_page(country, None, as_of).await
    }

    async fn communes(
        &self,
        country: &str,
        province: &str,
        as_of: Option<&str>,
    ) -> Result<Vec<AdministrativeUnit>, Error> {
        self.every_page(country, Some(province), as_of).await
    }

    async fn validate(&self, input: AddressInput, field: &str) -> Result<Address, Vec<FieldError>> {
        self.check(input, field).await
    }
}

// Trang theo mã đơn vị rồi id: mã chỉ trùng giữa các phiên bản của cùng đơn vị, mà `live` chỉ để lại một.
async fn by_code(
    select: Find<'_, GlobalRead<'_>, Unit>,
    page: &PageRequest,
) -> Result<Page<Unit>, Error> {
    select.page_by(Unit::CODE, |row| row.code.clone(), Unit::ID, page).await
}

/// Đơn vị còn hiệu lực ngày `as_of` (so chuỗi `YYYY-MM-DD`: đúng thứ tự, không lệch múi giờ).
pub(crate) fn live(as_of: &str) -> Cond<Unit> {
    Cond::all().with(Unit::VALID_FROM.lte(as_of.to_owned())).with(
        Cond::any().with(Unit::VALID_TO.is_null()).with(Unit::VALID_TO.gt(Some(as_of.to_owned()))),
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

pub(crate) fn describe(row: Unit) -> AdministrativeUnit {
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
