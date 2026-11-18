//! Trường tự định nghĩa trên bản ghi (bước 51 P19, D-235): khách thêm ô dữ liệu của riêng mình cho một
//! loại bản ghi ("ngày sinh con" cho khách hàng, "người phụ trách" cho bài viết) mà không sửa code.
//!
//! - **Module chọn tham gia** bằng [`Module::custom_field_targets`](crate::module::Module): loại bản
//!   ghi của sổ đối tượng, permission đọc / ghi, mức hiện hỗ trợ.
//! - **Định nghĩa** ở `kernel_custom_fields` (cấp khách hàng): `key` (không đổi), `data_type`, nhãn /
//!   trợ giúp theo ngôn ngữ, bắt buộc, lựa chọn, ràng buộc độ dài / khoảng giá trị (không nhận biểu
//!   thức chính quy do khách viết), mặc định, mức hiện, lọc được, dữ liệu cá nhân, thứ tự, bật / tắt.
//!   API `GET/POST /api/v1/kernel/custom-fields?target=`, `GET/PATCH/DELETE …/{id}`
//!   (`kernel.custom_fields.manage`; đọc định nghĩa thì cần permission đọc của loại bản ghi).
//! - **Giá trị nằm trên bản ghi** của module (cột `custom_fields`, JSON trong `text`). Service gọi
//!   [`CustomFields::validate`]: kiểm theo định nghĩa đang bật, gom mọi lỗi một lần (đường dẫn
//!   `custom_fields.<key>`), trả giá trị đã chuẩn hoá.
//! - **Lọc** trường `filterable`: module thêm [`index_migration`] (bảng `<module>_custom_index`), ghi
//!   index cùng transaction với bản ghi ([`CustomFields::write_index`]); danh sách nhận
//!   `cf.<key>=<toán tử>:<giá trị>` (`eq`, `in`, `gte`, `lte`, `prefix`) và kernel dựng điều kiện `id IN
//!   (subquery)` ([`CustomFields::condition`]) — câu SQL thường, như nhau trên hai database.
//! - **Đổi định nghĩa:** nhãn, lựa chọn, ràng buộc đổi được; `key` không bao giờ; `data_type` chỉ khi
//!   chưa bản ghi nào có giá trị (`kernel.custom_field_type_locked`, 409). Xoá là tắt (`active=false`).

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, NaiveDate, SubsecRound, Utc};
use sea_orm::sea_query::{Alias, Condition, Expr, Order, Query as SqlQuery};
use sea_orm::{ColumnTrait, ConnectionTrait};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::access::{Permission, Principal};
use crate::error::{Error, ErrorKind, FieldError};
use crate::http::ValidJson;
use crate::migration::prelude::*;
use crate::response::{Created, Data, Problem};
use crate::tenancy::TenantContext;

const TABLE: &str = "kernel_custom_fields";
/// Tối đa chừng này trường mỗi loại bản ghi (trần máy chủ).
const MAX_PER_TARGET: usize = 100;
/// Giá trị của mọi trường trên một bản ghi tối đa chừng này byte (JSON).
const MAX_BYTES: usize = 16_384;

/// Định nghĩa trường tự định nghĩa của khách.
pub const CUSTOM_FIELDS_MANAGE: Permission = Permission::new(
    "kernel.custom_fields.manage",
    "Define the tenant's own fields on records (custom fields).",
);

/// Kiểu dữ liệu nhận được.
pub const DATA_TYPES: [&str; 14] = [
    "text",
    "long_text",
    "integer",
    "decimal",
    "boolean",
    "date",
    "datetime",
    "option",
    "options",
    "url",
    "email",
    "phone",
    "media",
    "reference",
];

/// Một loại bản ghi module cho gắn trường tự định nghĩa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct CustomFieldTarget {
    /// Loại bản ghi của sổ đối tượng, vd `content.entry`.
    pub target: &'static str,
    pub read_permission: Permission,
    pub write_permission: Permission,
    /// Mức hiện loại này hỗ trợ: `internal` (luôn có), `public`, `buyer_input`.
    pub visibilities: &'static [&'static str],
}

impl CustomFieldTarget {
    pub const fn new(target: &'static str, read: Permission, write: Permission) -> Self {
        CustomFieldTarget {
            target,
            read_permission: read,
            write_permission: write,
            visibilities: &["internal"],
        }
    }

    pub const fn visibilities(mut self, visibilities: &'static [&'static str]) -> Self {
        self.visibilities = visibilities;
        self
    }
}

/// A custom field of the tenant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct CustomFieldView {
    pub id: Uuid,
    /// The record type, e.g. `content.entry`.
    pub target: String,
    /// `snake_case`, never changes: the key in the records' `custom_fields`.
    pub key: String,
    /// `text`, `long_text`, `integer`, `decimal`, `boolean`, `date`, `datetime`, `option`,
    /// `options`, `url`, `email`, `phone`, `media` or `reference`.
    pub data_type: String,
    /// Labels by language.
    pub labels: BTreeMap<String, String>,
    /// Help texts by language.
    pub help: BTreeMap<String, String>,
    pub required: bool,
    /// Allowed values of `option` / `options`.
    pub options: Vec<String>,
    /// Digits after the decimal point of `decimal` (0 to 6).
    pub scale: Option<i32>,
    /// Shortest text, smallest number (or date as `YYYY-MM-DD`).
    pub min: Option<String>,
    /// Longest text, largest number (or date).
    pub max: Option<String>,
    /// With `reference`: the record type it points to.
    pub reference_kind: Option<String>,
    /// Any JSON value of the field's type.
    #[schema(value_type = Option<serde_json::Value>)]
    pub default: Option<Value>,
    /// `internal` (staff only), `public` (in public reads) or `buyer_input` (end users fill it).
    pub visibility: String,
    /// Lists filter on it (`cf.<key>=`).
    pub filterable: bool,
    /// Personal data: goes into personal-data exports and erasure.
    pub personal_data: bool,
    pub position: i32,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Sổ loại bản ghi tham gia, điền khi khởi động; tra định nghĩa từ database. Clone rẻ.
