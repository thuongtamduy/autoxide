//! `#[derive(Validate)]`: input checks written as `#[validate(…)]` on fields, without naming `validator`.
//!
//! Rules: `length(min, max, equal)`, `range(min, max, exclusive_min, exclusive_max)`, `email`, `url`, `nested`; each
//! (but `nested`) takes `message = "…"` and `code = "…"`. They are the rules modules use; a new rule is added here.

// Mã sinh ra giống TỪNG DÒNG mã của `validator_derive` 0.20 cho cùng thuộc tính (cùng trait kiểm, cùng tham số lỗi,
// cùng cách bọc trường `Option`), chỉ khác đường dẫn: `::autoxide_kernel::__private::validator` thay `::validator`.
// Nhờ vậy lỗi trả về (mã, tham số, thứ tự) không đổi khi module chuyển từ derive của `validator` sang derive này,
// và module khác lồng (`nested`) kiểu cũ hay mới đều được.

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
    for attribute in field.attrs.iter().filter(|attribute| attribute.path().is_ident("validate")) {
        attribute.parse_nested_meta(|meta| {
            let name = meta.path.get_ident().map(ToString::to_string).unwrap_or_default();
            match name.as_str() {
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

fn check(
    rule: &str,
    bounds: &Bounds,
    call: TokenStream,
    value: &TokenStream,
    field_name: &str,
) -> TokenStream {
    let code = bounds.code.as_ref().map_or_else(|| rule.to_string(), LitStr::value);
    let message = bounds.message.as_ref().map(|message| {
        quote!(err.message = ::std::option::Option::Some(::std::borrow::Cow::from(#message));)
    });
    let params = bounds
        .values
        .iter()
        .map(|(name, value)| quote!(err.add_param(::std::borrow::Cow::from(#name), &#value);));
    quote! {
        if !#call {
            let mut err = validator::ValidationError::new(#code);
            #message
            #(#params)*
            err.add_param(::std::borrow::Cow::from("value"), &#value);
            errors.add(#field_name, err);
        }
    }
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
        if let Some(bounds) = &rules.length {
            let (min, max, equal) = (
                some_or_none(bounds, "min"),
                some_or_none(bounds, "max"),
                some_or_none(bounds, "equal"),
            );
            let call = quote!(#value.validate_length(#min, #max, #equal));
            field_checks.push(check("length", bounds, call, &value, &field_name));
        }
        if let Some(bounds) = &rules.email {
            field_checks.push(check(
                "email",
                bounds,
                quote!(#value.validate_email()),
                &value,
                &field_name,
            ));
        }
        if let Some(bounds) = &rules.url {
            field_checks.push(check(
                "url",
                bounds,
                quote!(#value.validate_url()),
                &value,
                &field_name,
            ));
        }
        if let Some(bounds) = &rules.range {
            let (min, max) = (some_or_none(bounds, "min"), some_or_none(bounds, "max"));
            let (exclusive_min, exclusive_max) =
                (some_or_none(bounds, "exclusive_min"), some_or_none(bounds, "exclusive_max"));
            let call = quote!(#value.validate_range(#min, #max, #exclusive_min, #exclusive_max));
            field_checks.push(check("range", bounds, call, &value, &field_name));
        }
        if rules.nested {
            field_checks.push(quote! {
                if let ::std::collections::hash_map::Entry::Vacant(_) =
                    errors.0.entry(::std::borrow::Cow::Borrowed(#field_name))
                {
                    errors.merge_self(#field_name, validator::Validate::validate(&#value));
                }
            });
        }
        checks.extend(field_checks.into_iter().map(&wrap));
    }
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        #[allow(clippy::all, clippy::pedantic, unused_imports)]
        const _: () = {
            use ::autoxide_kernel::__private::validator;
            use validator::{ValidateEmail as _, ValidateLength as _, ValidateRange as _, ValidateUrl as _};

            impl #impl_generics validator::Validate for #name #type_generics #where_clause {
                fn validate(&self) -> ::std::result::Result<(), validator::ValidationErrors> {
                    let mut errors = validator::ValidationErrors::new();
                    #(#checks)*
                    if errors.is_empty() {
                        ::std::result::Result::Ok(())
                    } else {
                        ::std::result::Result::Err(errors)
                    }
                }
            }
        };
    })
}
