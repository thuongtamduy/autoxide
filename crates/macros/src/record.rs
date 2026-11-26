//! `#[derive(Record)]`: checks, at compile time, the rules a module table must follow, then writes the
//! `Record` impl and one typed column constant per field.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, Ident, LitStr, Type};

/// The two scopes a record can declare.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Tenant,
    Global,
}

/// What `#[record(…)]` on the struct says.
struct Options {
    table: LitStr,
    scope: Scope,
}

pub(crate) fn expand(input: &DeriveInput) -> syn::Result<TokenStream> {
    let options = options(input)?;
    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => &fields.named,
            _ => {
                return Err(syn::Error::new_spanned(
                    &input.ident,
                    "`Record` needs a struct with named fields",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "`Record` needs a struct with named fields",
            ));
        }
    };
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "`Record` does not support generic parameters",
        ));
    }
    let tracked_manifest = check_table(&options.table)?;

    // `r#tenant_id` cũng là cột `tenant_id`: so theo tên đã bỏ `r#`, và không cho viết dạng thô.
    let tenant_field = fields
        .iter()
        .find(|field| field.ident.as_ref().is_some_and(|name| unraw(name) == "tenant_id"));
    if let Some(ident) = tenant_field.and_then(|field| field.ident.as_ref())
        && ident.to_string().starts_with("r#")
    {
        return Err(syn::Error::new_spanned(
            ident,
            "write the tenant column as `tenant_id`, not `r#tenant_id`",
        ));
    }
    match (options.scope, tenant_field) {
        (Scope::Global, Some(field)) => {
            return Err(syn::Error::new_spanned(
                field,
                "a record with a `tenant_id` field is a tenant table: declare `scope = tenant`",
            ));
        }
        (Scope::Tenant, None) => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "a `scope = tenant` record needs a field `tenant_id: TenantKey`; every record of a tenant table — \
                 even one that reads only a few columns — is `scope = tenant` with `tenant_id`",
            ));
        }
        (Scope::Tenant, Some(field)) if !is_tenant_key(&field.ty) => {
            return Err(syn::Error::new_spanned(
                &field.ty,
                "`tenant_id` of a tenant record has the type `TenantKey` (readable, never written by `set`)",
            ));
        }
        _ => {}
    }

    let mut primary_key = None;
    for field in fields {
        for attribute in field.attrs.iter().filter(|a| a.path().is_ident("record")) {
            attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("primary_key") {
                    if primary_key.is_some() {
                        return Err(meta.error("only one field is the primary key"));
                    }
                    primary_key = field.ident.clone();
                    Ok(())
                } else {
                    Err(meta.error("unknown field option; the only one is `primary_key`"))
                }
            })?;
        }
    }

    let name = &input.ident;
    let vis = &input.vis;
    let field_names: Vec<&Ident> = fields.iter().filter_map(|field| field.ident.as_ref()).collect();
    let field_types: Vec<&Type> = fields.iter().map(|field| &field.ty).collect();
    let column_names: Vec<String> = field_names.iter().map(|ident| unraw(ident)).collect();
    let constants: Vec<Ident> =
        column_names.iter().map(|column| format_ident!("{}", column.to_uppercase())).collect();
    let indexes: Vec<usize> = (0..field_names.len()).collect();
    let table = &options.table;
    let scope = match options.scope {
        Scope::Tenant => quote!(::autoxide_kernel::db::Tenant),
        Scope::Global => quote!(::autoxide_kernel::db::Global),
    };
    // Gọi qua đường đầy đủ của kernel: trường `tenant_id` mang kiểu khác `TenantKey` của kernel là lỗi biên dịch.
    let tenant_of = match options.scope {
        Scope::Tenant => {
            quote!(::core::option::Option::Some(::autoxide_kernel::db::TenantKey::as_uuid(
                self.tenant_id
            )))
        }
        Scope::Global => quote!(::core::option::Option::None),
    };
    let primary_key = match primary_key {
        Some(ident) => {
            let column = unraw(&ident);
            quote!(::core::option::Option::Some(#column))
        }
        None => quote!(::core::option::Option::None),
    };
    // Bảng có cột `id` (UUIDv7, luật dữ liệu) thì phân trang theo id được (`Find::page`).
    let has_id = if column_names.iter().any(|column| column == "id") {
        quote! {
            impl ::autoxide_kernel::db::HasId for #name {
                fn id(&self) -> ::autoxide_kernel::__private::Uuid {
                    self.id
                }
            }
        }
    } else {
        quote!()
    };
    let nullable: Vec<bool> = field_types.iter().map(|ty| is_option(ty)).collect();
    // Crate ngoài khai tên module trong `Cargo.toml`: đọc file đó qua `include_str!` để Cargo biên dịch lại khi nó đổi.
    let track = if tracked_manifest {
        quote!(
            const _: &str = ::core::include_str!(::core::concat!(
                ::core::env!("CARGO_MANIFEST_DIR"),
                "/Cargo.toml"
            ));
        )
    } else {
        quote!()
    };
    let docs: Vec<String> =
        column_names.iter().map(|column| format!("Column `{column}`.")).collect();

    Ok(quote! {
        impl #name {
            #(
                #[doc = #docs]
                #[allow(dead_code)]
                #vis const #constants: ::autoxide_kernel::db::Col<#name, #field_types> =
                    ::autoxide_kernel::db::Col::new(#column_names);
            )*
        }

        #has_id
        #track

        // Chỉ derive sinh dòng này (gate `private-paths` chặn viết tay `__private`): `Record` không cài tay được.
        impl ::autoxide_kernel::__private::RecordSeal for #name {}

        impl ::autoxide_kernel::db::Record for #name {
            const TABLE: &'static str = #table;
            type Scope = #scope;
            const COLUMNS: &'static [&'static str] = &[#(#column_names),*];
            const NULLABLE: &'static [bool] = &[#(#nullable),*];
            const PRIMARY_KEY: ::core::option::Option<&'static str> = #primary_key;

            fn tenant_of(&self) -> ::core::option::Option<::autoxide_kernel::__private::Uuid> {
                #tenant_of
            }

            fn read(
                row: &::autoxide_kernel::db::Row<'_>,
            ) -> ::core::result::Result<Self, ::autoxide_kernel::error::Error> {
                ::core::result::Result::Ok(#name {
                    #( #field_names: <#field_types as ::autoxide_kernel::db::DbValue>::read(row, #indexes)?, )*
                })
            }

            fn values(self) -> ::std::vec::Vec<::autoxide_kernel::db::Value> {
                ::std::vec![#( <#field_types as ::autoxide_kernel::db::DbValue>::into_value(self.#field_names) ),*]
            }
        }
    })
}

