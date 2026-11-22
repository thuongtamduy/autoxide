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

use crate::time::{Date, Timestamp};
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
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

mod index_migration;
pub use index_migration::index_migration;

mod http;
mod index;
mod migration;
mod values;

use index::WithField;
use values::normalize;

pub(crate) use http::{routes, tools};
pub(crate) use migration::CreateCustomFields;

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
    /// Nhận cả loại con `<target>.<khoá>` do khách tạo lúc chạy (vd `content.type.du_an` cho loại nội
    /// dung tự định nghĩa — bước 67, D-235): mỗi loại con một bộ trường riêng, cùng permission.
    pub subtypes: bool,
}

impl CustomFieldTarget {
    pub const fn new(target: &'static str, read: Permission, write: Permission) -> Self {
        CustomFieldTarget {
            target,
            read_permission: read,
            write_permission: write,
            visibilities: &["internal"],
            subtypes: false,
        }
    }

    /// Nhận cả loại con `<target>.<khoá>` (khoá `snake_case`) do module tạo lúc chạy.
    #[must_use]
    pub const fn with_subtypes(mut self) -> Self {
        self.subtypes = true;
        self
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
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
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
        let targets = self.targets.get()?;
        if let Some(found) = targets.get(name) {
            return Some(*found);
        }
        // Loại con `<target>.<khoá>` của loại khai `with_subtypes`.
        let (parent, key) = name.rsplit_once('.')?;
        targets.get(parent).filter(|target| target.subtypes && is_key(key)).copied()
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
        check_values(&fields, incoming, current, creating)
    }

    /// Như [`validate`](Self::validate) khi người dùng cuối tạo bản ghi (vd người mua đặt hàng —
    /// D-235): chỉ xét trường mức `buyer_input` của `target` — khoá khác (kể cả trường `internal`)
    /// là lỗi `unknown`, và trường `internal` bắt buộc không chặn người dùng cuối (nhân sự điền sau).
    pub async fn validate_buyer_input(
        &self,
        tenant: &TenantContext,
        target: &str,
        incoming: Option<&Map<String, Value>>,
    ) -> Result<Map<String, Value>, Error> {
        let fields: Vec<CustomFieldView> = self
            .definitions(tenant, target)
            .await?
            .into_iter()
            .filter(|field| field.visibility == "buyer_input")
            .collect();
        check_values(&fields, incoming, None, true)
    }
}

/// Kiểm `incoming` gộp vào `current` theo `fields` (định nghĩa đang bật), gom mọi lỗi một lần.
fn check_values(
    fields: &[CustomFieldView],
    incoming: Option<&Map<String, Value>>,
    current: Option<&Map<String, Value>>,
    creating: bool,
) -> Result<Map<String, Value>, Error> {
    {
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
        for field in fields {
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
#[serde(deny_unknown_fields)]
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
            bound.parse::<f64>().is_err() && Date::parse_from_str(bound, "%Y-%m-%d").is_err()
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