#[derive(Clone, Default)]
pub struct CustomFields {
    targets: Arc<OnceLock<BTreeMap<&'static str, CustomFieldTarget>>>,
}

fn is_key(key: &str) -> bool {
    (1..=64).contains(&key.len())
        && key.starts_with(|c: char| c.is_ascii_lowercase())
        && key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

impl CustomFields {
    pub(crate) fn fill(&self, targets: Vec<CustomFieldTarget>) {
        let _ =
            self.targets.set(targets.into_iter().map(|target| (target.target, target)).collect());
    }

    fn target(&self, name: &str) -> Option<CustomFieldTarget> {
        self.targets.get().and_then(|targets| targets.get(name)).copied()
    }

    /// Khoá các trường đang bật ở mức `public` của loại `target`: phần được đọc công khai.
    pub async fn public_keys(
        &self,
        tenant: &TenantContext,
        target: &str,
    ) -> Result<std::collections::BTreeSet<String>, Error> {
        Ok(self
            .definitions(tenant, target)
            .await?
            .into_iter()
            .filter(|field| field.visibility == "public")
            .map(|field| field.key)
            .collect())
    }

    /// Phần công khai của giá trị đã lưu (`stored`, JSON trong `text`): chỉ khoá trong `keys`.
    pub fn pick_public(
        stored: Option<&str>,
        keys: &std::collections::BTreeSet<String>,
    ) -> Map<String, Value> {
        stored
            .and_then(|text| serde_json::from_str::<Map<String, Value>>(text).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|(key, _)| keys.contains(key))
            .collect()
    }

    /// Định nghĩa đang bật của loại `target`, theo thứ tự.
    pub async fn definitions(
        &self,
        tenant: &TenantContext,
        target: &str,
    ) -> Result<Vec<CustomFieldView>, Error> {
        Ok(list(tenant, Some(target)).await?.into_iter().filter(|field| field.active).collect())
    }

    /// Kiểm `incoming` (giá trị người dùng gửi, `null` xoá một trường) gộp vào `current` (giá trị
    /// đang có) theo định nghĩa đang bật của `target`. Gom mọi lỗi một lần (`custom_fields.<key>`),
    /// trả bản đã chuẩn hoá để lưu. Trường đã tắt giữ nguyên giá trị cũ, không nhận giá trị mới.
    pub async fn validate(
        &self,
        tenant: &TenantContext,
        target: &str,
        incoming: Option<&Map<String, Value>>,
        current: Option<&Map<String, Value>>,
        creating: bool,
    ) -> Result<Map<String, Value>, Error> {
        let fields = self.definitions(tenant, target).await?;
        let mut merged = current.cloned().unwrap_or_default();
        let mut problems = Vec::new();
        if let Some(incoming) = incoming {
            for (key, value) in incoming {
                let path = format!("custom_fields.{key}");
                let Some(field) = fields.iter().find(|field| &field.key == key) else {
                    problems.push(FieldError::new(
                        path,
                        "unknown",
                        "is not a field of this record",
                    ));
                    continue;
                };
                if value.is_null() {
                    merged.remove(key);
                    continue;
                }
                match normalize(field, value) {
                    Ok(normal) => {
                        merged.insert(key.clone(), normal);
                    }
                    Err(error) => problems.push(error.with_field(path)),
                }
            }
        }
        for field in &fields {
            if creating
                && !merged.contains_key(&field.key)
                && let Some(default) = &field.default
            {
                merged.insert(field.key.clone(), default.clone());
            }
            if field.required && !merged.contains_key(&field.key) {
                problems.push(FieldError::new(
                    format!("custom_fields.{}", field.key),
                    "required",
                    "is required",
                ));
            }
        }
        if serde_json::to_string(&merged).map_or(0, |text| text.len()) > MAX_BYTES {
            problems.push(FieldError::new(
                "custom_fields",
                "invalid",
                "is too large (at most 16 KiB in all)",
            ));
        }
        if problems.is_empty() {
            Ok(merged)
        } else {
            let mut error = Error::new(
                ErrorKind::Validation,
                "kernel.custom_field_invalid",
                "Some custom fields are invalid; see `errors`.",
            );
            for problem in problems {
                error = error.with_field(problem);
            }
            Err(error)
        }
    }

    /// Ghi index lọc của bản ghi `record` (module `module`) theo giá trị đã lưu: gọi cùng
    /// transaction với bản ghi. Chỉ trường `filterable`.
    pub async fn write_index<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        module: &str,
        target: &str,
        record: Uuid,
        values: &Map<String, Value>,
    ) -> Result<(), Error> {
        let table = format!("{module}_custom_index");
        let delete = SqlQuery::delete()
            .from_table(Alias::new(&table))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
            .and_where(Expr::col(Alias::new("record_id")).eq(record))
            .to_owned();
        connection.execute(&delete).await?;
        let fields = list_on(connection, tenant, Some(target)).await?;
        for field in fields.iter().filter(|field| field.filterable && field.active) {
            let Some(value) = values.get(&field.key) else { continue };
            for (text, number) in index_values(field, value) {
                let insert = SqlQuery::insert()
                    .into_table(Alias::new(&table))
                    .columns([
                        "id",
                        "tenant_id",
                        "record_id",
                        "field_id",
                        "value_text",
                        "value_int",
                    ])
                    .values_panic([
                        Uuid::now_v7().into(),
                        tenant.id().as_uuid().into(),
                        record.into(),
                        field.id.into(),
                        text.into(),
                        number.into(),
                    ])
                    .to_owned();
                connection.execute(&insert).await?;
            }
        }
        Ok(())
    }

    /// Điều kiện lọc `cf.<key>=<toán tử>:<giá trị>` trên cột ID bản ghi `column` của module
    /// `module`. Lỗi khi trường không có, không lọc được, hay giá trị sai.
    #[allow(clippy::too_many_arguments)]
    pub async fn condition<Col: ColumnTrait>(
        &self,
        tenant: &TenantContext,
        module: &str,
        target: &str,
        column: Col,
        key: &str,
        filter: &str,
    ) -> Result<Condition, Error> {
        let path = format!("cf.{key}");
        let fields = self.definitions(tenant, target).await?;
        let field = fields.iter().find(|field| field.key == key).ok_or_else(|| {
            Error::validation(vec![FieldError::new(
                path.clone(),
                "unknown",
                "is not a field of this record",
            )])
        })?;
        if !field.filterable {
            return Err(Error::new(
                ErrorKind::Validation,
                "kernel.custom_field_not_filterable",
                "Field `{key}` is not filterable.",
            )
            .with_param("key", key.to_owned()));
        }
        let (operator, raw) = filter.split_once(':').unwrap_or(("eq", filter));
        let bad = || {
            Error::validation(vec![FieldError::new(
                path.clone(),
                "invalid",
                "must be `<eq|in|gte|lte|prefix>:<value>` with a value of the field's type",
            )])
        };
        let values: Vec<&str> = if operator == "in" { raw.split(',').collect() } else { vec![raw] };
        let mut keys = Vec::new();
        for value in &values {
            let typed = match field.data_type.as_str() {
                "integer" | "decimal" => serde_json::from_str::<Value>(value).map_err(|_| bad())?,
                "boolean" => Value::Bool(*value == "true"),
                _ => Value::String((*value).to_owned()),
            };
            let normal = normalize(field, &typed).map_err(|_| bad())?;
            keys.extend(index_values(field, &normal));
        }
        let table = format!("{module}_custom_index");
        let mut subquery = SqlQuery::select();
        subquery
            .column(Alias::new("record_id"))
            .from(Alias::new(&table))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
            .and_where(Expr::col(Alias::new("field_id")).eq(field.id));
        let numeric = keys.first().is_some_and(|(_, number)| number.is_some());
        match (operator, numeric) {
            ("eq" | "in", true) => {
                subquery.and_where(
                    Expr::col(Alias::new("value_int")).is_in(keys.iter().filter_map(|(_, n)| *n)),
                );
            }
            ("eq" | "in", false) => {
                subquery.and_where(
                    Expr::col(Alias::new("value_text"))
                        .is_in(keys.iter().filter_map(|(t, _)| t.clone())),
                );
            }
            ("gte", true) => {
                subquery.and_where(Expr::col(Alias::new("value_int")).gte(keys[0].1));
            }
            ("lte", true) => {
                subquery.and_where(Expr::col(Alias::new("value_int")).lte(keys[0].1));
            }
            ("prefix", false) => {
                let prefix = keys.first().and_then(|(text, _)| text.clone()).unwrap_or_default();
                subquery.and_where(
                    Expr::col(Alias::new("value_text"))
                        .like(format!("{}%", prefix.replace(['%', '_'], ""))),
                );
            }
            _ => return Err(bad()),
        }
        Ok(Condition::all().add(column.in_subquery(subquery.to_owned())))
    }
}