/// Reads `#[record(table = "…", scope = tenant | global)]`; both are required.
fn options(input: &DeriveInput) -> syn::Result<Options> {
    let mut table = None;
    let mut scope = None;
    for attribute in input.attrs.iter().filter(|a| a.path().is_ident("record")) {
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("table") {
                table = Some(meta.value()?.parse::<LitStr>()?);
                Ok(())
            } else if meta.path.is_ident("scope") {
                let value: Ident = meta.value()?.parse()?;
                scope = Some(match value.to_string().as_str() {
                    "tenant" => Scope::Tenant,
                    "global" => Scope::Global,
                    _ => return Err(syn::Error::new_spanned(value, "`scope` is `tenant` or `global`")),
                });
                Ok(())
            } else {
                Err(meta.error("unknown option; `#[record(…)]` takes `table = \"…\"` and `scope = tenant | global`"))
            }
        })?;
    }
    let table = table.ok_or_else(|| {
        syn::Error::new_spanned(
            &input.ident,
            "add `#[record(table = \"<module>_<name>\", scope = tenant)]`",
        )
    })?;
    let scope = scope.ok_or_else(|| {
        syn::Error::new_spanned(
            &input.ident,
            "add `scope = tenant` to `#[record(…)]` for a table with `tenant_id` (every tenant table); \
             `scope = global` is only for a control-plane table the module lists in `control_plane_tables`",
        )
    })?;
    Ok(Options { table, scope })
}

