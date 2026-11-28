//! Procedural macros of Autoxide. Modules use them through `autoxide-kernel` (`autoxide_kernel::db::Record`),
//! never by depending on this crate: the generated code names only `::autoxide_kernel` paths.
//!
//! - `#[derive(Record)]`: a database row of one module table (D-20261126-kernel-data-layer).
//! - `#[endpoint]`, `#[derive(Schema)]`, `#[derive(Params)]`, `#[derive(Validate)]`: the HTTP layer
//!   (D-20261126-kernel-web-layer).

// Macro thủ tục chạy LÚC BIÊN DỊCH, trong tiến trình của trình biên dịch: nó nhận cú pháp của struct (token),
// trả về token mới. Lỗi không `panic!` mà thành `compile_error!` gắn đúng chỗ, nên người viết module thấy câu
// báo lỗi ngay dưới dòng sai.

mod endpoint;
mod record;
mod shadow;
mod validate;

use proc_macro::TokenStream;

/// A row of a module table. See `autoxide_kernel::db` for the full description.
///
/// ```ignore
/// #[derive(Record, Clone, Debug)]
/// #[record(table = "notes_note", scope = tenant)]
/// pub(crate) struct Note {
///     #[record(primary_key)]
///     pub id: Uuid,
///     pub tenant_id: TenantKey,
///     pub title: String,
/// }
/// ```
#[proc_macro_derive(Record, attributes(record))]
pub fn derive_record(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    record::expand(&input).unwrap_or_else(syn::Error::into_compile_error).into()
}

/// An OpenAPI schema for a type, written in utoipa's attribute grammar (`#[schema(…)]`, `#[serde(…)]`, doc
/// comments, `#[deprecated]`) without naming `utoipa`. Use it as `autoxide_kernel::http::Schema`.
#[proc_macro_derive(Schema, attributes(schema))]
pub fn derive_schema(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    shadow::expand(shadow::Kind::Schema, &input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// The query or path parameters of an endpoint, written in utoipa's attribute grammar (`#[param(…)]`,
/// `#[into_params(…)]`) without naming `utoipa`. Use it as `autoxide_kernel::http::Params`.
#[proc_macro_derive(Params, attributes(param, into_params))]
pub fn derive_params(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    shadow::expand(shadow::Kind::Params, &input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Hidden: registers `#[serde(…)]` as a helper attribute and expands to nothing. The shadow copy of
/// `#[derive(Params)]` derives it, because utoipa's `IntoParams` reads `serde` attributes (`default`, `rename`)
/// without declaring them, so the copy would not compile on its own.
#[doc(hidden)]
#[proc_macro_derive(SerdeAttributes, attributes(serde))]
pub fn derive_serde_attributes(_input: TokenStream) -> TokenStream {
    TokenStream::new()
}

/// Input checks on fields: `#[validate(length(…))]`, `range(…)`, `email`, `url`, `nested`. Use it as
/// `autoxide_kernel::http::Validate`.
#[proc_macro_derive(Validate, attributes(validate))]
pub fn derive_validate(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    validate::expand(&input).unwrap_or_else(syn::Error::into_compile_error).into()
}

/// An HTTP endpoint with its OpenAPI description: the grammar of `utoipa::path`, plus who may call
/// (`permission = "…"`, `signed_in` or `public`, required), a default `operation_id`, `etag` on a response and the
/// standard 401 / 403 responses. Use it as `autoxide_kernel::http::endpoint`.
#[proc_macro_attribute]
pub fn endpoint(args: TokenStream, item: TokenStream) -> TokenStream {
    let function = syn::parse_macro_input!(item as syn::ItemFn);
    endpoint::expand(args.into(), function).unwrap_or_else(syn::Error::into_compile_error).into()
}