trait WithField {
    fn with_field(self, field: String) -> FieldError;
}

impl WithField for FieldError {
    fn with_field(mut self, field: String) -> FieldError {
        self.field = field;
        self
    }
}

/// Giá trị đã kiểm theo kiểu của `field`.
fn normalize(field: &CustomFieldView, value: &Value) -> Result<Value, FieldError> {
    let text_bounds = |text: &str| -> Result<(), FieldError> {
        let length = <i64 as TryFrom<usize>>::try_from(text.chars().count()).unwrap_or(i64::MAX);
        let min = field.min.as_deref().and_then(|min| min.parse::<i64>().ok()).unwrap_or(0);
        let max = field.max.as_deref().and_then(|max| max.parse::<i64>().ok()).unwrap_or(10_000);
        if (min..=max).contains(&length) {
            Ok(())
        } else {
            Err(FieldError::new("", "invalid", "has a length out of the field's range"))
        }
    };
    let number_bounds = |number: f64| -> Result<(), FieldError> {
        let below = field
            .min
            .as_deref()
            .and_then(|min| min.parse::<f64>().ok())
            .is_some_and(|min| number < min);
        let above = field
            .max
            .as_deref()
            .and_then(|max| max.parse::<f64>().ok())
            .is_some_and(|max| number > max);
        if below || above {
            Err(FieldError::new("", "invalid", "is out of the field's range"))
        } else {
            Ok(())
        }
    };
    let text = || {
        value.as_str().map(str::trim).ok_or_else(|| FieldError::new("", "invalid", "must be text"))
    };
    match field.data_type.as_str() {
        "text" | "long_text" | "phone" => {
            let text = text()?;
            if field.data_type == "text" && text.contains('\n') {
                return Err(FieldError::new("", "invalid", "must be one line"));
            }
            text_bounds(text)?;
            Ok(Value::String(text.to_owned()))
        }
        "integer" => {
            let number = value
                .as_i64()
                .ok_or_else(|| FieldError::new("", "invalid", "must be a whole number"))?;
            #[allow(clippy::cast_precision_loss)]
            number_bounds(number as f64)?;
            Ok(Value::from(number))
        }
        "decimal" => {
            // Lưu dạng chữ cố định số chữ số sau dấu chấm: không mất chính xác như `f64`.
            let raw = match value {
                Value::String(text) => text.trim().to_owned(),
                Value::Number(number) => number.to_string(),
                _ => return Err(FieldError::new("", "invalid", "must be a number")),
            };
            let parsed: f64 =
                raw.parse().map_err(|_| FieldError::new("", "invalid", "must be a number"))?;
            number_bounds(parsed)?;
            let scale = usize::try_from(field.scale.unwrap_or(2)).unwrap_or(2);
            let decimals = raw.split_once('.').map_or(0, |(_, fraction)| fraction.len());
            if decimals > scale {
                return Err(FieldError::new(
                    "",
                    "invalid",
                    "has too many digits after the decimal point",
                ));
            }
            Ok(Value::String(format!("{parsed:.scale$}")))
        }
        "boolean" => value
            .as_bool()
            .map(Value::Bool)
            .ok_or_else(|| FieldError::new("", "invalid", "must be `true` or `false`")),
        "date" => {
            let text = text()?;
            NaiveDate::parse_from_str(text, "%Y-%m-%d")
                .map_err(|_| FieldError::new("", "invalid", "must be a date `YYYY-MM-DD`"))?;
            Ok(Value::String(text.to_owned()))
        }
        "datetime" => {
            let at = DateTime::parse_from_rfc3339(text()?)
                .map_err(|_| FieldError::new("", "invalid", "must be an RFC 3339 time"))?;
            Ok(Value::String(at.with_timezone(&Utc).to_rfc3339()))
        }
        "option" => {
            let text = text()?;
            if field.options.iter().any(|option| option == text) {
                Ok(Value::String(text.to_owned()))
            } else {
                Err(FieldError::new("", "invalid", "is not one of the field's options"))
            }
        }
        "options" => {
            let list = value
                .as_array()
                .ok_or_else(|| FieldError::new("", "invalid", "must be a list of options"))?;
            let mut chosen = Vec::new();
            for item in list {
                match item.as_str() {
                    Some(option) if field.options.iter().any(|known| known == option) => {
                        chosen.push(option.to_owned())
                    }
                    _ => {
                        return Err(FieldError::new(
                            "",
                            "invalid",
                            "holds a value that is not one of the field's options",
                        ));
                    }
                }
            }
            chosen.sort();
            chosen.dedup();
            Ok(Value::from(chosen))
        }
        "url" => {
            let text = text()?;
            if !(text.starts_with("https://") || text.starts_with("http://")) || text.len() > 2000 {
                return Err(FieldError::new("", "invalid", "must be an `http(s)://` address"));
            }
            Ok(Value::String(text.to_owned()))
        }
        "email" => {
            let text = text()?;
            if !crate::email::is_valid(text) {
                return Err(FieldError::new("", "invalid", "must be a mail address"));
            }
            Ok(Value::String(crate::email::normalize(text)))
        }
        "media" => {
            let id = text()?
                .parse::<Uuid>()
                .map_err(|_| FieldError::new("", "invalid", "must be a media item id"))?;
            Ok(Value::String(id.to_string()))
        }
        "reference" => {
            let id = text()?;
            if id.is_empty() || id.len() > 200 {
                return Err(FieldError::new("", "invalid", "must be the id of a record"));
            }
            Ok(Value::String(id.to_owned()))
        }
        _ => Err(FieldError::new("", "invalid", "has an unknown type")),
    }
}

