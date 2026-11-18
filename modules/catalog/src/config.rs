//! Cấu hình của module `catalog`, tiền tố `AUTOXIDE_CATALOG_`. Mọi biến có mặc định chạy được
//! ngay. Ngôn ngữ và tiền tệ của từng cửa hàng lấy từ cài đặt kế thừa theo cây (bước 34), không
//! từ biến môi trường.

use autoxide_kernel::config::{ConfigReader, Invalid, Setting};

pub(crate) const LOCALES: Setting = Setting::new(
    "AUTOXIDE_CATALOG_LOCALES",
    "Languages product and category texts may be written in, comma-separated BCP 47 codes \
     (e.g. `en,vi`).",
)
.with_default("en,vi");

pub(crate) const PRODUCT_PATH_PREFIX: Setting = Setting::new(
    "AUTOXIDE_CATALOG_PRODUCT_PATH_PREFIX",
    "First segment of the public address of every product: `products` gives \
     `/products/<slug>`. Lower-case letters, digits and `-`.",
)
.with_default("products");

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CatalogConfig {
    pub locales: Vec<String>,
    pub product_path_prefix: String,
}

impl Default for CatalogConfig {
    fn default() -> Self {
        CatalogConfig {
            locales: vec!["en".to_owned(), "vi".to_owned()],
            product_path_prefix: "products".to_owned(),
        }
    }
}

impl CatalogConfig {
    pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<Self, Invalid> {
        let locales = reader.get_with(&LOCALES, |raw| {
            let mut locales = Vec::new();
            for item in raw.split(',').map(str::trim).filter(|item| !item.is_empty()) {
                let locale = canonical_locale(item).ok_or_else(|| {
                    format!("`{item}` is not a language code like `en` or `pt-BR`")
                })?;
                if !locales.contains(&locale) {
                    locales.push(locale);
                }
            }
            if locales.is_empty() {
                Err("must list at least one language".to_owned())
            } else {
                Ok(locales)
            }
        });
        let product_path_prefix = reader.get_with(&PRODUCT_PATH_PREFIX, |raw| {
            let prefix = raw.trim().trim_matches('/');
            if autoxide_kernel::slug::is_valid(prefix) {
                Ok(prefix.to_owned())
            } else {
                Err("must be lower-case letters, digits and `-`, like `products`".to_owned())
            }
        });
        Ok(CatalogConfig { locales: locales?, product_path_prefix: product_path_prefix? })
    }

    /// `locale` đã chuẩn hoá nếu nằm trong ngôn ngữ cấu hình.
    pub(crate) fn locale(&self, locale: &str) -> Option<String> {
        canonical_locale(locale).filter(|locale| self.locales.contains(locale))
    }
}

/// Mã ngôn ngữ BCP 47 dạng đơn giản `ll` hay `ll-RR`, chuẩn hoá hoa thường (`pt-br` → `pt-BR`).
pub(crate) fn canonical_locale(raw: &str) -> Option<String> {
    let mut parts = raw.trim().split(['-', '_']);
    let language = parts.next()?;
    if !(2..=3).contains(&language.len()) || !language.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let mut out = language.to_ascii_lowercase();
    if let Some(region) = parts.next() {
        if region.len() != 2 || !region.bytes().all(|b| b.is_ascii_alphabetic()) {
            return None;
        }
        out.push('-');
        out.push_str(&region.to_ascii_uppercase());
    }
    if parts.next().is_some() {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::canonical_locale;

    #[test]
    fn chuan_hoa_ma_ngon_ngu() {
        assert_eq!(canonical_locale("vi").as_deref(), Some("vi"));
        assert_eq!(canonical_locale("pt_br").as_deref(), Some("pt-BR"));
        assert_eq!(canonical_locale("vietnamese"), None);
        assert_eq!(canonical_locale("en-US-x"), None);
    }
}
