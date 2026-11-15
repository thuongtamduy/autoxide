//! Mã tham chiếu do client đặt (`reference`) — chống trùng lệnh ở tầng nghiệp vụ (D-184).
//!
//! Idempotency key của HTTP (`Idempotency-Key`) chỉ giữ 24 giờ và chỉ chặn gửi lại cùng một
//! request. Lệnh có hậu quả lâu dài (chuyển tiền, tạo giao dịch) cần mạnh hơn: client đặt một
//! `reference` (vd mã hoá đơn, mã lệnh trong hệ thống của họ), module lưu nó **vĩnh viễn cùng
//! bản ghi**, duy nhất theo người gọi:
//!
//! - gửi lại cùng `reference`, cùng nội dung → trả lại bản ghi đã tạo (không tạo thêm);
//! - cùng `reference`, nội dung khác → lỗi `reference_reused` (422);
//! - hai request cùng `reference` chạy đồng thời → đúng một bản ghi được tạo.
//!
//! Cách lưu chuẩn trong bảng của module: cột `reference` (chuỗi, tối đa [`MAX_LENGTH`]) và
//! `reference_fingerprint` ([`fingerprint`] của nội dung lệnh), cùng một **unique index** trên
//! (người sở hữu, `reference`). Service gọi [`submit_once`] và [`replayed`].

use std::fmt;
use std::future::Future;

use sea_orm::{DbErr, SqlErr};
use serde::Serialize;
use serde_json::Value;

use crate::database::Database;
use crate::error::{Error, ErrorKind, FieldError};

/// Độ dài tối đa của một `reference`, theo byte.
pub const MAX_LENGTH: usize = 100;

/// Một mã tham chiếu đã kiểm: 1–[`MAX_LENGTH`] ký tự ASCII nhìn thấy được (không khoảng trắng).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Reference(String);

impl Reference {
    /// Kiểm `raw` của trường `field`; sai thì thêm lỗi vào `problems` và trả `None` (service gom
    /// mọi lỗi rồi báo một lần, D-083).
    pub fn checked(raw: &str, field: &str, problems: &mut Vec<FieldError>) -> Option<Reference> {
        let valid =
            (1..=MAX_LENGTH).contains(&raw.len()) && raw.bytes().all(|b| b.is_ascii_graphic());
        if valid {
            Some(Reference(raw.to_owned()))
        } else {
            problems.push(FieldError::new(
                field,
                "reference",
                format!(
                    "must be 1 to {MAX_LENGTH} visible ASCII characters without spaces, e.g. \
                     an invoice number or a UUID"
                ),
            ));
            None
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Reference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Reference({:?})", self.0)
    }
}

impl fmt::Display for Reference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Dấu vân tay của nội dung lệnh: SHA-256 của JSON chuẩn hoá (khoá sắp theo thứ tự, không khoảng
/// trắng) — cùng nội dung luôn cho cùng dấu vân tay, dù client gửi khoá theo thứ tự nào. Chỉ
/// đưa vào những trường quyết định kết quả của lệnh (không đưa `reference` hay thời gian gửi).
pub fn fingerprint(content: &impl Serialize) -> Result<String, Error> {
    let value = serde_json::to_value(content).map_err(Error::internal)?;
    let mut text = String::new();
    canonical(&value, &mut text);
    Ok(crate::secret::sha256(&text))
}

/// Viết `value` thành JSON với khoá của object sắp tăng dần. Không dựa vào thứ tự của
/// `serde_json::Map`: một thư viện khác bật feature `preserve_order` là thứ tự đổi theo.
fn canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(fields) => {
            let mut keys: Vec<&String> = fields.keys().collect();
            keys.sort();
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                if let Some(field) = fields.get(key) {
                    canonical(field, out);
                }
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                canonical(item, out);
            }
            out.push(']');
        }
        scalar => out.push_str(&scalar.to_string()),
    }
}

/// Kết quả của [`submit_once`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Submitted<T> {
    /// Lần đầu: bản ghi vừa tạo (REST trả 201).
    Created(T),
    /// Đã có từ lần gửi trước với cùng `reference` và cùng nội dung (REST trả 200).
    Replayed(T),
}

impl<T> Submitted<T> {
    /// Bản ghi, dù mới tạo hay có sẵn.
    pub fn into_inner(self) -> T {
        match self {
            Submitted::Created(value) | Submitted::Replayed(value) => value,
        }
    }
}

/// So bản ghi đã có với lệnh đang gửi. `stored`: bản ghi của người gọi mang `reference` này cùng
/// dấu vân tay đã lưu, nếu có (tìm trong transaction ghi của lệnh). Trả:
///
/// - `Ok(None)`: chưa có — tạo bản ghi mới (lưu kèm `reference` và `fingerprint`);
/// - `Ok(Some(bản ghi))`: đã có, cùng nội dung — trả lại nó, không tạo thêm;
/// - `Err(reference_reused)`: đã có, nội dung khác.
pub fn replayed<T>(
    stored: Option<(T, String)>,
    reference: &Reference,
    fingerprint: &str,
) -> Result<Option<T>, Error> {
    match stored {
        None => Ok(None),
        Some((existing, stored)) if stored == fingerprint => Ok(Some(existing)),
        Some(_) => Err(reused(reference)),
    }
}

