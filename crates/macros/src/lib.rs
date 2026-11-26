//! Procedural macros of Autoxide. Modules use them through `autoxide-kernel` (`autoxide_kernel::db::Record`),
//! never by depending on this crate: the generated code names only `::autoxide_kernel` paths.
//!
//! - `#[derive(Record)]`: a database row of one module table (D-20261126-kernel-data-layer).

// Macro thủ tục chạy LÚC BIÊN DỊCH, trong tiến trình của trình biên dịch: nó nhận cú pháp của struct (token),
// trả về token mới. Lỗi không `panic!` mà thành `compile_error!` gắn đúng chỗ, nên người viết module thấy câu
// báo lỗi ngay dưới dòng sai.

mod endpoint;
mod record;
mod shadow;

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

/// SPIKE F9 (never merged): an OpenAPI schema for a type, written in utoipa's attribute grammar without naming
/// `utoipa`.
#[proc_macro_derive(Schema, attributes(schema))]
pub fn derive_schema(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    shadow::expand(shadow::Kind::Schema, &input).unwrap_or_else(syn::Error::into_compile_error).into()
}

/// SPIKE F9 (never merged): query / path parameters of an endpoint, written in utoipa's attribute grammar
/// without naming `utoipa`.
#[proc_macro_derive(Params, attributes(param, into_params))]
pub fn derive_params(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    shadow::expand(shadow::Kind::Params, &input).unwrap_or_else(syn::Error::into_compile_error).into()
}

/// SPIKE F9 (never merged): an HTTP endpoint with its OpenAPI description (the grammar of `utoipa::path`, plus
/// `permission = "…"`, a default `operation_id` and `etag` on a response).
#[proc_macro_attribute]
pub fn endpoint(args: TokenStream, item: TokenStream) -> TokenStream {
    let function = syn::parse_macro_input!(item as syn::ItemFn);
    endpoint::expand(args.into(), function).unwrap_or_else(syn::Error::into_compile_error).into()
}
