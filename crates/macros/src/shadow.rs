//! `#[derive(Schema)]` and `#[derive(Params)]` by a "shadow" copy.

// Module viết đúng ngữ pháp thuộc tính của utoipa (`#[schema(…)]`, `#[param(…)]`, `#[into_params(…)]`, `#[serde(…)]`,
// doc comment, `#[deprecated]`) nhưng không gọi tên `utoipa`. Derive sinh một bản sao riêng tư của kiểu trong khối
// `const _` có `use ::autoxide_kernel::__private::utoipa;`, cho bản sao derive của utoipa, rồi cài trait của utoipa cho
// kiểu thật bằng cách chuyển tiếp sang bản sao (cùng schema, cùng tên, cùng schema lồng). Bản sao chỉ mô tả, không
// sinh mã chạy. `utoipa::__dev::ComposeSchema` là API ẩn của utoipa: nâng utoipa thì test OpenAPI giữ từng byte phát
// hiện thay đổi (§4.2 của thiết kế G1).

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Attribute, Data, DeriveInput, Fields};

/// Which utoipa derive the shadow takes.
#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Schema,
    Params,
}

impl Kind {
    // Thuộc tính utoipa đọc; mọi thứ khác (derive khác, `validate`, `allow`…) bỏ khỏi bản sao.
    fn keeps(self, attribute: &Attribute) -> bool {
        let path = attribute.path();
        let shared = ["doc", "serde", "deprecated", "cfg"].iter().any(|name| path.is_ident(name));
        shared
            || match self {
                Kind::Schema => path.is_ident("schema"),
                Kind::Params => path.is_ident("param") || path.is_ident("into_params"),
            }
    }
}

fn keep(kind: Kind, attributes: &[Attribute]) -> Vec<Attribute> {
    attributes.iter().filter(|attribute| kind.keeps(attribute)).cloned().collect()
}

// `#[schema(as = …)]` đã có thì giữ; chưa có thì gắn tên thật (utoipa đặt tên schema theo tên struct — của bản sao).
fn has_as(attributes: &[Attribute]) -> bool {
    attributes.iter().filter(|attribute| attribute.path().is_ident("schema")).any(|attribute| {
        let mut found = false;
        let _ = attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("as") {
                found = true;
            }
            if meta.input.peek(syn::Token![=]) {
                let _: syn::Expr = meta.value()?.parse()?;
            } else if meta.input.peek(syn::token::Paren) {
                let content;
                syn::parenthesized!(content in meta.input);
                let _: TokenStream = content.parse()?;
            }
            Ok(())
        });
        found
    })
}

fn strip_fields(kind: Kind, fields: &mut Fields) {
    for field in fields.iter_mut() {
        field.attrs = keep(kind, &field.attrs);
        field.vis = syn::Visibility::Inherited;
    }
}

pub(crate) fn expand(kind: Kind, input: &DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;
    let shadow_name = format_ident!("__AutoxideShadow{}", name);
    let mut shadow = input.clone();
    shadow.ident = shadow_name.clone();
    shadow.vis = syn::Visibility::Inherited;
    shadow.attrs = keep(kind, &input.attrs);
    if matches!(kind, Kind::Schema) && !has_as(&input.attrs) {
        shadow.attrs.push(syn::parse_quote!(#[schema(as = #name)]));
    }
    match &mut shadow.data {
        Data::Struct(data) => strip_fields(kind, &mut data.fields),
        Data::Enum(data) => {
            for variant in &mut data.variants {
                variant.attrs = keep(kind, &variant.attrs);
                strip_fields(kind, &mut variant.fields);
            }
        }
        Data::Union(_) => return Err(syn::Error::new_spanned(name, "unions are not supported")),
    }
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    let base = where_clause.cloned().unwrap_or_else(|| syn::parse_quote!(where));
    let body = match kind {
        Kind::Schema => {
            let mut compose_where = base.clone();
            compose_where
                .predicates
                .push(syn::parse_quote!(#shadow_name #type_generics: utoipa::__dev::ComposeSchema));
            let mut schema_where = compose_where.clone();
            schema_where
                .predicates
                .push(syn::parse_quote!(#shadow_name #type_generics: utoipa::ToSchema));
            quote! {
                #[derive(utoipa::ToSchema)]
                #shadow

                // `PartialSchema` đến từ impl chung của utoipa cho mọi kiểu có `ComposeSchema`; utoipa dùng trait
                // ẩn này khi kiểu làm tham số generic (`Data<T>`).
                impl #impl_generics utoipa::__dev::ComposeSchema for #name #type_generics #compose_where {
                    fn compose(
                        generics: ::std::vec::Vec<utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>>,
                    ) -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
                        <#shadow_name #type_generics as utoipa::__dev::ComposeSchema>::compose(generics)
                    }
                }

                impl #impl_generics utoipa::ToSchema for #name #type_generics #schema_where {
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
            }
        }
        Kind::Params => {
            let mut params_where = base;
            params_where
                .predicates
                .push(syn::parse_quote!(#shadow_name #type_generics: utoipa::IntoParams));
            quote! {
                #[derive(utoipa::IntoParams)]
                #shadow

                impl #impl_generics utoipa::IntoParams for #name #type_generics #params_where {
                    fn into_params(
                        parameter_in_provider: impl Fn() -> ::std::option::Option<utoipa::openapi::path::ParameterIn>,
                    ) -> ::std::vec::Vec<utoipa::openapi::path::Parameter> {
                        <#shadow_name #type_generics as utoipa::IntoParams>::into_params(parameter_in_provider)
                    }
                }
            }
        }
    };
    Ok(quote! {
        #[allow(deprecated, dead_code, non_camel_case_types, clippy::all, clippy::pedantic)]
        const _: () = {
            use ::autoxide_kernel::__private::utoipa;
            #body
        };
    })
}
