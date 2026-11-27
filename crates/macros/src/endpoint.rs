//! `#[endpoint(…)]`: an HTTP handler with its OpenAPI description (D-20261126-kernel-web-layer).
//!
//! The grammar is that of `utoipa::path`, plus:
//! - who may call, declared on every endpoint (a missing declaration is a compile error):
//!   `permission = "notes.read"` (or a list), `signed_in` (any signed-in caller), `public` (no sign-in);
//! - `approval = "agent"`: an AI agent's call waits for a person's approval (`x-approval`);
//! - `operation_id` defaults to the function name;
//! - `etag` inside a response adds the standard `ETag` header;
//! - the standard 401 / 403 responses are added when the module does not write its own.

// Hàm của module giữ nguyên. Macro sinh thêm một struct đánh dấu CÙNG TÊN hàm, cài `autoxide_kernel::http::Endpoint`.
// Struct có ngoặc nhọn (`struct x {}`) chỉ nằm ở namespace kiểu, hàm ở namespace giá trị — Rust cho hai thứ trùng
// tên ở hai namespace; struct đơn vị `struct x;` thì chiếm cả namespace giá trị nên không dùng được. Nhờ vậy
// `routes![a, b]` chỉ cần tên hàm.
//
// Trong thân `Endpoint::__describe` có một bản sao chữ ký của hàm (thân `loop {}`, không bao giờ chạy) mang
// `#[utoipa::path(…)]`, với `utoipa` lấy từ `::autoxide_kernel::__private`: module không khai `utoipa`. Bản sao giữ
// nguyên mẫu tham số vì feature `axum_extras` của utoipa đọc extractor (`Path(id): Path<Uuid>`) để điền tham số.

use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use quote::{format_ident, quote};
use syn::ItemFn;

const ETAG_HEADER: &str = "Version of the record: send it back in `If-Match` when changing it.";
const NOT_SIGNED_IN: &str = "Not signed in; see `GET /api/v1/auth/me` for the codes.";
const METHODS: [&str; 8] = ["get", "post", "put", "patch", "delete", "head", "options", "trace"];

/// Who may call the endpoint.
enum Access {
    Permissions(Vec<syn::LitStr>),
    SignedIn,
    Public,
}

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

fn is_bare(item: &TokenStream) -> bool {
    item.clone().into_iter().count() == 1
}

// `permission = "…"` hay `permission = ["…", "…"]`.
fn permissions_of(item: TokenStream) -> syn::Result<Vec<syn::LitStr>> {
    let tokens: Vec<TokenTree> = item.clone().into_iter().collect();
    let error = || {
        syn::Error::new_spanned(
            &item,
            "expected `permission = \"<module>.<action>\"` or `permission = [\"…\", …]`",
        )
    };
    let value = match tokens.as_slice() {
        [_, TokenTree::Punct(eq), value] if eq.as_char() == '=' => value.clone(),
        _ => return Err(error()),
    };
    let permissions = match value {
        TokenTree::Literal(_) => vec![syn::parse2::<syn::LitStr>(value.into())?],
        TokenTree::Group(group) if group.delimiter() == Delimiter::Bracket => {
            split_commas(group.stream())
                .into_iter()
                .map(syn::parse2::<syn::LitStr>)
                .collect::<syn::Result<Vec<_>>>()?
        }
        _ => return Err(error()),
    };
    if permissions.is_empty() {
        return Err(syn::Error::new_spanned(
            &item,
            "list at least one permission, or write `signed_in`",
        ));
    }
    for permission in &permissions {
        let name = permission.value();
        let valid = name
            .split_once('.')
            .is_some_and(|(module, action)| !module.is_empty() && !action.is_empty());
        if !valid {
            return Err(syn::Error::new_spanned(
                permission,
                "a permission is named `<module>.<action>`",
            ));
        }
    }
    Ok(permissions)
}