/// Số thực đã làm tròn thành số nguyên (giá trị thập phân đã nhân theo `scale`, luôn trong khoảng
/// của `i64` với trường hợp thường gặp; vượt khoảng thì bão hoà).
#[allow(clippy::cast_possible_truncation)]
fn whole(number: f64) -> i64 {
    number as i64
}

/// Dòng index của một giá trị: (chữ thường bỏ dấu, số nguyên). Ngày, số, boolean về `value_int`.
fn index_values(field: &CustomFieldView, value: &Value) -> Vec<(Option<String>, Option<i64>)> {
    match field.data_type.as_str() {
        "integer" => vec![(None, value.as_i64())],
        "decimal" => {
            let scale = field.scale.unwrap_or(2);
            let scaled = value
                .as_str()
                .and_then(|text| text.parse::<f64>().ok())
                .map(|number| (number * 10f64.powi(scale)).round());
            vec![(None, scaled.map(whole))]
        }
        "boolean" => vec![(None, value.as_bool().map(i64::from))],
        "date" => vec![(
            None,
            value
                .as_str()
                .and_then(|text| NaiveDate::parse_from_str(text, "%Y-%m-%d").ok())
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .map(|at| at.and_utc().timestamp()),
        )],
        "datetime" => vec![(
            None,
            value
                .as_str()
                .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
                .map(|at| at.timestamp()),
        )],
        "options" => value
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|item| (Some(crate::search::fold(item)), None))
                    .collect()
            })
            .unwrap_or_default(),
        _ => vec![(value.as_str().map(crate::search::fold), None)],
    }
}

/// Migration tạo bảng index lọc `<module>_custom_index` của module `module` (như `outbox_migration`).
pub fn index_migration(name: &'static str, module: &'static str) -> Box<dyn MigrationTrait> {
    Box::new(IndexMigration { name, module })
}

struct IndexMigration {
    name: &'static str,
    module: &'static str,
}

