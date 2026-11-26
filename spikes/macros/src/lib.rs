//! SPIKE — never merged. `#[derive(Schema)]`: the module writes utoipa's attribute grammar (`#[schema(…)]`,
//! `#[serde(…)]`, doc comments, `#[deprecated]`) but never names `utoipa`. The derive emits a private "shadow"
//! copy of the type that derives `utoipa::ToSchema` inside a block where `utoipa` is imported from
//! `::autoxide_kernel::__private`, then implements `PartialSchema` / `ToSchema` for the real type by
//! delegating to the shadow (same schema, same name, same nested schemas).

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{Attribute, Data, DeriveInput, Fields, parse_macro_input};

/// Attributes utoipa reads; everything else (other derives, `allow`…) is dropped from the shadow.
fn kept(attribute: &Attribute) -> bool {
    let path = attribute.path();
    path.is_ident("doc")
        || path.is_ident("serde")
        || path.is_ident("schema")
        || path.is_ident("deprecated")
        || path.is_ident("cfg")
}

fn keep(attributes: &[Attribute]) -> Vec<Attribute> {
    attributes.iter().filter(|a| kept(a)).cloned().collect()
}

fn has_as(attributes: &[Attribute]) -> bool {
    attributes.iter().filter(|a| a.path().is_ident("schema")).any(|a| {
        let mut found = false;
        // `parse_nested_meta` đọc từng mục trong `#[schema(…)]`; mục không phải `as` thì bỏ qua phần giá trị.
        let _ = a.parse_nested_meta(|meta| {
            if meta.path.is_ident("as") {
                found = true;
            }
            if meta.input.peek(syn::Token![=]) {
                let _: syn::Expr = meta.value()?.parse()?;
            } else if meta.input.peek(syn::token::Paren) {
                let content;
                syn::parenthesized!(content in meta.input);
                let _: proc_macro2::TokenStream = content.parse()?;
            }
            Ok(())
        });
        found
    })
}

fn strip_fields(fields: &mut Fields) {
    for field in fields.iter_mut() {
        field.attrs = keep(&field.attrs);
        field.vis = syn::Visibility::Inherited;
    }
}

#[proc_macro_derive(Schema, attributes(schema))]
pub fn derive_schema(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let shadow_name = format_ident!("__AutoxideSchema{}", name);
    let mut shadow = input.clone();
    shadow.ident = shadow_name.clone();
    shadow.vis = syn::Visibility::Inherited;
    shadow.attrs = keep(&input.attrs);
    if !has_as(&input.attrs) {
        // Không có `as`: utoipa đặt tên theo tên struct — của bản sao. Gắn tên thật.
        shadow.attrs.push(syn::parse_quote!(#[schema(as = #name)]));
    }
    match &mut shadow.data {
        Data::Struct(data) => strip_fields(&mut data.fields),
        Data::Enum(data) => {
            for variant in &mut data.variants {
                variant.attrs = keep(&variant.attrs);
                strip_fields(&mut variant.fields);
            }
        }
        Data::Union(_) => {
            return syn::Error::new_spanned(name, "`Schema` does not support unions")
                .to_compile_error()
                .into();
        }
    }
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    let mut where_clause = where_clause.cloned().unwrap_or_else(|| syn::parse_quote!(where));
    where_clause.predicates.push(syn::parse_quote!(#shadow_name #type_generics: utoipa::ToSchema));
    quote! {
        #[allow(deprecated, dead_code, non_camel_case_types, clippy::all, clippy::pedantic)]
        const _: () = {
            use ::autoxide_kernel::__private::utoipa;
            #[derive(utoipa::ToSchema)]
            #shadow

            impl #impl_generics utoipa::PartialSchema for #name #type_generics #where_clause {
                fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
                    <#shadow_name #type_generics as utoipa::PartialSchema>::schema()
                }
            }

            impl #impl_generics utoipa::ToSchema for #name #type_generics #where_clause {
                fn name() -> ::std::borrow::Cow<'static, str> {
                    <#shadow_name #type_generics as utoipa::ToSchema>::name()
                }
                fn schemas(
                    schemas: &mut ::std::vec::Vec<(
                        ::std::string::String,
                        utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>,
                    )>,
                ) {
                    <#shadow_name #type_generics as utoipa::ToSchema>::schemas(schemas)
                }
            }
        };
    }
    .into()
}
