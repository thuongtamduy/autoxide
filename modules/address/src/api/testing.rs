//! Danh mục hành chính giả dùng chung cho test của module khác (feature `testing`, chỉ khai ở
//! `[dev-dependencies]`; quy ước D-20261122-module-test-fakes).
//!
//! Giữ vài tỉnh và xã trong bộ nhớ; `validate` trả lỗi như module `address` thật
//! (`address.commune_not_in_province`, `address.unit_expired`, gắn tiền tố `field`). Không xét ngày
//! hiệu lực của `provinces` / `communes` (trả mọi đơn vị còn hiệu lực đã khai).
//!
//! ```ignore
//! let units = Arc::new(FakeAdministrativeUnits::sample());
//! let app = TestApp::builder(test).provide::<dyn AdministrativeUnits>(units)…;
//! ```

use autoxide_kernel::error::{Error, FieldError};

use super::{Address, AddressInput, AdministrativeUnit, AdministrativeUnits};

/// Danh mục hành chính giả. Công bố bằng
/// `TestApp::builder(…).provide::<dyn AdministrativeUnits>(Arc::new(units))`.
#[derive(Debug, Clone, Default)]
pub struct FakeAdministrativeUnits {
    provinces: Vec<AdministrativeUnit>,
    communes: Vec<AdministrativeUnit>,
}

fn unit(code: &str, name: &str, kind: &str, parent: Option<&str>) -> AdministrativeUnit {
    AdministrativeUnit {
        code: code.to_owned(),
        name: name.to_owned(),
        kind: kind.to_owned(),
        level: if parent.is_some() { 2 } else { 1 },
        parent_code: parent.map(str::to_owned),
        valid_from: "2025-07-01".to_owned(),
        valid_to: None,
    }
}

impl FakeAdministrativeUnits {
    /// Danh mục rỗng: mọi địa chỉ đều sai.
    pub fn new() -> Self {
        FakeAdministrativeUnits::default()
    }

    /// Mẫu dùng ở test của core: tỉnh 01 (Thanh pho Ha Noi) có xã 00004 (Phuong Ba Dinh) và xã
    /// 00001 đã hết hiệu lực; tỉnh 79 (Thanh pho Ho Chi Minh) có xã 26734 (Phuong Sai Gon).
    pub fn sample() -> Self {
        FakeAdministrativeUnits::new()
            .province("01", "Thanh pho Ha Noi")
            .commune("00004", "Phuong Ba Dinh", "01")
            .expired_commune("00001", "Phuong Phuc Xa", "01")
            .province("79", "Thanh pho Ho Chi Minh")
            .commune("26734", "Phuong Sai Gon", "79")
    }

    /// Thêm tỉnh `code`.
    #[must_use]
    pub fn province(mut self, code: &str, name: &str) -> Self {
        self.provinces.push(unit(code, name, "province", None));
        self
    }

    /// Thêm xã `code` thuộc tỉnh `province`.
    #[must_use]
    pub fn commune(mut self, code: &str, name: &str, province: &str) -> Self {
        self.communes.push(unit(code, name, "commune", Some(province)));
        self
    }

    /// Thêm xã đã hết hiệu lực (sáp nhập): `validate` trả `address.unit_expired`.
    #[must_use]
    pub fn expired_commune(mut self, code: &str, name: &str, province: &str) -> Self {
        let mut commune = unit(code, name, "commune", Some(province));
        commune.valid_from = "2008-08-01".to_owned();
        commune.valid_to = Some("2025-07-01".to_owned());
        self.communes.push(commune);
        self
    }
}

#[async_trait::async_trait]
impl AdministrativeUnits for FakeAdministrativeUnits {
    async fn provinces(&self, _: &str, _: Option<&str>) -> Result<Vec<AdministrativeUnit>, Error> {
        Ok(self.provinces.clone())
    }

    async fn communes(
        &self,
        _: &str,
        province: &str,
        _: Option<&str>,
    ) -> Result<Vec<AdministrativeUnit>, Error> {
        if !self.provinces.iter().any(|unit| unit.code == province) {
            return Err(Error::not_found(
                "address.unit_not_found",
                "No administrative unit has this code.",
            ));
        }
        Ok(self
            .communes
            .iter()
            .filter(|unit| unit.parent_code.as_deref() == Some(province) && unit.valid_to.is_none())
            .cloned()
            .collect())
    }

    async fn validate(&self, input: AddressInput, field: &str) -> Result<Address, Vec<FieldError>> {
        let commune = self.communes.iter().find(|unit| unit.code == input.commune_code);
        if let Some(commune) = commune
            && commune.valid_to.is_some()
        {
            return Err(vec![FieldError::new(
                format!("{field}commune_code"),
                "address.unit_expired",
                "is no longer in force",
            )]);
        }
        let province = self.provinces.iter().find(|unit| unit.code == input.province_code);
        match (province, commune) {
            (Some(province), Some(commune))
                if commune.parent_code.as_deref() == Some(province.code.as_str()) =>
            {
                Ok(Address {
                    country: input.country,
                    province_code: province.code.clone(),
                    province_name: province.name.clone(),
                    commune_code: commune.code.clone(),
                    commune_name: commune.name.clone(),
                    line: input.line,
                })
            }
            _ => Err(vec![FieldError::new(
                format!("{field}commune_code"),
                "address.commune_not_in_province",
                "commune `{commune}` is not in province `{province}`",
            )]),
        }
    }
}