impl MigrationName for IndexMigration {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for IndexMigration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = format!("{}_custom_index", self.module);
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(&table))
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("record_id"))
                    .col(schema::uuid("field_id"))
                    .col(schema::string_len_null("value_text", 400))
                    .col(schema::big_integer_null("value_int"))
                    .to_owned(),
            )
            .await?;
        for (suffix, column) in
            [("text", "value_text"), ("int", "value_int"), ("record", "record_id")]
        {
            let mut index = Index::create();
            index
                .name(format!("{table}_{suffix}_idx"))
                .table(Alias::new(&table))
                .col(Alias::new("tenant_id"));
            if column != "record_id" {
                index.col(Alias::new("field_id"));
            }
            index.col(Alias::new(column));
            manager.create_index(index.to_owned()).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop().table(Alias::new(format!("{}_custom_index", self.module))).to_owned(),
            )
            .await
    }
}

const COLUMNS: [&str; 20] = [
    "id",
    "target",
    "key",
    "data_type",
    "labels",
    "help",
    "required",
    "options",
    "scale",
    "min_value",
    "max_value",
    "reference_kind",
    "default_value",
    "visibility",
    "filterable",
    "personal_data",
    "position",
    "active",
    "created_at",
    "updated_at",
];

fn view(row: &sea_orm::QueryResult) -> Result<CustomFieldView, Error> {
    let json = |column: &str| -> Result<Option<Value>, Error> {
        let raw: Option<String> = row.try_get("", column)?;
        Ok(raw.and_then(|raw| serde_json::from_str(&raw).ok()))
    };
    let map = |value: Option<Value>| -> BTreeMap<String, String> {
        value.and_then(|value| serde_json::from_value(value).ok()).unwrap_or_default()
    };
    Ok(CustomFieldView {
        id: row.try_get("", "id")?,
        target: row.try_get("", "target")?,
        key: row.try_get("", "key")?,
        data_type: row.try_get("", "data_type")?,
        labels: map(json("labels")?),
        help: map(json("help")?),
        required: row.try_get("", "required")?,
        options: json("options")?
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default(),
        scale: row.try_get("", "scale")?,
        min: row.try_get("", "min_value")?,
        max: row.try_get("", "max_value")?,
        reference_kind: row.try_get("", "reference_kind")?,
        default: json("default_value")?,
        visibility: row.try_get("", "visibility")?,
        filterable: row.try_get("", "filterable")?,
        personal_data: row.try_get("", "personal_data")?,
        position: row.try_get("", "position")?,
        active: row.try_get("", "active")?,
        created_at: row.try_get("", "created_at")?,
        updated_at: row.try_get("", "updated_at")?,
    })
}

async fn list(tenant: &TenantContext, target: Option<&str>) -> Result<Vec<CustomFieldView>, Error> {
    list_on(tenant.database().read(), tenant, target).await
}

async fn list_on<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    target: Option<&str>,
) -> Result<Vec<CustomFieldView>, Error> {
    let mut select = SqlQuery::select();
    select
        .columns(COLUMNS)
        .from(Alias::new(TABLE))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .order_by(Alias::new("position"), Order::Asc)
        .order_by(Alias::new("key"), Order::Asc);
    if let Some(target) = target {
        select.and_where(Expr::col(Alias::new("target")).eq(target));
    }
    let rows = connection.query_all(&select).await?;
    rows.iter().map(view).collect()
}

/// A field to define.
#[derive(Debug, Clone, Default, Deserialize, ToSchema, validator::Validate)]
pub(crate) struct NewCustomField {
    /// The record type, e.g. `content.entry`.
    pub target: String,
    /// `snake_case`, 1 to 64 characters; never changes.
    pub key: String,
    /// One of the types listed on `data_type` of the answer.
    pub data_type: String,
    #[serde(flatten)]
    pub settings: FieldSettings,
}

/// The changeable settings of a field; omitted fields stay (or take their default).
#[derive(Debug, Clone, Default, Deserialize, ToSchema, validator::Validate)]
pub(crate) struct FieldSettings {
    pub labels: Option<BTreeMap<String, String>>,
    pub help: Option<BTreeMap<String, String>>,
    pub required: Option<bool>,
    pub options: Option<Vec<String>>,
    pub scale: Option<i32>,
    pub min: Option<String>,
    pub max: Option<String>,
    pub reference_kind: Option<String>,
    /// Any JSON value of the field's type.
    #[schema(value_type = Option<serde_json::Value>)]
    pub default: Option<Value>,
    pub visibility: Option<String>,
    pub filterable: Option<bool>,
    pub personal_data: Option<bool>,
    pub position: Option<i32>,
    pub active: Option<bool>,
    /// With `data_type`: change the type (only while no record has a value).
    pub data_type: Option<String>,
}

/// Which fields.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct FieldsQuery {
    /// Only the fields of this record type, e.g. `content.entry`.
    target: Option<String>,
}

#[derive(Clone)]
pub(crate) struct CustomFieldsState {
    pub fields: CustomFields,
    pub entitlements: crate::entitlement::EntitlementsHandle,
}

/// Hạn mức theo gói (bước 35): số trường tự định nghĩa đang bật của khách, mọi loại bản ghi.
pub const CUSTOM_FIELDS_LIMIT: crate::entitlement::Limit = crate::entitlement::Limit::count(
    "kernel.custom_fields",
    "Custom fields a tenant can have on at once, across every record type.",
);

fn limit_reference(id: Uuid) -> String {
    format!("kernel.custom_field:{id}")
}

fn not_found(id: Uuid) -> Error {
    Error::not_found("kernel.custom_field_not_found", "There is no custom field `{id}`.")
        .with_param("id", id)
}

