//! `#[derive(Validate)]`: input checks written as `#[validate(…)]` on fields, without naming `validator`.
//!
//! Rules: `length(min, max, equal)`, `range(min, max, exclusive_min, exclusive_max)`, `email`, `url`, `nested`; each
//! (but `nested`) takes `message = "…"` and `code = "…"`. They are the rules modules use; a new rule is added here.

// Mã sinh ra cài thẳng `autoxide_kernel::http::Validate` và gọi hàm kiểm của kernel (`__private::check`), không còn
// sinh mã của `validator_derive` (D-20261128-library-layer-plan mục 2). Luật giữ như `validator_derive` 0.20 (cùng
// cách bọc trường `Option`, cùng tham số lỗi, `nested` bỏ qua trường đã có lỗi) nên lỗi trả về (mã, tham số, câu,
// đường dẫn) không đổi; test `http::input::tests` của kernel so với derive của `validator`.

use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{Data, DeriveInput, Expr, Fields, LitStr};

const NUMBER_TYPES: [&str; 14] = [
    "usize", "u8", "u16", "u32", "u64", "u128", "isize", "i8", "i16", "i32", "i64", "i128", "f32",
    "f64",
];
const OPTION_PATHS: [&str; 3] = ["Option|", "std|option|Option|", "core|option|Option|"];

#[derive(Default)]
struct Bounds {
    values: Vec<(String, Expr)>,
    message: Option<LitStr>,
    code: Option<LitStr>,
}

#[derive(Default)]
struct FieldRules {
    length: Option<Bounds>,
    email: Option<Bounds>,
    url: Option<Bounds>,
    range: Option<Bounds>,
    nested: bool,
}

fn parse_bounds(meta: &syn::meta::ParseNestedMeta<'_>, allowed: &[&str]) -> syn::Result<Bounds> {
    let mut bounds = Bounds::default();
    if !meta.input.peek(syn::token::Paren) {
        return Ok(bounds);
    }
    meta.parse_nested_meta(|inner| {
        let name = inner.path.get_ident().map(ToString::to_string).unwrap_or_default();
        if name == "message" {
            bounds.message = Some(inner.value()?.parse()?);
        } else if name == "code" {
            bounds.code = Some(inner.value()?.parse()?);
        } else if allowed.contains(&name.as_str()) {
            bounds.values.push((name, inner.value()?.parse()?));
        } else {
            let mut expected: Vec<&str> = allowed.to_vec();
            expected.extend(["message", "code"]);
            return Err(
                inner.error(format!("unknown option; expected one of: {}", expected.join(", ")))
            );
        }
        Ok(())
    })?;
    Ok(bounds)
}

fn parse_rules(field: &syn::Field) -> syn::Result<FieldRules> {
    let mut rules = FieldRules::default();
    let mut seen = std::collections::BTreeSet::new();
    for attribute in field.attrs.iter().filter(|attribute| attribute.path().is_ident("validate")) {
        attribute.parse_nested_meta(|meta| {
            let name = meta.path.get_ident().map(ToString::to_string).unwrap_or_default();
            match name.as_str() {
                "length" | "range" | "email" | "url" | "nested" if !seen.insert(name.clone()) => {
                    // `validator` cũng chỉ giữ một luật mỗi loại; viết hai lần là nhầm, báo ngay thay vì bỏ lặng lẽ.
                    return Err(meta.error(format!("`{name}` is declared twice on this field; keep one")));
                }
                "length" => rules.length = Some(parse_bounds(&meta, &["min", "max", "equal"])?),
                "range" => {
                    rules.range =
                        Some(parse_bounds(&meta, &["min", "max", "exclusive_min", "exclusive_max"])?);
                }
                "email" => rules.email = Some(parse_bounds(&meta, &[])?),
                "url" => rules.url = Some(parse_bounds(&meta, &[])?),
                "nested" => rules.nested = true,
                _ => {
                    return Err(meta.error(
                        "unknown rule; `#[validate(…)]` takes `length`, `range`, `email`, `url` and `nested`",
                    ));
                }
            }
            Ok(())
        })?;
    }
    Ok(rules)
}

// Số lớp `Option` bọc kiểu (0, 1 hay 2), như `validator_derive`.
fn option_depth(ty: &syn::Type) -> u8 {
    let mut depth = 0;
    let mut current = ty;
    while let syn::Type::Path(path) = current {
        let joined: String =
            path.path.segments.iter().map(|segment| format!("{}|", segment.ident)).collect();
        if !OPTION_PATHS.contains(&joined.as_str()) {
            break;
        }
        depth += 1;
        let inner = path.path.segments.first().and_then(|segment| match &segment.arguments {
            syn::PathArguments::AngleBracketed(arguments) => arguments.args.first(),
            _ => None,
        });
        match inner {
            Some(syn::GenericArgument::Type(inner)) => current = inner,
            _ => break,
        }
    }
    depth
}

