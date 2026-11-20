//! Ghi và lọc theo chỉ mục giá trị trường tự định nghĩa (`write_index`, `condition`).

use super::values::{index_values, normalize};
use super::*;

impl CustomFields {
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

pub(super) trait WithField {
    fn with_field(self, field: String) -> FieldError;
}

impl WithField for FieldError {
    fn with_field(mut self, field: String) -> FieldError {
        self.field = field;
        self
    }
}