/// Kiểm và áp `settings` lên `field` (đang có hay mới).
fn apply(
    field: &mut CustomFieldView,
    settings: FieldSettings,
    target: CustomFieldTarget,
    problems: &mut Vec<FieldError>,
) {
    if let Some(labels) = settings.labels {
        if labels.values().any(|label| label.trim().is_empty() || label.chars().count() > 100) {
            problems.push(FieldError::new(
                "labels",
                "invalid",
                "must be labels of 1 to 100 characters by language",
            ));
        }
        field.labels = labels;
    }
    if let Some(help) = settings.help {
        if help.values().any(|text| text.chars().count() > 500) {
            problems.push(FieldError::new(
                "help",
                "invalid",
                "must be texts of at most 500 characters by language",
            ));
        }
        field.help = help;
    }
    if let Some(options) = settings.options {
        if options.len() > 200
            || options.iter().any(|option| option.trim().is_empty() || option.chars().count() > 100)
        {
            problems.push(FieldError::new(
                "options",
                "invalid",
                "must be at most 200 options of 1 to 100 characters",
            ));
        }
        field.options = options;
    }
    if let Some(scale) = settings.scale {
        if !(0..=6).contains(&scale) {
            problems.push(FieldError::new("scale", "invalid", "must be between 0 and 6"));
        }
        field.scale = Some(scale);
    }
    field.min = settings.min.or(field.min.take());
    field.max = settings.max.or(field.max.take());
    for (name, bound) in [("min", &field.min), ("max", &field.max)] {
        if bound.as_deref().is_some_and(|bound| {
            bound.parse::<f64>().is_err() && NaiveDate::parse_from_str(bound, "%Y-%m-%d").is_err()
        }) {
            problems.push(FieldError::new(
                name,
                "invalid",
                "must be a number or a date `YYYY-MM-DD`",
            ));
        }
    }
    field.reference_kind = settings.reference_kind.or(field.reference_kind.take());
    if let Some(visibility) = settings.visibility {
        if visibility != "internal" && !target.visibilities.contains(&visibility.as_str()) {
            problems.push(FieldError::new(
                "visibility",
                "invalid",
                "is not a visibility this record type supports",
            ));
        }
        field.visibility = visibility;
    }
    field.required = settings.required.unwrap_or(field.required);
    field.filterable = settings.filterable.unwrap_or(field.filterable);
    field.personal_data = settings.personal_data.unwrap_or(field.personal_data);
    field.position = settings.position.unwrap_or(field.position);
    field.active = settings.active.unwrap_or(field.active);
    if !DATA_TYPES.contains(&field.data_type.as_str()) {
        problems.push(FieldError::new("data_type", "invalid", "is not a known type"));
    }
    if matches!(field.data_type.as_str(), "option" | "options") && field.options.is_empty() {
        problems.push(FieldError::new(
            "options",
            "invalid",
            "are required by `option` and `options`",
        ));
    }
    if field.data_type == "reference" && field.reference_kind.is_none() {
        problems.push(FieldError::new("reference_kind", "invalid", "is required by `reference`"));
    }
    if let Some(default) = settings.default {
        if Value::is_null(&default) {
            field.default = None;
        } else {
            match normalize(field, &default) {
                Ok(normal) => field.default = Some(normal),
                Err(_) => problems.push(FieldError::new(
                    "default",
                    "invalid",
                    "is not a valid value of the field",
                )),
            }
        }
    }
    if field.labels.is_empty() {
        problems.push(FieldError::new("labels", "invalid", "need at least one label"));
    }
}

async fn save(tenant: &TenantContext, field: &CustomFieldView, insert: bool) -> Result<(), Error> {
    let json = |value: &dyn erased::Json| value.text();
    let values: Vec<(&str, sea_orm::Value)> = vec![
        ("labels", json(&field.labels).into()),
        ("help", json(&field.help).into()),
        ("required", field.required.into()),
        ("options", json(&field.options).into()),
        ("scale", field.scale.into()),
        ("min_value", field.min.clone().into()),
        ("max_value", field.max.clone().into()),
        ("reference_kind", field.reference_kind.clone().into()),
        ("default_value", field.default.as_ref().map(Value::to_string).into()),
        ("visibility", field.visibility.clone().into()),
        ("filterable", field.filterable.into()),
        ("personal_data", field.personal_data.into()),
        ("position", field.position.into()),
        ("active", field.active.into()),
        ("data_type", field.data_type.clone().into()),
        ("updated_at", field.updated_at.into()),
    ];
    if insert {
        let mut columns = vec!["id", "tenant_id", "target", "key", "created_at"];
        let mut row: Vec<sea_orm::sea_query::SimpleExpr> = vec![
            field.id.into(),
            tenant.id().as_uuid().into(),
            field.target.clone().into(),
            field.key.clone().into(),
            field.created_at.into(),
        ];
        for (column, value) in values {
            columns.push(column);
            row.push(value.into());
        }
        let statement = SqlQuery::insert()
            .into_table(Alias::new(TABLE))
            .columns(columns.into_iter().map(Alias::new))
            .values_panic(row)
            .to_owned();
        tenant.database().write().execute(&statement).await?;
    } else {
        let mut statement = SqlQuery::update();
        statement
            .table(Alias::new(TABLE))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
            .and_where(Expr::col(Alias::new("id")).eq(field.id));
        for (column, value) in values {
            statement.value(Alias::new(column), value);
        }
        tenant.database().write().execute(&statement).await?;
    }
    Ok(())
}

mod erased {
    //! Đổi giá trị bất kỳ thành JSON dạng chữ để lưu (cột `text`, như nhau trên hai database).
    pub(super) trait Json {
        fn text(&self) -> String;
    }

    impl<T: serde::Serialize> Json for T {
        fn text(&self) -> String {
            serde_json::to_string(self).unwrap_or_default()
        }
    }
}

