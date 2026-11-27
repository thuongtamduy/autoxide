//! Procedural macros of Autoxide. Modules use them through `autoxide-kernel` (`autoxide_kernel::db::Record`),
//! never by depending on this crate: the generated code names only `::autoxide_kernel` paths.
//!
//! - `#[derive(Record)]`: a database row of one module table (D-20261126-kernel-data-layer).

// Macro thủ tục chạy LÚC BIÊN DỊCH, trong tiến trình của trình biên dịch: nó nhận cú pháp của struct (token),
// trả về token mới. Lỗi không `panic!` mà thành `compile_error!` gắn đúng chỗ, nên người viết module thấy câu
// báo lỗi ngay dưới dòng sai.

mod record;

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
