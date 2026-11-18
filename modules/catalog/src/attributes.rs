//! Thuộc tính (xuất xứ, chất liệu…) của khách, thuộc tính bắt buộc theo danh mục có ngày hiệu lực,
//! giá trị thuộc tính của sản phẩm (D-223…D-227). Thuộc tính bắt buộc là **dữ liệu**: nhãn hàng
//! hoá theo NĐ 43/2017/NĐ-CP (sửa bởi NĐ 111/2021/NĐ-CP) nạp bằng `catalog.load_restrictions` —
//! cần đối chiếu văn bản gốc.

use std::collections::{BTreeMap, HashMap};

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use chrono::NaiveDate;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde_json::Value;
use uuid::Uuid;

use crate::entity::{attribute, attribute_option};
use crate::permissions::CATEGORIES_MANAGE;
use crate::service::{CatalogService, check_length, fail_if_any, now, texts_json};

/// Kiểu dữ liệu của thuộc tính.
pub(crate) const DATA_TYPES: [&str; 7] =
    ["text", "integer", "decimal", "boolean", "date", "option", "options"];

/// Thuộc tính mới.
#[derive(Debug, Clone)]
pub(crate) struct NewAttribute {
    pub code: String,
    pub data_type: String,
    /// Số chữ số thập phân khi `decimal` (0–6).
    pub scale: i32,
    pub unit: Option<String>,
    pub names: BTreeMap<String, String>,
    pub filterable: bool,
    pub legal_basis: Option<String>,
}

/// Một giá trị chọn của thuộc tính `option` / `options`.
#[derive(Debug, Clone)]
pub(crate) struct OptionInput {
    pub code: String,
    pub labels: BTreeMap<String, String>,
}

/// Thuộc tính gắn vào danh mục.
#[derive(Debug, Clone)]
pub(crate) struct CategoryAttributeInput {
    pub attribute_id: Uuid,
    pub required: bool,
    pub applies_to_descendants: bool,
    pub effective_from: NaiveDate,
    pub effective_to: Option<NaiveDate>,
    pub legal_basis: Option<String>,
}

/// Một thuộc tính kèm giá trị chọn.
#[derive(Debug, Clone)]
pub(crate) struct AttributeDetail {
    pub attribute: attribute::Model,
    pub options: Vec<attribute_option::Model>,
}

pub(crate) fn attribute_not_found(id: Uuid) -> Error {
    Error::not_found("catalog.attribute_not_found", "There is no attribute `{id}`.")
        .with_param("id", id)
}

fn code_valid(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 64
        && code.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Giá trị đã kiểm của một thuộc tính trên sản phẩm (một dòng; `options` thành nhiều dòng).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Stored {
    pub text: Option<String>,
    pub integer: Option<i64>,
    pub boolean: Option<bool>,
    pub date: Option<NaiveDate>,
    pub option_id: Option<Uuid>,
}

/// Đổi giá trị JSON theo kiểu của thuộc tính. `decimal` lưu số nguyên theo `scale` (chuỗi `"12.5"`
/// với scale 2 → 1250), không qua số thực.
pub(crate) fn parse_value(
    detail: &AttributeDetail,
    value: &Value,
    field: &str,
) -> Result<Vec<Stored>, FieldError> {
    let attribute = &detail.attribute;
    let option = |code: &Value| -> Result<Uuid, FieldError> {
        let code = code
            .as_str()
            .ok_or_else(|| FieldError::new(field, "type", "must be an option code"))?;
        detail
            .options
            .iter()
            .find(|option| option.code == code)
            .map(|option| option.id)
            .ok_or_else(|| FieldError::new(field, "option", "is not an option of the attribute"))
    };
    Ok(match attribute.data_type.as_str() {
        "text" => {
            let text = value
                .as_str()
                .ok_or_else(|| FieldError::new(field, "type", "must be text"))?
                .trim();
            if text.is_empty() || text.chars().count() > 1_000 {
                return Err(FieldError::new(field, "length", "must be 1 to 1000 characters"));
            }
            vec![Stored { text: Some(text.to_owned()), ..Stored::default() }]
        }
        "integer" => vec![Stored {
            integer: Some(
                value
                    .as_i64()
                    .ok_or_else(|| FieldError::new(field, "type", "must be a whole number"))?,
            ),
            ..Stored::default()
        }],
        "decimal" => {
            let raw = match value {
                Value::String(raw) => raw.clone(),
                Value::Number(number) => number.to_string(),
                _ => {
                    return Err(FieldError::new(
                        field,
                        "type",
                        "must be a decimal number written as text, like `12.5`",
                    ));
                }
            };
            let scaled = scale_decimal(&raw, attribute.scale).ok_or_else(|| {
                FieldError::new(
                    field,
                    "type",
                    "must be a decimal number written as text, like `12.5`",
                )
            })?;
            vec![Stored { integer: Some(scaled), ..Stored::default() }]
        }
        "boolean" => vec![Stored {
            boolean: Some(
                value
                    .as_bool()
                    .ok_or_else(|| FieldError::new(field, "type", "must be true or false"))?,
            ),
            ..Stored::default()
        }],
        "date" => {
            let raw = value
                .as_str()
                .ok_or_else(|| FieldError::new(field, "type", "must be a date like 2026-12-31"))?;
            let date = NaiveDate::parse_from_str(raw, "%Y-%m-%d")
                .map_err(|_| FieldError::new(field, "type", "must be a date like 2026-12-31"))?;
            vec![Stored { date: Some(date), ..Stored::default() }]
        }
        "option" => vec![Stored { option_id: Some(option(value)?), ..Stored::default() }],
        _ => {
            let codes = value
                .as_array()
                .ok_or_else(|| FieldError::new(field, "type", "must be a list of option codes"))?;
            let mut out = Vec::new();
            for code in codes {
                let id = option(code)?;
                if !out.iter().any(|stored: &Stored| stored.option_id == Some(id)) {
                    out.push(Stored { option_id: Some(id), ..Stored::default() });
                }
            }
            out
        }
    })
}

/// `"12.5"` với `scale` 2 → 1250. Chữ số thập phân nhiều hơn `scale` thì từ chối (không làm tròn
/// ngầm).
pub(crate) fn scale_decimal(raw: &str, scale: i32) -> Option<i64> {
    let raw = raw.trim();
    let (negative, digits) = match raw.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, raw),
    };
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    let scale = usize::try_from(scale).ok()?;
    if whole.is_empty()
        || fraction.len() > scale
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let padded = format!("{whole}{fraction:0<scale$}");
    let value: i64 = padded.parse().ok()?;
    Some(if negative { -value } else { value })
}