fn is_number(ty: &syn::Type) -> bool {
    let text = ty.to_token_stream().to_string();
    NUMBER_TYPES.iter().any(|number| {
        let plain = quote::format_ident!("{}", number);
        [quote!(#plain), quote!(Option<#plain>), quote!(Option<Option<#plain> >)]
            .iter()
            .any(|candidate| candidate.to_string() == text)
    })
}

fn check(rule: &str, bounds: &Bounds, call: TokenStream, field_name: &str) -> TokenStream {
    let code = bounds.code.as_ref().map_or_else(|| rule.to_string(), LitStr::value);
    let message = match &bounds.message {
        Some(message) => quote!(::std::option::Option::Some(#message)),
        None => quote!(::std::option::Option::None),
    };
    let params = bounds.values.iter().map(|(name, value)| quote!((#name, check::json!(#value))));
    quote! {
        if !#call {
            errors.fail(#field_name, #code, #message, &[#(#params),*]);
        }
    }
}

// Kiểu ngoài cùng (sau khi bỏ `Option`) là `Vec<…>`: `nested` kiểm từng phần tử.
fn is_vec(ty: &syn::Type) -> bool {
    let mut current = ty;
    loop {
        let syn::Type::Path(path) = current else { return false };
        let Some(last) = path.path.segments.last() else { return false };
        let inner = match &last.arguments {
            syn::PathArguments::AngleBracketed(arguments) => match arguments.args.first() {
                Some(syn::GenericArgument::Type(inner)) => Some(inner),
                _ => None,
            },
            _ => None,
        };
        match (last.ident.to_string().as_str(), inner) {
            ("Option", Some(inner)) => current = inner,
            ("Vec", _) => return true,
            _ => return false,
        }
    }
}

fn cast_u64(value: TokenStream) -> TokenStream {
    // `Some(1)` / `None` -> `Some((1) as u64)`; hằng kiểu `usize` của module cũng dùng được.
    let text = value.to_string();
    if text.contains("None") {
        return value;
    }
    quote!(#value.map(|bound| bound as u64))
}

fn some_or_none(bounds: &Bounds, name: &str) -> TokenStream {
    match bounds.values.iter().find(|(key, _)| key == name) {
        Some((_, value)) => quote!(::std::option::Option::Some(#value)),
        None => quote!(::std::option::Option::None),
    }
}

pub(crate) fn expand(input: &DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;
    if let Some(attribute) =
        input.attrs.iter().find(|attribute| attribute.path().is_ident("validate"))
    {
        return Err(syn::Error::new_spanned(
            attribute,
            "`#[validate(…)]` goes on fields, not on the type",
        ));
    }
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(name, "`Validate` is derived for structs"));
    };
    let fields = match &data.fields {
        Fields::Named(fields) => fields.named.iter().collect::<Vec<_>>(),
        Fields::Unit => Vec::new(),
        Fields::Unnamed(_) => {
            return Err(syn::Error::new_spanned(name, "`Validate` needs named fields"));
        }
    };
    let mut checks = Vec::new();
    for field in fields {
        let rules = parse_rules(field)?;
        let Some(ident) = &field.ident else { continue };
        let field_name = ident.to_string();
        let depth = option_depth(&field.ty);
        let value = if depth > 0 { quote!(#ident) } else { quote!(self.#ident) };
        let binding = if is_number(&field.ty) { quote!(#ident) } else { quote!(ref #ident) };
        let wrap = |tokens: TokenStream| match depth {
            0 => tokens,
            1 => quote!(if let ::std::option::Option::Some(#binding) = self.#ident { #tokens }),
            _ => {
                quote!(if let ::std::option::Option::Some(::std::option::Option::Some(#binding)) = self.#ident {
                    #tokens
                })
            }
        };
        if depth > 2 {
            return Err(syn::Error::new_spanned(
                &field.ty,
                "at most two `Option` around a checked value",
            ));
        }
        let mut field_checks = Vec::new();
        // Tham chiếu tới giá trị: trường thường thì `&self.x`; trong nhánh `Some(ref x)` thì `x` đã là tham chiếu.
        let reference = if depth > 0 { quote!(#ident) } else { quote!(&self.#ident) };
        if let Some(bounds) = &rules.length {
            let (min, max, equal) = (
                cast_u64(some_or_none(bounds, "min")),
                cast_u64(some_or_none(bounds, "max")),
                cast_u64(some_or_none(bounds, "equal")),
            );
            let call = quote!(check::length(#reference, #min, #max, #equal));
            field_checks.push(check("length", bounds, call, &field_name));
        }
        if let Some(bounds) = &rules.email {
            let call = quote!(check::email(::std::convert::AsRef::<str>::as_ref(#reference)));
            field_checks.push(check("email", bounds, call, &field_name));
        }
        if let Some(bounds) = &rules.url {
            let call = quote!(check::url(::std::convert::AsRef::<str>::as_ref(#reference)));
            field_checks.push(check("url", bounds, call, &field_name));
        }
        if let Some(bounds) = &rules.range {
            let (min, max) = (some_or_none(bounds, "min"), some_or_none(bounds, "max"));
            let (exclusive_min, exclusive_max) =
                (some_or_none(bounds, "exclusive_min"), some_or_none(bounds, "exclusive_max"));
            let call = quote!(check::range(#value, #min, #max, #exclusive_min, #exclusive_max));
            field_checks.push(check("range", bounds, call, &field_name));
        }
        if rules.nested {
            let call = if is_vec(&field.ty) {
                quote!(errors.nested_list(#field_name, #reference))
            } else {
                quote!(errors.nested(#field_name, #reference))
            };
            field_checks.push(quote! {
                if !errors.has(#field_name) {
                    #call;
                }
            });
        }
        checks.extend(field_checks.into_iter().map(&wrap));
    }
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        #[allow(clippy::all, clippy::pedantic, unused_imports)]
        const _: () = {
            use ::autoxide_kernel::__private::check;

            impl #impl_generics ::autoxide_kernel::http::Validate for #name #type_generics #where_clause {
                fn check(&self) -> ::std::result::Result<(), ::autoxide_kernel::error::Error> {
                    let mut errors = check::Errors::new();
                    #(#checks)*
                    errors.finish()
                }
            }
        };
    })
}
