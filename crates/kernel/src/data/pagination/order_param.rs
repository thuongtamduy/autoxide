//! OpenAPI schema of the `order` query parameter of the three page query types: a plain string enum with
//! its default and description (D-20261125-list-pages).

// Viết tay vì hai lẽ: `inline` + `default` của utoipa ra `allOf` [enum, {default, type: object}] mà trình sinh SDK
// dịch thành `("asc" | "desc") & Record<string, unknown>`; còn `schema_with` thì derive bỏ qua doc comment của
// trường — nên mô tả phải đặt ở đây, trong schema.

use utoipa::openapi::schema::{ObjectBuilder, Schema, Type};

use super::Order;

/// Description of `order` in [`super::PageQuery`].
pub(super) const NEWEST_FIRST: &str = "`desc` (newest first, the default) or `asc` (oldest first). Keep \
    the same order while following `next_cursor`: changing it midway only changes where the next page starts.";

/// Description of `order` in [`super::NaturalOrderPageQuery`].
pub(super) const NATURAL: &str = "`asc` (the list's natural order, the default; the endpoint says what it is) \
    or `desc` (reversed). Text is ordered by the database's collation. Keep the same order while following \
    `next_cursor`: changing it midway only changes where the next page starts.";

/// Description of `order` in [`super::SortChoicePageQuery`].
pub(super) const BY_SORT: &str = "`asc` or `desc`. Default: follows `sort` — `desc` (newest first) for a \
    time, `asc` (A→Z, smallest first) for a name, position, code or size; the endpoint's `sort` says which. \
    Text is ordered by the database's collation. Keep the same order and `sort` while following \
    `next_cursor`: changing them midway only changes where the next page starts.";

fn schema(default: Option<Order>, description: &str) -> Schema {
    let default = default.map(|order| match order {
        Order::Asc => "asc",
        Order::Desc => "desc",
    });
    ObjectBuilder::new()
        .schema_type(Type::String)
        .enum_values(Some(["asc", "desc"]))
        .default(default.map(serde_json::Value::from))
        .description(Some(description))
        .build()
        .into()
}

pub(super) fn newest_first() -> Schema {
    schema(Some(Order::Desc), NEWEST_FIRST)
}

pub(super) fn natural() -> Schema {
    schema(Some(Order::Asc), NATURAL)
}

// Không có `default`: mặc định theo `sort`, OpenAPI không diễn tả được nên mô tả nói rõ.
pub(super) fn by_sort() -> Schema {
    schema(None, BY_SORT)
}

#[cfg(test)]
mod tests {
    use utoipa::IntoParams;

    use super::*;
    use crate::pagination::{NaturalOrderPageQuery, PageQuery, SortChoicePageQuery};

    // Schema của `order`: enum chuỗi thuần, có mô tả, có `default` khi kiểu query có mặc định cố định. Dạng `allOf`
    // làm SDK TypeScript hỏng kiểu (CI #517); thiếu mô tả thì client không biết mặc định (vòng kiểm thứ ba #517).
    #[test]
    fn tham_so_order_la_enum_chuoi_thuan_co_mo_ta_va_mac_dinh() {
        let check = |params: Vec<utoipa::openapi::path::Parameter>,
                     default: Option<&str>,
                     description: &str| {
            let order = params.into_iter().find(|param| param.name == "order").unwrap();
            let schema = serde_json::to_value(order.schema.unwrap()).unwrap();
            let mut expected = serde_json::json!({
                "type": "string",
                "enum": ["asc", "desc"],
                "description": description,
            });
            if let Some(default) = default {
                expected["default"] = default.into();
            }
            assert_eq!(schema, expected);
            assert!(!schema.to_string().contains("\"object\""), "{schema}");
        };
        check(PageQuery::into_params(|| None), Some("desc"), NEWEST_FIRST);
        check(NaturalOrderPageQuery::into_params(|| None), Some("asc"), NATURAL);
        check(SortChoicePageQuery::into_params(|| None), None, BY_SORT);
    }
}