impl CustomFieldsState {
    fn require_read(
        &self,
        principal: &Principal,
        target: &str,
    ) -> Result<CustomFieldTarget, Error> {
        let found = self.fields.target(target).ok_or_else(|| {
            Error::validation(vec![FieldError::new(
                "target",
                "unknown",
                "is not a record type that takes custom fields",
            )])
        })?;
        if principal.require(&CUSTOM_FIELDS_MANAGE).is_err() {
            principal.require(&found.read_permission)?;
        }
        Ok(found)
    }
}

/// List custom fields.
///
/// `kernel.custom_fields.manage`, or the permission that reads `target`'s records (without
/// `target`: the fields of every record type you may read). The fields the tenant defined, in order — interfaces and agents build forms and filter chips from them.
#[utoipa::path(
    get,
    path = "/custom-fields",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_custom_fields",
    security(("bearer" = [])),
    params(FieldsQuery),
    responses(
        (status = 200, description = "The fields.", body = Data<Vec<CustomFieldView>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission; code `permission_denied`.", body = Problem),
        (status = 422, description = "`target` takes no custom fields; see `errors`.", body = Problem),
    )
)]
async fn list_custom_fields(
    State(state): State<Arc<CustomFieldsState>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<FieldsQuery>,
) -> Result<Data<Vec<CustomFieldView>>, Error> {
    if let Some(target) = &query.target {
        state.require_read(&principal, target)?;
    }
    // Không nêu loại: chỉ trường của loại bản ghi người gọi đọc được.
    let fields = list(&tenant, query.target.as_deref())
        .await?
        .into_iter()
        .filter(|field| state.require_read(&principal, &field.target).is_ok())
        .collect();
    Ok(Data { data: fields })
}

