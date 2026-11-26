//! OpenAPI documentation of the page query parameters `limit`, `cursor`, `order`, `count`, shared by
//! [`PageQuery`], [`NaturalOrderPageQuery`] and [`SortChoicePageQuery`] (D-20261125-list-pages).

// Dựng tay thay cho derive `IntoParams`, vì: (1) `inline` + `default` của utoipa ra `allOf` [enum, {default, type:
// object}] mà trình sinh SDK dịch thành `("asc" | "desc") & Record<string, unknown>`; (2) `schema_with` thì bỏ qua
// doc comment, mô tả không lên `parameter.description` — chỗ Swagger UI / Scalar hiển thị. Một hàm cho cả ba kiểu:
// thêm tham số trang sau này sửa một chỗ.

use serde_json::Value;
use utoipa::IntoParams;
use utoipa::openapi::path::{Parameter, ParameterBuilder, ParameterIn};
use utoipa::openapi::schema::{ObjectBuilder, Type};
use utoipa::openapi::{Required, Schema};

use super::{MAX_LIMIT, NaturalOrderPageQuery, Order, PageQuery, SortChoicePageQuery};

pub(super) const LIMIT: &str = "Maximum number of items to return, 1 to 200. Defaults to 50.";

pub(super) const CURSOR: &str = "Where to continue: the `next_cursor` of the previous page. Omit for the \
    first page. Treat the value as opaque.";

/// Description of `order` in [`PageQuery`].
pub(super) const NEWEST_FIRST: &str = "`desc` (newest first, the default) or `asc` (oldest first). Keep \
    the same order (and `sort`, where offered) while following `next_cursor`: changing it midway only \
    changes where the next page starts.";

/// Description of `order` in [`NaturalOrderPageQuery`].
pub(super) const NATURAL: &str = "`asc` (the list's natural order, the default; the endpoint says what it is) \
    or `desc` (reversed). Text is ordered by the database's collation. Keep the same order while following \
    `next_cursor`: changing it midway only changes where the next page starts.";

/// Description of `order` in [`SortChoicePageQuery`].
pub(super) const BY_SORT: &str = "`asc` or `desc`. Default: follows `sort` — `desc` (newest first) for a \
    time, `asc` (A→Z, smallest first) for a name, position, code or size; the endpoint's `sort` says which. \
    Text is ordered by the database's collation. Keep the same order and `sort` while following \
    `next_cursor`: changing them midway only changes where the next page starts.";

pub(super) const COUNT: &str = "`true` to also get `pagination.total`, the number of items matching the \
    filters, on the first page (without `cursor`). Costs one more query; leave it out when not needed.";

fn parameter(name: &str, description: &str, schema: Schema, example: Option<Value>) -> Parameter {
    ParameterBuilder::new()
        .name(name)
        .parameter_in(ParameterIn::Query)
        .required(Required::False)
        .description(Some(description))
        .schema(Some(schema))
        .example(example)
        .build()
}

/// The four parameters; `default` is the documented default of `order` (`None`: it follows `sort`).
fn page_parameters(default: Option<Order>, order_description: &str) -> Vec<Parameter> {
    let limit = ObjectBuilder::new()
        .schema_type(Type::Integer)
        .format(Some(utoipa::openapi::SchemaFormat::KnownFormat(
            utoipa::openapi::KnownFormat::Int32,
        )))
        .minimum(Some(1))
        .maximum(Some(MAX_LIMIT))
        .build();
    let cursor = ObjectBuilder::new().schema_type(Type::String).build();
    let default = default.map(|order| match order {
        Order::Asc => "asc",
        Order::Desc => "desc",
    });
    let order = ObjectBuilder::new()
        .schema_type(Type::String)
        .enum_values(Some(["asc", "desc"]))
        .default(default.map(Value::from))
        .build();
    let count = ObjectBuilder::new().schema_type(Type::Boolean).build();
    vec![
        parameter("limit", LIMIT, limit.into(), Some(Value::from(50))),
        parameter("cursor", CURSOR, cursor.into(), None),
        parameter("order", order_description, order.into(), None),
        parameter("count", COUNT, count.into(), Some(Value::Bool(true))),
    ]
}

// Bốn tham số luôn ở query, bất kể nơi gọi đề nghị chỗ nào.
impl IntoParams for PageQuery {
    fn into_params(_: impl Fn() -> Option<ParameterIn>) -> Vec<Parameter> {
        page_parameters(Some(Order::Desc), NEWEST_FIRST)
    }
}

impl IntoParams for NaturalOrderPageQuery {
    fn into_params(_: impl Fn() -> Option<ParameterIn>) -> Vec<Parameter> {
        page_parameters(Some(Order::Asc), NATURAL)
    }
}

impl IntoParams for SortChoicePageQuery {
    fn into_params(_: impl Fn() -> Option<ParameterIn>) -> Vec<Parameter> {
        page_parameters(None, BY_SORT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mỗi tham số có `description` ở parameter (Swagger UI / Scalar đọc chỗ này); `order` là enum chuỗi thuần,
    // có `default` khi kiểu query có mặc định cố định. Dạng `allOf` làm SDK TypeScript hỏng kiểu (CI #517).
    #[test]
    fn tham_so_trang_co_mo_ta_va_order_la_enum_chuoi_thuan() {
        let check = |params: Vec<Parameter>, default: Option<&str>, description: &str| {
            let params = serde_json::to_value(params).unwrap();
            let names: Vec<&str> =
                params.as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
            assert_eq!(names, ["limit", "cursor", "order", "count"]);
            for param in params.as_array().unwrap() {
                assert_eq!(param["in"], "query");
                assert_eq!(param["required"], false);
                assert!(param["description"].as_str().is_some_and(|d| !d.is_empty()), "{param}");
            }
            let order = &params[2];
            assert_eq!(order["description"], description);
            let mut expected = serde_json::json!({"type": "string", "enum": ["asc", "desc"]});
            if let Some(default) = default {
                expected["default"] = default.into();
            }
            assert_eq!(order["schema"], expected);
            assert_eq!(
                params[0]["schema"],
                serde_json::json!({"type": "integer", "format": "int32", "minimum": 1, "maximum": 200})
            );
        };
        check(PageQuery::into_params(|| None), Some("desc"), NEWEST_FIRST);
        check(NaturalOrderPageQuery::into_params(|| None), Some("asc"), NATURAL);
        check(SortChoicePageQuery::into_params(|| None), None, BY_SORT);
    }
}
