//! What `#[derive(Validate)]` calls: the five rules (`length`, `range`, `email`, `url`, `nested`) and the error
//! collector. Hidden (`__private::check`): only generated code uses it, so the rules and their errors live in the
//! kernel and not in a library a module would have to declare (D-20261128-library-layer-plan, item 2).

// Ghi chú cho chủ dự án: trước đây derive của kernel sinh mã của thư viện `validator`. Giờ nó gọi các hàm ở đây;
// lỗi trả về giữ nguyên (mã, tham số, câu, đường dẫn trường) vì cùng đi qua `error::describe_field`. Bên trong,
// email và URL vẫn kiểm bằng hàm của `validator` cho đúng y luật cũ — đó là chi tiết cài đặt, đổi được bất cứ lúc
// nào mà module không biết.

use std::borrow::Cow;
use std::collections::HashMap;

use serde_json::Value;

use crate::error::{Error, FieldError, describe_field};

pub use serde_json::json;

/// The errors of one input, collected field by field.
#[derive(Debug, Default)]
pub struct Errors {
    fields: Vec<FieldError>,
}

impl Errors {
    pub fn new() -> Self {
        Errors::default()
    }

    /// Field `field` breaks rule `code`; `params` are the rule's bounds (and the offending `value`).
    pub fn fail(
        &mut self,
        field: &str,
        code: &str,
        message: Option<&str>,
        params: &[(&str, Value)],
    ) {
        let params: HashMap<&str, &Value> =
            params.iter().map(|(name, value)| (*name, value)).collect();
        self.fields.push(describe_field(field.to_owned(), code, message, |name| {
            params.get(name).copied()
        }));
    }

    /// Whether field `field` already has an error (a nested check is skipped then, as `validator` did).
    pub fn has(&self, field: &str) -> bool {
        self.fields.iter().any(|error| error.field == field)
    }

    /// The errors of a nested input under `field` (`field.inner`).
    pub fn nested<T: super::Validate + ?Sized>(&mut self, field: &str, value: &T) {
        self.prefixed(field, value.check());
    }

    /// The errors of every item of a nested list under `field[index]`.
    pub fn nested_list<T: super::Validate>(&mut self, field: &str, items: &[T]) {
        for (index, item) in items.iter().enumerate() {
            self.prefixed(&format!("{field}[{index}]"), item.check());
        }
    }

    fn prefixed(&mut self, prefix: &str, result: Result<(), Error>) {
        let Err(error) = result else { return };
        let fields = error.fields();
        if fields.is_empty() {
            // Không phải lỗi đầu vào (không nên xảy ra với derive): giữ lại dưới tên trường thay vì bỏ lặng lẽ.
            self.fields.push(FieldError::new(prefix.to_owned(), "invalid", "is not valid"));
            return;
        }
        for inner in fields {
            let mut inner = inner.clone();
            inner.field = format!("{prefix}.{}", inner.field);
            self.fields.push(inner);
        }
    }

    /// `Ok` when nothing failed, otherwise `validation_failed` with every field (sorted, as before).
    pub fn finish(mut self) -> Result<(), Error> {
        if self.fields.is_empty() {
            return Ok(());
        }
        self.fields.sort_by(|a, b| (&a.field, &a.code).cmp(&(&b.field, &b.code)));
        Err(Error::validation(self.fields))
    }
}

/// Things whose length `length(…)` measures: characters of a text, items of a list.
pub trait HasLength {
    fn length(&self) -> u64;
}

impl HasLength for str {
    fn length(&self) -> u64 {
        self.chars().count() as u64
    }
}

impl HasLength for String {
    fn length(&self) -> u64 {
        self.as_str().length()
    }
}

impl HasLength for Cow<'_, str> {
    fn length(&self) -> u64 {
        self.as_ref().length()
    }
}

impl<T> HasLength for [T] {
    fn length(&self) -> u64 {
        self.len() as u64
    }
}

impl<T> HasLength for Vec<T> {
    fn length(&self) -> u64 {
        self.len() as u64
    }
}

impl<K, V, S> HasLength for HashMap<K, V, S> {
    fn length(&self) -> u64 {
        self.len() as u64
    }
}

impl<T: HasLength + ?Sized> HasLength for &T {
    fn length(&self) -> u64 {
        (**self).length()
    }
}

/// `length(min, max, equal)`: `equal` wins over the bounds (as `validator`).
pub fn length<T: HasLength + ?Sized>(
    value: &T,
    min: Option<u64>,
    max: Option<u64>,
    equal: Option<u64>,
) -> bool {
    let length = value.length();
    if let Some(equal) = equal {
        return length == equal;
    }
    min.is_none_or(|min| length >= min) && max.is_none_or(|max| length <= max)
}

/// `range(min, max, exclusive_min, exclusive_max)`.
pub fn range<T: PartialOrd>(
    value: T,
    min: Option<T>,
    max: Option<T>,
    exclusive_min: Option<T>,
    exclusive_max: Option<T>,
) -> bool {
    min.is_none_or(|min| value >= min)
        && max.is_none_or(|max| value <= max)
        && exclusive_min.is_none_or(|min| value > min)
        && exclusive_max.is_none_or(|max| value < max)
}

/// `email`: the HTML5 rule (an address `<input type="email">` accepts), as before.
pub fn email(value: &str) -> bool {
    validator::ValidateEmail::validate_email(&value)
}

/// `url`: an absolute URL, as before.
pub fn url(value: &str) -> bool {
    validator::ValidateUrl::validate_url(&value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_dem_ky_tu_khong_dem_byte() {
        assert!(length("Đà Lạt", Some(6), Some(6), None));
        assert!(!length("", Some(1), None, None));
        assert!(length(&vec![1, 2], None, None, Some(2)));
        assert!(!length("abc", Some(1), Some(5), Some(2)));
    }

    #[test]
    fn range_bien_mo_va_dong() {
        assert!(range(5, Some(1), Some(5), None, None));
        assert!(!range(5, None, None, None, Some(5)));
        assert!(!range(0.0, None, None, Some(0.0), None));
    }

    #[test]
    fn loi_long_nhau_co_tien_to_va_sap_xep() {
        let mut errors = Errors::new();
        errors.fail("title", "length", None, &[("max", json!(5)), ("value", json!("toolong"))]);
        assert!(errors.has("title"));
        let Err(error) = errors.finish() else { panic!("expected an error") };
        let field = &error.fields()[0];
        assert_eq!((field.field.as_str(), field.code.as_str()), ("title", "length"));
        assert_eq!(field.message, "must be at most 5 characters long");
    }
}