impl CatalogService {
    pub(crate) async fn create_attribute(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewAttribute,
    ) -> Result<AttributeDetail, Error> {
        principal.require(&CATEGORIES_MANAGE)?;
        let mut problems = Vec::new();
        if !code_valid(&input.code) {
            problems.push(FieldError::new(
                "code",
                "format",
                "must be 1 to 64 lower-case letters, digits or `_`",
            ));
        }
        if !DATA_TYPES.contains(&input.data_type.as_str()) {
            problems.push(FieldError::new(
                "data_type",
                "one_of",
                "must be text, integer, decimal, boolean, date, option or options",
            ));
        }
        if !(0..=6).contains(&input.scale) {
            problems.push(FieldError::new("scale", "range", "must be 0 to 6"));
        }
        if input.names.is_empty() {
            problems.push(FieldError::new("names", "required", "name the attribute in a language"));
        }
        for (locale, name) in &input.names {
            self.locale(locale)?;
            check_length(&mut problems, "names", name, 1, 100);
        }
        fail_if_any(problems)?;
        let now = now();
        let row = attribute::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            code: Set(input.code.clone()),
            data_type: Set(input.data_type),
            scale: Set(input.scale),
            unit: Set(input.unit),
            names: Set(texts_json(&input.names)),
            filterable: Set(input.filterable),
            legal_basis: Set(input.legal_basis),
            updated_at: Set(now),
        }
        .insert(tenant.database().write())
        .await
        .map_err(|error| match crate::service::unique_violation(&error) {
            Some(_) => Error::conflict(
                "catalog.attribute_code_taken",
                "There is already an attribute with the code `{code}`.",
            )
            .with_param("code", &input.code),
            None => Error::from(error),
        })?;
        Ok(AttributeDetail { attribute: row, options: Vec::new() })
    }

    pub(crate) async fn list_attributes(
        &self,
        tenant: &TenantContext,
    ) -> Result<Vec<AttributeDetail>, Error> {
        let read = tenant.database().read();
        let rows = attribute::Entity::find()
            .scoped(tenant)
            .order_by_asc(attribute::Column::Code)
            .all(read)
            .await?;
        let options = attribute_option::Entity::find()
            .scoped(tenant)
            .order_by_asc(attribute_option::Column::Position)
            .all(read)
            .await?;
        Ok(rows
            .into_iter()
            .map(|attribute| {
                let options = options
                    .iter()
                    .filter(|option| option.attribute_id == attribute.id)
                    .cloned()
                    .collect();
                AttributeDetail { attribute, options }
            })
            .collect())
    }

    pub(crate) async fn attribute_details<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, AttributeDetail>, Error> {
        let rows = attribute::Entity::find()
            .filter(attribute::Column::Id.is_in(ids.iter().copied()))
            .scoped(tenant)
            .all(connection)
            .await?;
        let options = attribute_option::Entity::find()
            .filter(attribute_option::Column::AttributeId.is_in(ids.iter().copied()))
            .scoped(tenant)
            .order_by_asc(attribute_option::Column::Position)
            .all(connection)
            .await?;
        Ok(rows
            .into_iter()
            .map(|attribute| {
                let options = options
                    .iter()
                    .filter(|option| option.attribute_id == attribute.id)
                    .cloned()
                    .collect();
                (attribute.id, AttributeDetail { attribute, options })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::scale_decimal;

    #[test]
    fn so_thap_phan_luu_nguyen_theo_scale() {
        assert_eq!(scale_decimal("12.5", 2), Some(1250));
        assert_eq!(scale_decimal("-0.25", 2), Some(-25));
        assert_eq!(scale_decimal("7", 3), Some(7000));
        assert_eq!(scale_decimal("1.234", 2), None, "no silent rounding");
        assert_eq!(scale_decimal("1,5", 2), None);
        assert_eq!(scale_decimal(".5", 2), None);
    }
}
