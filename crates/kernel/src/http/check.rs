//! What `#[derive(Validate)]` calls: the five rules (`length`, `range`, `email`, `url`, `nested`) and the error
//! collector. Hidden (`__private::check`): only generated code uses it, so the rules and their errors live in the
//! kernel and not in a library a module would have to declare (D-20261128-library-layer-plan, item 2).
//!
//! A `length` bound is an unsigned integer; a negative one does not compile:
//!
//! ```compile_fail
//! # use autoxide_kernel::http::Validate;
//! #[derive(Validate)]
//! struct New { #[validate(length(min = -1))] title: String }
//! ```
//!
//! Control (a literal, a `usize` constant, a block):
//!
//! ```
//! # use autoxide_kernel::http::Validate;
//! const MAX: usize = 20;
//! #[derive(Validate)]
//! struct New { #[validate(length(min = 1, max = MAX))] title: String, #[validate(length(max = { MAX }))] body: String }
//! assert!(New { title: "a".into(), body: String::new() }.check().is_ok());
//! assert!(New { title: String::new(), body: String::new() }.check().is_err());
//! ```

// Ghi chú cho chủ dự án: trước đây derive của kernel sinh mã của thư viện `validator`. Giờ nó gọi các hàm ở đây;
// lỗi trả về giữ nguyên (mã, tham số, câu, đường dẫn trường) vì cùng đi qua `error::describe_field`. Bên trong,
// email và URL vẫn kiểm bằng hàm của `validator` cho đúng y luật cũ — đó là chi tiết cài đặt, đổi được bất cứ lúc
// nào mà module không biết.

use std::borrow::Cow;
use std::collections::HashMap;

use serde_json::Value;

use crate::error::{Error, FieldError, describe_field};

/// The errors of one input, collected field by field.
#[derive(Debug, Default)]
pub struct Errors {
    fields: Vec<FieldError>,
    // Lỗi không phải lỗi đầu vào từ một `Validate` viết tay lồng bên trong: trả nguyên, không đổi thành lỗi trường.
    other: Option<Error>,
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
            if self.other.is_none() {
                self.other = Some(error);
            }
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
        if let Some(other) = self.other {
            return Err(other);
        }
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
    // Như `validator` (`!(v < min)`): không so được (NaN) thì qua cận đóng, như trước.
    use std::cmp::Ordering;
    min.is_none_or(|min| value.partial_cmp(&min) != Some(Ordering::Less))
        && max.is_none_or(|max| value.partial_cmp(&max) != Some(Ordering::Greater))
        && exclusive_min.is_none_or(|min| value > min)
        && exclusive_max.is_none_or(|max| value < max)
}

/// A rule's bound as an error parameter (JSON).
pub fn param<T: serde::Serialize + ?Sized>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// Bounds `length(…)` accepts: unsigned integers (a negative or fractional bound does not compile).
pub trait LengthBound {
    fn to_u64(self) -> u64;
}

macro_rules! length_bounds {
    ($($ty:ty),*) => {$(
        impl LengthBound for $ty {
            fn to_u64(self) -> u64 {
                u64::try_from(self).unwrap_or(u64::MAX)
            }
        }
    )*};
}

length_bounds!(u8, u16, u32, u64, usize);

/// A `length(…)` bound written as a constant or an expression.
pub fn length_bound(bound: impl LengthBound) -> u64 {
    bound.to_u64()
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
    fn nan_qua_can_dong_nhu_validator() {
        assert!(range(f64::NAN, Some(-0.5), Some(0.5), None, None));
        assert!(!range(f64::NAN, None, None, Some(0.0), None));
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
        errors.fail("title", "length", None, &[("max", param(&5))]);
        assert!(errors.has("title"));
        let Err(error) = errors.finish() else { panic!("expected an error") };
        let field = &error.fields()[0];
        assert_eq!((field.field.as_str(), field.code.as_str()), ("title", "length"));
        assert_eq!(field.message, "must be at most 5 characters long");
    }
}