// Một response `(status = 200, …)`: thay `etag` bằng header `ETag` chuẩn; trả kèm mã trạng thái nếu đọc được.
fn expand_response(response: TokenStream) -> (TokenStream, Option<String>) {
    let mut parts = response.clone().into_iter();
    let (Some(TokenTree::Group(inner)), None) = (parts.next(), parts.next()) else {
        return (response, None);
    };
    if inner.delimiter() != Delimiter::Parenthesis {
        return (response, None);
    }
    let mut status = None;
    let fields: Vec<TokenStream> = split_commas(inner.stream())
        .into_iter()
        .map(|field| {
            let tokens: Vec<TokenTree> = field.clone().into_iter().collect();
            if let [TokenTree::Ident(name), TokenTree::Punct(eq), value] = tokens.as_slice()
                && name == "status"
                && eq.as_char() == '='
            {
                status = Some(value.to_string().trim_matches('"').to_string());
            }
            if field.to_string() == "etag" {
                quote!(headers(("ETag" = String, description = #ETAG_HEADER)))
            } else {
                field
            }
        })
        .collect();
    let mut rebuilt = Group::new(Delimiter::Parenthesis, quote!(#(#fields),*));
    rebuilt.set_span(inner.span());
    (quote!(#rebuilt), status)
}

// `responses(…)`: mở rộng từng response, thêm 401 / 403 chuẩn khi module chưa viết.
fn expand_responses(item: TokenStream, access: &Access) -> syn::Result<TokenStream> {
    let mut tokens = item.clone().into_iter();
    let (Some(head), Some(TokenTree::Group(group)), None) =
        (tokens.next(), tokens.next(), tokens.next())
    else {
        return Err(syn::Error::new_spanned(item, "expected `responses((status = …, …), …)`"));
    };
    let mut statuses = Vec::new();
    let mut responses: Vec<TokenStream> = split_commas(group.stream())
        .into_iter()
        .map(|response| {
            let (response, status) = expand_response(response);
            statuses.extend(status);
            response
        })
        .collect();
    let has = |code: &str| statuses.iter().any(|status| status == code);
    if !matches!(access, Access::Public) && !has("401") {
        responses.push(quote!((status = 401, description = #NOT_SIGNED_IN, body = ::autoxide_kernel::response::Problem)));
    }
    if let Access::Permissions(permissions) = access
        && !has("403")
    {
        let names: Vec<String> =
            permissions.iter().map(|permission| format!("`{}`", permission.value())).collect();
        let description = if names.len() == 1 {
            format!("Missing permission {}; code `permission_denied`.", names[0])
        } else {
            format!(
                "Missing permission: needs one of {}; code `permission_denied`.",
                names.join(", ")
            )
        };
        responses.push(quote!((status = 403, description = #description, body = ::autoxide_kernel::response::Problem)));
    }
    let rebuilt = Group::new(Delimiter::Parenthesis, quote!(#(#responses),*));
    Ok(quote!(#head #rebuilt))
}

/// Phần mở rộng mà chỉ macro được sinh: tác giả viết thì sai lệch với kiểm quyền lúc chạy.
const RESERVED_EXTENSIONS: [(&str, &str); 3] = [
    ("x-permissions", "`permission = …` / `signed_in`"),
    ("x-public", "`public`"),
    ("x-approval", "`approval = \"agent\"`"),
];

// `extensions(("x-a" = …), ("x-b" = …))` → từng mục `("x-a" = …)`; từ chối mục dành riêng cho macro.
fn extension_entries(item: TokenStream) -> syn::Result<Vec<TokenStream>> {
    let mut tokens = item.clone().into_iter();
    let (Some(_), Some(TokenTree::Group(group)), None) =
        (tokens.next(), tokens.next(), tokens.next())
    else {
        return Err(syn::Error::new_spanned(
            item,
            "expected `extensions((\"x-…\" = json!(…)), …)`",
        ));
    };
    let entries = split_commas(group.stream());
    for entry in &entries {
        let key = entry.clone().into_iter().find_map(|token| match token {
            TokenTree::Group(inner) => {
                inner.stream().into_iter().next().map(|first| first.to_string())
            }
            _ => None,
        });
        if let Some(key) = key
            && let Some((name, instead)) =
                RESERVED_EXTENSIONS.iter().find(|(name, _)| key.trim_matches('"') == *name)
        {
            return Err(syn::Error::new_spanned(
                entry,
                format!("`{name}` is generated by `#[endpoint]`: write {instead} instead"),
            ));
        }
    }
    Ok(entries)
}

// `approval = "agent"`: thao tác do AI agent gọi phải chờ người duyệt (kernel `approvals`, `x-approval`).
fn approval_of(item: TokenStream) -> syn::Result<syn::LitStr> {
    let tokens: Vec<TokenTree> = item.clone().into_iter().collect();
    let value = match tokens.as_slice() {
        [_, TokenTree::Punct(eq), TokenTree::Literal(literal)] if eq.as_char() == '=' => {
            syn::parse2::<syn::LitStr>(TokenTree::Literal(literal.clone()).into())?
        }
        _ => return Err(syn::Error::new_spanned(item, "expected `approval = \"agent\"`")),
    };
    if value.value() != "agent" {
        return Err(syn::Error::new_spanned(value, "the only approval kind is `\"agent\"`"));
    }
    Ok(value)
}

pub(crate) fn expand(args: TokenStream, function: ItemFn) -> syn::Result<TokenStream> {
    let name = &function.sig.ident;
    let mut access = None;
    let mut leading = Vec::new();
    let mut rest = Vec::new();
    let mut responses = None;
    let mut has_operation_id = false;
    let mut has_security = false;
    // Mục `extensions(…)` của tác giả: utoipa chỉ giữ nhóm `extensions` cuối cùng, nên macro gộp mọi mục vào MỘT nhóm
    // cùng các mục nó sinh — nếu để hai nhóm, `x-permissions` (mà permission gate dựa vào) mất lặng lẽ.
    let mut extensions: Vec<TokenStream> = Vec::new();
    let mut approval = None;
    for item in split_commas(args) {
        let ident = leading_ident(&item);
        let mut set_access = |value: Access, item: &TokenStream| {
            if access.is_some() {
                return Err(syn::Error::new_spanned(
                    item,
                    "declare who may call once: `permission = …`, `signed_in` or `public`",
                ));
            }
            access = Some(value);
            Ok(())
        };
        match ident.as_deref() {
            Some("permission") => {
                set_access(Access::Permissions(permissions_of(item.clone())?), &item)?;
            }
            Some("signed_in") if is_bare(&item) => set_access(Access::SignedIn, &item)?,
            Some("public") if is_bare(&item) => set_access(Access::Public, &item)?,
            Some("responses") => responses = Some(item),
            Some(method) if is_bare(&item) && METHODS.contains(&method) => leading.push(item),
            Some("path") => leading.push(item),
            Some("operation_id") => {
                has_operation_id = true;
                rest.push(item);
            }
            Some("security") => {
                has_security = true;
                rest.push(item);
            }
            Some("extensions") => extensions.extend(extension_entries(item)?),
            Some("approval") => approval = Some(approval_of(item)?),
            _ => rest.push(item),
        }
    }
    let Some(access) = access else {
        return Err(syn::Error::new_spanned(
            name,
            "declare who may call this endpoint: `permission = \"<module>.<action>\"`, `signed_in` (any signed-in \
             caller) or `public` (no sign-in)",
        ));
    };
    let Some(responses) = responses else {
        return Err(syn::Error::new_spanned(
            name,
            "describe the responses: `responses((status = 200, …), …)`",
        ));
    };
    let responses = expand_responses(responses, &access)?;

    // Thứ tự mục trong `#[utoipa::path]` không đổi tài liệu sinh ra; giữ gần cách viết tay cũ.
    let mut all = leading;
    let mut generated = match &access {
        Access::Permissions(permissions) => {
            vec![quote!(("x-permissions" = json!([#(#permissions),*])))]
        }
        Access::SignedIn => vec![quote!(("x-permissions" = json!([])))],
        Access::Public => vec![quote!(("x-public" = json!(true)))],
    };
    if let Some(approval) = approval {
        generated.push(quote!(("x-approval" = json!(#approval))));
    }
    generated.extend(extensions);
    all.push(quote!(extensions(#(#generated),*)));
    if !has_operation_id {
        let id = name.to_string();
        all.push(quote!(operation_id = #id));
    }
    if !has_security && !matches!(access, Access::Public) {
        all.push(quote!(security(("bearer" = []))));
    }
    all.extend(rest);
    all.push(responses);

    let docs: Vec<_> =
        function.attrs.iter().filter(|attribute| attribute.path().is_ident("doc")).collect();
    // `#[cfg]` và `#[deprecated]` của hàm đi theo struct đánh dấu: hàm bị tắt thì đánh dấu cũng tắt; endpoint cũ thì
    // `routes![…]` cảnh báo như gọi hàm cũ.
    let carried: Vec<_> = function
        .attrs
        .iter()
        .filter(|attribute| {
            attribute.path().is_ident("cfg") || attribute.path().is_ident("deprecated")
        })
        .collect();
    let cfgs: Vec<_> =
        carried.iter().filter(|attribute| attribute.path().is_ident("cfg")).collect();
    let visibility = &function.vis;
    let inputs = &function.sig.inputs;
    let output = &function.sig.output;
    let path_struct = format_ident!("__path_{}", name);
    Ok(quote! {
        #function

        #(#carried)*
        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        #visibility struct #name {}

        #(#cfgs)*
        #[allow(deprecated)]
        impl ::autoxide_kernel::http::Endpoint for #name {
            fn __describe() -> ::autoxide_kernel::http::EndpointDoc {
                use ::autoxide_kernel::__private::utoipa;

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
                    operation
                        .tags
                        .get_or_insert_with(::std::vec::Vec::new)
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
