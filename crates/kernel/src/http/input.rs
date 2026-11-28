//! A handler's inputs besides the JSON body ([`ValidJson`](super::ValidJson)): [`Path`], [`Query`], [`State`], and
//! the [`Validate`] check every body and tool input passes.

// Ba extractor bọc extractor cùng tên của axum: module không khai `axum`, kernel đổi được thư viện bên dưới. Lỗi đọc
// đầu vào trả nguyên response của axum; lớp `problem` của kernel đổi nó thành `Problem` như trước (không đổi hành vi).
// utoipa (feature `axum_extras`) nhận extractor theo TÊN kiểu (`Path`, `Query`), nên tài liệu OpenAPI không đổi.

use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;

use crate::error::Error;

/// The path parameters of the request (`/{id}`), read as `T`: `Path(id): Path<Uuid>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Path<T>(pub T);

impl<S, T> FromRequestParts<S> for Path<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        axum::extract::Path::<T>::from_request_parts(parts, state)
            .await
            .map(|axum::extract::Path(value)| Path(value))
            .map_err(IntoResponse::into_response)
    }
}

/// The query string of the request, read as `T`: `Query(query): Query<PageQuery>`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Query<T>(pub T);

impl<S, T> FromRequestParts<S> for Query<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        axum::extract::Query::<T>::from_request_parts(parts, state)
            .await
            .map(|axum::extract::Query(value)| Query(value))
            .map_err(IntoResponse::into_response)
    }
}

/// The state given to the routes with [`Routes::with_state`](super::Routes::with_state):
/// `State(service): State<Arc<NoteService>>`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct State<T>(pub T);

impl<S, T> FromRequestParts<S> for State<T>
where
    S: Send + Sync,
    T: FromRef<S>,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(_parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Ok(State(T::from_ref(state)))
    }
}

/// An input that checks itself: derive it with `#[derive(Validate)]` and `#[validate(…)]` on fields. A JSON body
/// ([`ValidJson`](super::ValidJson)) and an MCP tool input are checked before the handler runs; every failing field
/// is reported at once (`validation_failed`).
pub trait Validate {
    /// Checks the value; the error lists every invalid field.
    fn check(&self) -> Result<(), Error>;
}