/// Chạy lệnh mang `reference` đúng một lần. `work` (như của
/// [`Database::write_with_retry`]) mở transaction ghi, tìm bản ghi đã có rồi gọi [`replayed`],
/// tạo bản ghi nếu chưa có, commit, và trả [`Submitted`].
///
/// Hai request cùng `reference` chạy đồng thời: SQLite xếp chúng nối đuôi (một bên ghi); trên
/// Postgres cả hai cùng thấy "chưa có", unique index chặn bên thứ hai — hàm này chạy lại `work`
/// của bên đó một lần, và lần này nó thấy bản ghi của bên kia. Xung đột tạm thời cũng được chạy
/// lại như `write_with_retry`, nên `work` phải chịu được chạy lại.
///
/// ```ignore
/// let (database, reference, fingerprint) = (&self.database, &reference, &fingerprint);
/// let submitted = reference::submit_once(database, move || async move {
///     let transaction = database.begin_write().await?;
///     let stored = find_by_reference(&transaction, owner, reference).await?;
///     if let Some(existing) = reference::replayed(stored, reference, fingerprint)? {
///         return Ok(Submitted::Replayed(existing));
///     }
///     let created = insert(&transaction, owner, reference, fingerprint).await?;
///     transaction.commit().await?;
///     Ok(Submitted::Created(created))
/// })
/// .await?;
/// ```
pub async fn submit_once<T, F, Work>(
    database: &Database,
    mut work: F,
) -> Result<Submitted<T>, Error>
where
    F: FnMut() -> Work,
    Work: Future<Output = Result<Submitted<T>, Error>>,
{
    let mut raced = false;
    loop {
        match database.write_with_retry(&mut work).await {
            // Bên kia vừa tạo cùng `reference` (Postgres): chạy lại một lần để thấy bản ghi đó.
            Err(error) if !raced && is_unique_violation(&error) => raced = true,
            outcome => return outcome,
        }
    }
}

fn reused(reference: &Reference) -> Error {
    Error::new(
        ErrorKind::Validation,
        "reference_reused",
        "The reference `{reference}` was already used for a different request. Send a new \
         reference for a new operation, or resend exactly the same content to get the earlier \
         result.",
    )
    .with_param("reference", reference)
    .with_field(FieldError::new(
        "reference",
        "reused",
        "was already used for a different request",
    ))
}

/// Lỗi vi phạm unique (của database) có nằm trong chuỗi nguyên nhân của `error` không.
fn is_unique_violation(error: &Error) -> bool {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(error) = current {
        if let Some(database) = error.downcast_ref::<DbErr>() {
            return matches!(database.sql_err(), Some(SqlErr::UniqueConstraintViolation(_)));
        }
        current = error.source();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn kiem_dang_ma_tham_chieu() {
        let mut problems = Vec::new();
        assert_eq!(
            Reference::checked("INV-2026/0042", "reference", &mut problems).unwrap().as_str(),
            "INV-2026/0042"
        );
        for bad in ["", "has space", "tiếng-việt", &"x".repeat(MAX_LENGTH + 1)] {
            assert!(Reference::checked(bad, "reference", &mut problems).is_none(), "{bad}");
        }
        assert_eq!(problems.len(), 4);
        assert_eq!(problems[0].code, "reference");
    }

    #[test]
    fn dau_van_tay_khong_phu_thuoc_thu_tu_khoa() {
        let a = fingerprint(&json!({"amount": "5", "to": {"id": 1, "kind": "x"}})).unwrap();
        let b = fingerprint(&json!({"to": {"kind": "x", "id": 1}, "amount": "5"})).unwrap();
        assert_eq!(a, b);
        assert_ne!(a, fingerprint(&json!({"amount": "6", "to": {"id": 1, "kind": "x"}})).unwrap());
        assert_ne!(fingerprint(&json!([1, 2])).unwrap(), fingerprint(&json!([2, 1])).unwrap());
        let mut text = String::new();
        canonical(&json!({"b": [1, {"d": null, "c": "é\""}], "a": true}), &mut text);
        assert_eq!(text, r#"{"a":true,"b":[1,{"c":"é\"","d":null}]}"#);
    }

    #[test]
    fn so_ban_ghi_da_co() {
        let reference = Reference("INV-1".to_owned());
        assert_eq!(replayed::<u8>(None, &reference, "f").unwrap(), None);
        assert_eq!(replayed(Some((7, "f".to_owned())), &reference, "f").unwrap(), Some(7));
        let error = replayed(Some((7, "g".to_owned())), &reference, "f").unwrap_err();
        assert_eq!(error.code(), "reference_reused");
    }

    #[test]
    fn loi_khi_dung_lai_ma() {
        let reference = Reference("INV-1".to_owned());
        let error = reused(&reference);
        assert_eq!((error.code(), error.kind().status()), ("reference_reused", 422));
        assert!(error.message().contains("`INV-1`"));
        assert_eq!(error.fields()[0].field, "reference");
    }
}
