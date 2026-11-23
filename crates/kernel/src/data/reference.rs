//! Client-set reference code (`reference`): guards against duplicate commands at the business level (D-184).
//!
//! The HTTP idempotency key (`Idempotency-Key`) is kept for only 24 hours and only blocks resending the
//! same request. A command with lasting consequences (moving money, creating a transaction) needs something
//! stronger: the client sets a `reference` (e.g. an invoice number or an order ID in its own system), and
//! the module stores it **permanently with the record**, unique per caller:
//!
//! - resend with the same `reference` and the same content → returns the record already created (nothing new
//!   is created);
//! - same `reference`, different content → error `reference_reused` (422);
//! - two requests with the same `reference` running concurrently → exactly one record is created.
//!
//! Standard storage in the module's table: a `reference` column (string, at most [`MAX_LENGTH`]) and
//! `reference_fingerprint` (the [`fingerprint`] of the command content), plus one **unique index** on
//! (owner, `reference`). The service calls [`submit_once`] and [`replayed`].

use std::fmt;
use std::future::Future;

use sea_orm::{DbErr, SqlErr};
use serde::Serialize;
use serde_json::Value;

use crate::database::Database;
use crate::error::{Error, ErrorKind, FieldError};

/// Maximum length of a `reference`, in bytes.
pub const MAX_LENGTH: usize = 100;

/// A validated reference code: 1 to [`MAX_LENGTH`] visible ASCII characters (no whitespace).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Reference(String);

impl Reference {
    /// Validates `raw` of field `field`; if invalid, adds an error to `problems` and returns `None` (the
    /// service gathers every error and reports once, D-083).
    pub fn checked(raw: &str, field: &str, problems: &mut Vec<FieldError>) -> Option<Reference> {
        let valid =
            (1..=MAX_LENGTH).contains(&raw.len()) && raw.bytes().all(|b| b.is_ascii_graphic());
        if valid {
            Some(Reference(raw.to_owned()))
        } else {
            problems.push(
                FieldError::new(
                    field,
                    "reference",
                    "must be 1 to {max_length} visible ASCII characters without spaces, e.g. \
                     an invoice number or a UUID",
                )
                .with_param("max_length", MAX_LENGTH),
            );
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

/// Fingerprint of the command content: SHA-256 of canonical JSON (keys in sorted order, no whitespace).
/// The same content always gives the same fingerprint, whatever order the client sent the keys in. Include only
/// the fields that determine the command's result (not `reference` or the send time).
pub fn fingerprint(content: &impl Serialize) -> Result<String, Error> {
    let value = serde_json::to_value(content).map_err(Error::internal)?;
    let mut text = String::new();
    canonical(&value, &mut text);
    Ok(crate::secret::sha256(&text))
}

/// Writes `value` as JSON with object keys in ascending order. Does not rely on the order of
/// `serde_json::Map`: if another library enables the `preserve_order` feature, the order changes with it.
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

/// Result of [`submit_once`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Submitted<T> {
    /// First time: the record just created (REST returns 201).
    Created(T),
    /// Already existed from an earlier send with the same `reference` and the same content (REST returns 200).
    Replayed(T),
}

impl<T> Submitted<T> {
    /// The record, whether newly created or already present.
    pub fn into_inner(self) -> T {
        match self {
            Submitted::Created(value) | Submitted::Replayed(value) => value,
        }
    }
}

/// Compares an existing record with the command being sent. `stored`: the caller's record that carries this
/// `reference` together with the stored fingerprint, if any (looked up in the command's write transaction).
/// Returns:
///
/// - `Ok(None)`: none yet; create a new record (store it with `reference` and `fingerprint`);
/// - `Ok(Some(record))`: exists with the same content; return it, create nothing more;
/// - `Err(reference_reused)`: exists with different content.
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

/// Runs a command that carries a `reference` exactly once. `work` (like the one of
/// [`Database::write_with_retry`]) opens a write transaction, looks up the existing record, then calls
/// [`replayed`], creates the record if none exists, commits, and returns [`Submitted`].
///
/// Two requests with the same `reference` running concurrently: SQLite queues them one after another (single
/// writer); on Postgres both see "none yet" and the unique index blocks the second; this function reruns that
/// side's `work` once, and this time it sees the other's record. A transient conflict is also rerun as in
/// `write_with_retry`, so `work` must tolerate being rerun.
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

/// Whether a unique-violation error (from the database) appears in the cause chain of `error`.
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