// Đường cũ: kiểu derive bằng `validator` (module chưa chuyển). `#[derive(Validate)]` của kernel cài thẳng trait này
// (hàm kiểm ở `http::check`). Gỡ impl chung khi module cuối bỏ `validator` (D-20261128-library-layer-plan mục 2).
impl<T: validator::Validate> Validate for T {
    fn check(&self) -> Result<(), Error> {
        self.validate().map_err(Error::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_macros::Validate as DeriveValidate;
    // Derive của `validator` (bản để so) gọi `.validate()` của trường lồng: cần trait của nó trong phạm vi.
    use validator::Validate as _;

    // Derive của kernel dùng đường `::autoxide_kernel`, mà trong crate kernel thì đó là chính crate (lib.rs khai
    // `extern crate self as autoxide_kernel`).
    #[derive(DeriveValidate)]
    struct New {
        #[validate(length(min = 1, max = 5))]
        title: String,
        #[validate(range(min = 1, max = 9))]
        count: Option<u32>,
        #[validate(email)]
        email: Option<String>,
    }

    #[derive(validator::Validate)]
    struct Old {
        #[validate(length(min = 1, max = 5))]
        title: String,
        #[validate(range(min = 1, max = 9))]
        count: Option<u32>,
        #[validate(email)]
        email: Option<String>,
    }

    // Lỗi cuối (`Error`: danh sách trường với mã, câu, tham số) của một đầu vào, để so hai derive.
    fn errors<T: Validate>(value: &T) -> String {
        format!("{:?}", value.check())
    }

    // Derive của kernel (hàm kiểm của kernel) cho đúng lỗi như derive của `validator` (mã, tham số, câu, đường dẫn).
    #[test]
    fn derive_cua_kernel_bao_loi_nhu_validator() {
        let cases = [
            ("", Some(0), Some("x")),
            ("toolong", Some(10), Some("a@b.vn")),
            ("ok", None, None),
            ("ok", Some(3), Some("not-an-email")),
        ];
        for (title, count, email) in cases {
            let new = New { title: title.into(), count, email: email.map(Into::into) };
            let old = Old { title: title.into(), count, email: email.map(Into::into) };
            assert_eq!(errors(&new), errors(&old), "{title}");
        }
    }

    #[derive(DeriveValidate)]
    struct NewLine {
        #[validate(length(equal = 2, message = "two letters"))]
        code: String,
        #[validate(url)]
        link: Option<String>,
    }

    #[derive(validator::Validate)]
    struct OldLine {
        #[validate(length(equal = 2, message = "two letters"))]
        code: String,
        #[validate(url)]
        link: Option<String>,
    }

    #[derive(DeriveValidate)]
    struct NewOrder {
        #[validate(nested)]
        lines: Vec<NewLine>,
        #[validate(nested)]
        first: Option<NewLine>,
        #[validate(range(min = 0, max = 100))]
        percent: i64,
    }

    #[derive(validator::Validate)]
    struct OldOrder {
        #[validate(nested)]
        lines: Vec<OldLine>,
        #[validate(nested)]
        first: Option<OldLine>,
        #[validate(range(min = 0, max = 100))]
        percent: i64,
    }

    // `nested` trên `Vec` và `Option`, `url`, `equal`, câu `message` riêng: đường lỗi (`lines[1].code`) như validator.
    #[test]
    fn derive_long_nhau_nhu_validator() {
        let line = |code: &str, link: Option<&str>| {
            (
                NewLine { code: code.into(), link: link.map(Into::into) },
                OldLine { code: code.into(), link: link.map(Into::into) },
            )
        };
        let cases = [
            (
                vec![("vn", None), ("x", Some("not a url"))],
                Some(("abc", Some("https://a.vn"))),
                101,
            ),
            (vec![("vn", Some("https://a.vn"))], None, 50),
            (Vec::new(), Some(("v", None)), -1),
        ];
        for (lines, first, percent) in cases {
            let (new_lines, old_lines): (Vec<_>, Vec<_>) =
                lines.iter().map(|(code, link)| line(code, *link)).unzip();
            let (new_first, old_first) = match first {
                Some((code, link)) => {
                    let (new, old) = line(code, link);
                    (Some(new), Some(old))
                }
                None => (None, None),
            };
            let new = NewOrder { lines: new_lines, first: new_first, percent };
            let old = OldOrder { lines: old_lines, first: old_first, percent };
            assert_eq!(errors(&new), errors(&old), "{percent}");
        }
    }

    #[derive(DeriveValidate)]
    struct NewShare {
        #[validate(range(exclusive_min = 0.0, exclusive_max = 1.0))]
        ratio: f64,
        #[validate(range(exclusive_min = 0, max = 10, code = "share.count", message = "1 to 10"))]
        count: Option<i64>,
    }

    #[derive(validator::Validate)]
    struct OldShare {
        #[validate(range(exclusive_min = 0.0, exclusive_max = 1.0))]
        ratio: f64,
        #[validate(range(exclusive_min = 0, max = 10, code = "share.count", message = "1 to 10"))]
        count: Option<i64>,
    }

    // `range` có cận mở (`exclusive_*`), mã và câu riêng: lỗi giống nhau.
    #[test]
    fn derive_range_can_mo_nhu_validator() {
        // Phép so có nghĩa: ca sai cho lỗi thật ở cả hai trường.
        let failing = NewShare { ratio: 1.0, count: Some(11) }.check().err();
        let fields: Vec<_> =
            failing.iter().flat_map(|error| error.fields()).map(|f| f.field.clone()).collect();
        assert_eq!(fields, ["count", "ratio"], "{failing:?}");
        for (ratio, count) in [(0.0, Some(0)), (0.5, Some(10)), (1.0, Some(11)), (0.25, None)] {
            assert_eq!(
                errors(&NewShare { ratio, count }),
                errors(&OldShare { ratio, count }),
                "{ratio} {count:?}"
            );
        }
    }

    // `#[derive(Params)]` với `#[serde(default)]` trên trường: bản sao vẫn biên dịch, và tham số có giá trị mặc định
    // không bắt buộc — như `IntoParams` viết thẳng.
    #[derive(serde::Deserialize, autoxide_macros::Params)]
    #[into_params(parameter_in = Query)]
    #[allow(dead_code)]
    struct CountryQuery {
        /// ISO country code.
        #[serde(default = "vn")]
        #[param(example = "VN")]
        country: String,
        #[serde(rename = "asOf")]
        as_of: String,
    }

    fn vn() -> String {
        "VN".to_owned()
    }

    #[test]
    fn params_nhan_thuoc_tinh_serde() {
        use utoipa::IntoParams as _;
        let params = CountryQuery::into_params(|| None);
        let country = params.iter().find(|param| param.name == "country").unwrap();
        assert!(country.required != utoipa::openapi::Required::True);
        let as_of = params.iter().find(|param| param.name == "asOf").unwrap();
        assert!(as_of.required == utoipa::openapi::Required::True);
    }
}