/// Define a custom field.
///
/// Requires `kernel.custom_fields.manage`. At most 100 fields per record type.
#[utoipa::path(
    post,
    path = "/custom-fields",
    extensions(("x-permissions" = json!(["kernel.custom_fields.manage"]))),
    operation_id = "create_custom_field",
    security(("bearer" = [])),
    request_body = NewCustomField,
    responses(
        (status = 201, description = "Defined.", body = Data<CustomFieldView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.custom_fields.manage`; code `permission_denied`.", body = Problem),
        (status = 409, description = "The key is taken (`kernel.custom_field_key_taken`) or the record type has 100 fields (`kernel.custom_field_limit_reached`).", body = Problem),
        (status = 422, description = "Invalid definition; see `errors`.", body = Problem),
    )
)]
async fn create_custom_field(
    State(state): State<Arc<CustomFieldsState>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<NewCustomField>,
) -> Result<Created<CustomFieldView>, Error> {
    principal.require(&CUSTOM_FIELDS_MANAGE)?;
    let target = state.require_read(&principal, &input.target)?;
    let mut problems = Vec::new();
    if !is_key(&input.key) {
        problems.push(FieldError::new(
            "key",
            "invalid",
            "must be 1 to 64 lower-case letters, digits and `_`, starting with a letter",
        ));
    }
    let now = Utc::now().trunc_subsecs(6);
    let mut field = CustomFieldView {
        id: Uuid::now_v7(),
        target: input.target.clone(),
        key: input.key.clone(),
        data_type: input.data_type.clone(),
        labels: BTreeMap::new(),
        help: BTreeMap::new(),
        required: false,
        options: Vec::new(),
        scale: None,
        min: None,
        max: None,
        reference_kind: None,
        default: None,
        visibility: "internal".to_owned(),
        filterable: false,
        personal_data: false,
        position: 0,
        active: true,
        created_at: now,
        updated_at: now,
    };
    apply(&mut field, input.settings, target, &mut problems);
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    let existing = list(&tenant, Some(&input.target)).await?;
    if existing.iter().any(|known| known.key == field.key) {
        return Err(Error::conflict(
            "kernel.custom_field_key_taken",
            "Field `{key}` already exists.",
        )
        .with_param("key", field.key));
    }
    if existing.len() >= MAX_PER_TARGET {
        return Err(Error::conflict(
            "kernel.custom_field_limit_reached",
            "A record type has at most {max} custom fields.",
        )
        .with_param("max", MAX_PER_TARGET));
    }
    // Giữ một suất của hạn mức trước khi lưu; lưu hỏng thì trả lại.
    let reference = limit_reference(field.id);
    state.entitlements.reserve(tenant.id(), &CUSTOM_FIELDS_LIMIT, 1, &reference).await?;
    if let Err(error) = save(&tenant, &field, true).await {
        state.entitlements.release(tenant.id(), &CUSTOM_FIELDS_LIMIT, &reference).await?;
        return Err(error);
    }
    state.entitlements.confirm(tenant.id(), &CUSTOM_FIELDS_LIMIT, &reference).await?;
    Ok(Created::new(format!("/api/v1/kernel/custom-fields/{}", field.id), field))
}

async fn find(tenant: &TenantContext, id: Uuid) -> Result<CustomFieldView, Error> {
    list(tenant, None).await?.into_iter().find(|field| field.id == id).ok_or_else(|| not_found(id))
}

/// Get a custom field.
///
/// `kernel.custom_fields.manage`, or the permission that reads its record type.
#[utoipa::path(
    get,
    path = "/custom-fields/{id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_custom_field",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the field.")),
    responses(
        (status = 200, description = "The field.", body = Data<CustomFieldView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such field; code `kernel.custom_field_not_found`.", body = Problem),
    )
)]
async fn get_custom_field(
    State(state): State<Arc<CustomFieldsState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<CustomFieldView>, Error> {
    let field = find(&tenant, id).await?;
    state.require_read(&principal, &field.target)?;
    Ok(Data { data: field })
}

/// Change a custom field.
///
/// Requires `kernel.custom_fields.manage`. Labels, options, constraints and the rest change; the
/// key never does, and the type only while no record has a value (`kernel.custom_field_type_locked`).
#[utoipa::path(
    patch,
    path = "/custom-fields/{id}",
    extensions(("x-permissions" = json!(["kernel.custom_fields.manage"]))),
    operation_id = "update_custom_field",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the field.")),
    request_body = FieldSettings,
    responses(
        (status = 200, description = "The field.", body = Data<CustomFieldView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.custom_fields.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such field; code `kernel.custom_field_not_found`.", body = Problem),
        (status = 409, description = "Changing the type of a field records already use; code `kernel.custom_field_type_locked`.", body = Problem),
        (status = 422, description = "Invalid settings; see `errors`.", body = Problem),
    )
)]
async fn update_custom_field(
    State(state): State<Arc<CustomFieldsState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(settings): ValidJson<FieldSettings>,
) -> Result<Data<CustomFieldView>, Error> {
    principal.require(&CUSTOM_FIELDS_MANAGE)?;
    let mut field = find(&tenant, id).await?;
    let target = state.require_read(&principal, &field.target)?;
    if let Some(data_type) = settings.data_type.clone()
        && data_type != field.data_type
    {
        if field.updated_at != field.created_at || field.filterable {
            // Đơn giản và an toàn: trường đã dùng (đã sửa hay đã có index) thì khoá kiểu.
            return Err(Error::conflict(
                "kernel.custom_field_type_locked",
                "Field `{key}` may hold values already: create a new field instead of changing its type.",
            )
            .with_param("key", field.key));
        }
        field.data_type = data_type;
    }
    let mut problems = Vec::new();
    apply(&mut field, settings, target, &mut problems);
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    field.updated_at = Utc::now().trunc_subsecs(6);
    save(&tenant, &field, false).await?;
    Ok(Data { data: find(&tenant, id).await? })
}

/// Turn a custom field off.
///
/// Requires `kernel.custom_fields.manage`. The field stops being shown and accepted; values already
/// on records stay (a field is never deleted with its data).
#[utoipa::path(
    delete,
    path = "/custom-fields/{id}",
    extensions(("x-permissions" = json!(["kernel.custom_fields.manage"]))),
    operation_id = "delete_custom_field",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the field.")),
    responses(
        (status = 204, description = "Turned off."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.custom_fields.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such field; code `kernel.custom_field_not_found`.", body = Problem),
    )
)]
async fn delete_custom_field(
    State(state): State<Arc<CustomFieldsState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&CUSTOM_FIELDS_MANAGE)?;
    let mut field = find(&tenant, id).await?;
    let was_active = field.active;
    field.active = false;
    field.updated_at = Utc::now().trunc_subsecs(6);
    save(&tenant, &field, false).await?;
    if was_active {
        state.entitlements.release(tenant.id(), &CUSTOM_FIELDS_LIMIT, &limit_reference(id)).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Which custom fields to list.
#[derive(Debug, Default, Deserialize, validator::Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListFieldsInput {
    /// Only the fields of this record type, e.g. `content.entry`.
    target: Option<String>,
}

pub(crate) fn tools(fields: CustomFields) -> Vec<crate::mcp::Tool> {
    vec![
        crate::mcp::Tool::new(
            "list_custom_fields",
            "List the custom fields this tenant added to record types you may read (key, type, \
             labels, options, whether lists filter on it with `cf.<key>=`). Write their values in \
             a record's `custom_fields`.",
            move |tenant: TenantContext, principal: Principal, input: ListFieldsInput| {
                let state = CustomFieldsState {
                    fields: fields.clone(),
                    entitlements: crate::entitlement::EntitlementsHandle::default(),
                };
                async move {
                    if let Some(target) = &input.target {
                        state.require_read(&principal, target)?;
                    }
                    Ok::<Vec<CustomFieldView>, Error>(
                        list(&tenant, input.target.as_deref())
                            .await?
                            .into_iter()
                            .filter(|field| {
                                field.active && state.require_read(&principal, &field.target).is_ok()
                            })
                            .collect(),
                    )
                }
            },
        )
        .for_any_signed_in()
        .read_only(),
    ]
}

pub(crate) fn routes(state: CustomFieldsState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_custom_fields, create_custom_field))
        .routes(routes!(get_custom_field, update_custom_field, delete_custom_field))
        .with_state(Arc::new(state))
}

/// Migration tạo `kernel_custom_fields` (cấp khách hàng, mọi database dữ liệu).
pub(crate) struct CreateCustomFields;

impl MigrationName for CreateCustomFields {
    fn name(&self) -> &str {
        "m0029_create_custom_fields"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateCustomFields {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(TABLE))
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("target", 128))
                    .col(schema::string_len("key", 64))
                    .col(schema::string_len("data_type", 16))
                    .col(schema::text("labels"))
                    .col(schema::text("help"))
                    .col(schema::boolean("required"))
                    .col(schema::text("options"))
                    .col(schema::integer_null("scale"))
                    .col(schema::string_len_null("min_value", 64))
                    .col(schema::string_len_null("max_value", 64))
                    .col(schema::string_len_null("reference_kind", 128))
                    .col(schema::text_null("default_value"))
                    .col(schema::string_len("visibility", 16))
                    .col(schema::boolean("filterable"))
                    .col(schema::boolean("personal_data"))
                    .col(schema::integer("position"))
                    .col(schema::boolean("active"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("kernel_custom_fields_key")
                    .table(Alias::new(TABLE))
                    .col(Alias::new("tenant_id"))
                    .col(Alias::new("target"))
                    .col(Alias::new("key"))
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new(TABLE)).to_owned()).await
    }
}
