//! Tool MCP của `address`: cùng service với REST, đều chỉ đọc.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::mcp::{PageInput, Tool};
use autoxide_kernel::pagination::{NaturalOrderPageQuery, PageRequest};
use autoxide_kernel::tenancy::TenantContext;
use serde::Deserialize;
use utoipa::ToSchema;
use validator::Validate;

use crate::api::AddressInput;
use crate::service::AddressService;

fn vn() -> String {
    "VN".to_owned()
}

/// Which provinces.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProvincesInput {
    /// ISO country code; only `VN` for now. Defaults to `VN`.
    #[serde(default = "vn")]
    country: String,
    /// The list in force on this day (`YYYY-MM-DD`); defaults to today.
    as_of: Option<String>,
    /// Maximum number of items to return, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `asc` (code order, the default) or `desc`.
    order: Option<String>,
}

/// The communes of a province.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CommunesInput {
    #[serde(default = "vn")]
    country: String,
    /// Code of the province, e.g. `01`.
    province: String,
    as_of: Option<String>,
    /// Maximum number of items to return, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `asc` (code order, the default) or `desc`.
    order: Option<String>,
}

// Cùng kiểu tham số trang với REST (`NaturalOrderPageQuery`, mặc định `asc`), nên kiểm và lỗi giống hệt.
fn natural_page(
    limit: Option<u32>,
    cursor: Option<String>,
    order: Option<String>,
) -> Result<PageRequest, Error> {
    PageRequest::try_from(NaturalOrderPageQuery::new(
        limit.map(|limit| limit.to_string()),
        cursor,
        order,
    ))
}

/// A search by name.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchInput {
    #[serde(default = "vn")]
    country: String,
    /// Part of the name, with or without accents.
    #[validate(length(max = 100))]
    q: Option<String>,
    /// Only units of this province.
    province: Option<String>,
    /// `1`: provinces; `2`: communes.
    level: Option<i32>,
    /// Maximum number of items to return, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
}

/// An address to check.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ValidateInput {
    #[serde(default = "vn")]
    country: String,
    province_code: String,
    commune_code: String,
    /// House number, street, village…
    line: String,
}

pub(crate) fn all(service: &Arc<AddressService>) -> Vec<Tool> {
    let (provinces, communes, search, validate) =
        (service.clone(), service.clone(), service.clone(), service.clone());
    vec![
        Tool::new(
            "list_provinces",
            "List the provinces of a country (Vietnam), in the order of the official list. \
             Follow `pagination.next_cursor` for more.",
            move |_: TenantContext, _: Principal, input: ProvincesInput| {
                let service = provinces.clone();
                async move {
                    let page = natural_page(input.limit, input.cursor, input.order)?;
                    service.provinces(&input.country, input.as_of.as_deref(), &page).await
                }
            },
        )
        .for_any_signed_in()
        .read_only(),
        Tool::new(
            "list_communes",
            "List the communes, wards and special zones of a province, in code order. Follow \
             `pagination.next_cursor` for more.",
            move |_: TenantContext, _: Principal, input: CommunesInput| {
                let service = communes.clone();
                async move {
                    let page = natural_page(input.limit, input.cursor, input.order)?;
                    service
                        .communes(&input.country, &input.province, input.as_of.as_deref(), &page)
                        .await
                }
            },
        )
        .for_any_signed_in()
        .read_only(),
        Tool::new(
            "search_units",
            "Search administrative units by name, with or without accents (`ha noi` also finds the accented name).",
            move |_: TenantContext, _: Principal, input: SearchInput| {
                let service = search.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None).page()?;
                    service
                        .search(&input.country, input.q.as_deref(), input.province.as_deref(), input.level, &page)
                        .await
                }
            },
        )
        .for_any_signed_in()
        .read_only(),
        Tool::new(
            "validate_address",
            "Check an address (the commune must belong to the province) and fill in the names of \
             its units.",
            move |_: TenantContext, _: Principal, input: ValidateInput| {
                let service = validate.clone();
                async move {
                    let input = AddressInput::new(input.country, input.province_code, input.commune_code, input.line);
                    let address = service.check(input, "").await.map_err(Error::validation)?;
                    Ok::<_, Error>(serde_json::json!({ "data": address }))
                }
            },
        )
        .for_any_signed_in()
        .read_only(),
    ]
}
