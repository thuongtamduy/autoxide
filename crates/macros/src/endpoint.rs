//! SPIKE F9 (nhánh nháp, không merge): `#[endpoint(…)]` thay `#[utoipa::path(…)]`.
//!
//! Ngữ pháp là của `utoipa::path` cộng ba đường tắt:
//! - `permission = "notes.read"` → `extensions(("x-permissions" = json!(["notes.read"])))` +
//!   `security(("bearer" = []))`;
//! - `operation_id` mặc định là tên hàm;
//! - `etag` trong một response → header `ETag` chuẩn.
//!
//! Hàm của module giữ nguyên. Macro sinh thêm một struct "đánh dấu" CÙNG TÊN hàm (struct có ngoặc nhọn chỉ ở namespace
//! kiểu, hàm ở namespace giá trị — Rust cho hai thứ trùng tên ở hai namespace; struct đơn vị `struct x;` thì chiếm
//! cả namespace giá trị nên không dùng được) cài `autoxide_kernel::http::Endpoint`. Trong thân
//! `Endpoint::__describe` có một bản sao chữ ký của hàm mang `#[utoipa::path(…)]` (utoipa đọc chữ ký để hiểu
//! extractor của axum), với `utoipa` lấy từ `::autoxide_kernel::__private` — module không khai `utoipa`.

use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use quote::{format_ident, quote};
use syn::ItemFn;

const ETAG_HEADER: &str = "Version of the record: send it back in `If-Match` when changing it.";

// Tách token theo dấu phẩy ở mức ngoài cùng (dấu phẩy trong ngoặc nằm trong `Group`, không bị tách).
fn split_commas(tokens: TokenStream) -> Vec<TokenStream> {
    let mut items = Vec::new();
    let mut current = Vec::new();
    for token in tokens {
        match &token {
            TokenTree::Punct(punct) if punct.as_char() == ',' => {
                if !current.is_empty() {
                    items.push(current.drain(..).collect());
                }
            }
            _ => current.push(token),
        }
    }
    if !current.is_empty() {
        items.push(current.into_iter().collect());
    }
    items
}

fn leading_ident(item: &TokenStream) -> Option<String> {
    match item.clone().into_iter().next() {
        Some(TokenTree::Ident(ident)) => Some(ident.to_string()),
        _ => None,
    }
}

// `permission = "…"` → chuỗi permission.
fn permission_of(item: &TokenStream) -> syn::Result<syn::LitStr> {
    let tokens: Vec<TokenTree> = item.clone().into_iter().collect();
    match tokens.as_slice() {
        [_, TokenTree::Punct(eq), TokenTree::Literal(literal)] if eq.as_char() == '=' => {
            syn::parse2::<syn::LitStr>(TokenTree::Literal(literal.clone()).into())
        }
        _ => Err(syn::Error::new_spanned(item, "expected `permission = \"<module>.<action>\"`")),
    }
}

// `responses((status = 200, …, etag), …)`: thay `etag` trong từng response bằng header `ETag` chuẩn.
fn expand_responses(item: TokenStream) -> TokenStream {
    let mut tokens = item.into_iter();
    let (Some(head), Some(TokenTree::Group(group))) = (tokens.next(), tokens.next()) else {
        return TokenStream::new();
    };
    let responses = split_commas(group.stream()).into_iter().map(|response| {
        let mut parts = response.into_iter();
        match (parts.next(), parts.next()) {
            (Some(TokenTree::Group(inner)), None) if inner.delimiter() == Delimiter::Parenthesis => {
                let fields = split_commas(inner.stream()).into_iter().map(|field| {
                    if field.to_string() == "etag" {
                        quote!(headers(("ETag" = String, description = #ETAG_HEADER)))
                    } else {
                        field
                    }
                });
                let mut rebuilt = Group::new(Delimiter::Parenthesis, quote!(#(#fields),*));
                rebuilt.set_span(inner.span());
                quote!(#rebuilt)
            }
            (first, second) => quote!(#first #second #(#parts)*),
        }
    });
    let rebuilt = Group::new(Delimiter::Parenthesis, quote!(#(#responses),*));
    quote!(#head #rebuilt)
}

pub(crate) fn expand(args: TokenStream, function: ItemFn) -> syn::Result<TokenStream> {
    let name = &function.sig.ident;
    let mut utoipa_args = Vec::new();
    let mut has_operation_id = false;
    let mut permission = None;
    for item in split_commas(args) {
        match leading_ident(&item).as_deref() {
            Some("permission") => permission = Some(permission_of(&item)?),
            Some("responses") => utoipa_args.push(expand_responses(item)),
            Some("operation_id") => {
                has_operation_id = true;
                utoipa_args.push(item);
            }
            _ => utoipa_args.push(item),
        }
    }
    // Thứ tự mục trong `#[utoipa::path]` không đổi tài liệu sinh ra; giữ gần cách viết tay cũ cho dễ so.
    let mut generated = Vec::new();
    if let Some(permission) = &permission {
        generated.push(quote!(extensions(("x-permissions" = json!([#permission])))));
    }
    if !has_operation_id {
        let id = name.to_string();
        generated.push(quote!(operation_id = #id));
    }
    if permission.is_some() {
        generated.push(quote!(security(("bearer" = []))));
    }
    let mut all = utoipa_args;
    // Đặt sau phương thức và `path` (hai mục đầu) như cách viết tay.
    let split = all.len().min(2);
    let tail = all.split_off(split);
    all.extend(generated);
    all.extend(tail);

    let docs: Vec<_> = function.attrs.iter().filter(|attribute| attribute.path().is_ident("doc")).collect();
    let visibility = &function.vis;
    let inputs = &function.sig.inputs;
    let output = &function.sig.output;
    let path_struct = format_ident!("__path_{}", name);
    Ok(quote! {
        #function

        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        #visibility struct #name {}

        impl ::autoxide_kernel::http::Endpoint for #name {
            fn __describe() -> ::autoxide_kernel::http::EndpointDoc {
                use ::autoxide_kernel::__private::utoipa;
                #[allow(unused_imports)]
                use utoipa::{Path as _, __dev::{SchemaReferences as _, Tags as _}};

                // Bản sao chữ ký cho utoipa đọc; không bao giờ chạy.
                #[utoipa::path(#(#all),*)]
                #(#docs)*
                #[allow(dead_code, unused_variables, clippy::all)]
                async fn #name(#inputs) #output {
                    loop {}
                }

                let mut schemas = ::std::vec::Vec::new();
                <#path_struct as utoipa::__dev::SchemaReferences>::schemas(&mut schemas);
                let mut operation = <#path_struct as utoipa::Path>::operation();
                let tags = <#path_struct as utoipa::__dev::Tags>::tags();
                if !tags.is_empty() {
                    operation.tags.get_or_insert_with(::std::vec::Vec::new)
                        .extend(tags.iter().map(::std::string::ToString::to_string));
                }
                ::autoxide_kernel::http::EndpointDoc::__new(
                    <#path_struct as utoipa::Path>::path(),
                    <#path_struct as utoipa::Path>::methods(),
                    operation,
                    schemas,
                )
            }
        }
    })
}