/// Architecture rule 1 at compile time: a module declares records only for its own tables, named
/// `<module>_…`, where `<module>` comes from the crate being compiled.
fn check_table(table: &LitStr) -> syn::Result<bool> {
    let (module, from_manifest) =
        module_name().map_err(|message| syn::Error::new(table.span(), message))?;
    let value = table.value();
    let prefix = format!("{module}_");
    if value.starts_with(&prefix) && value.len() > prefix.len() {
        Ok(from_manifest)
    } else {
        Err(syn::Error::new(
            table.span(),
            format!(
                "table `{value}` does not belong to module `{module}`: a module declares records only for its own \
                 tables, named `{prefix}…` (architecture rule 1)"
            ),
        ))
    }
}

/// The module of the crate being compiled: `autoxide-<name>` gives `<name>` (`-` becomes `_`, so
/// `autoxide-einvoice-sepay` is `einvoice_sepay`); any other crate declares it in its `Cargo.toml`:
/// `[package.metadata.autoxide] module = "<name>"`.
fn module_name() -> Result<(String, bool), String> {
    // Cargo đặt các biến này cho mỗi lần biên dịch một crate; macro chạy trong tiến trình đó nên đọc được.
    let package = std::env::var("CARGO_PKG_NAME")
        .map_err(|_| "CARGO_PKG_NAME is not set; build with cargo".to_owned())?;
    if let Some(name) = package.strip_prefix("autoxide-") {
        return Ok((name.replace('-', "_"), false));
    }
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let manifest = std::fs::read_to_string(std::path::Path::new(&manifest_dir).join("Cargo.toml"))
        .unwrap_or_default();
    metadata_module(&manifest).map(|name| (name, true)).ok_or_else(|| {
        format!(
            "crate `{package}` is not named `autoxide-<module>`: declare its module in Cargo.toml, \
             `[package.metadata.autoxide]` then `module = \"<name>\"`"
        )
    })
}

/// `module = "…"` under `[package.metadata.autoxide]`. A small reader instead of a TOML library: the macro
/// crate stays light and only this one key is needed.
fn metadata_module(manifest: &str) -> Option<String> {
    let mut in_section = false;
    for line in manifest.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.starts_with('[') {
            in_section = line == "[package.metadata.autoxide]";
            continue;
        }
        if !in_section {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "module" {
            let value = value.trim().trim_matches('"');
            let valid = !value.is_empty()
                && value.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            return valid.then(|| value.to_owned());
        }
    }
    None
}

/// `Option<…>` (by its last path segment): a nullable column.
fn is_option(ty: &Type) -> bool {
    match ty {
        Type::Path(path) => {
            path.path.segments.last().is_some_and(|segment| segment.ident == "Option")
        }
        _ => false,
    }
}

fn is_tenant_key(ty: &Type) -> bool {
    match ty {
        Type::Path(path) => {
            path.path.segments.last().is_some_and(|segment| segment.ident == "TenantKey")
        }
        _ => false,
    }
}

/// Field name without the `r#` of a raw identifier (`r#type` is column `type`).
fn unraw(ident: &Ident) -> String {
    let text = ident.to_string();
    text.strip_prefix("r#").map_or(text.clone(), str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proc_macro2::Span;

    #[test]
    fn doc_ten_module_tu_metadata() {
        let manifest = "[package]\nname = \"bookmarks\"\n\n[package.metadata.autoxide]\n\n# chú thích\nmodule = \"bookmarks\" # x\n";
        assert_eq!(metadata_module(manifest).as_deref(), Some("bookmarks"));
        assert_eq!(metadata_module("[package]\nname = \"x\"\n"), None);
        assert_eq!(metadata_module("[package.metadata.autoxide]\nmodule = \"Bad-Name\"\n"), None);
    }

    #[test]
    fn ten_truong_tho() {
        let ident = Ident::new_raw("type", Span::call_site());
        assert_eq!(unraw(&ident), "type");
    }
}
