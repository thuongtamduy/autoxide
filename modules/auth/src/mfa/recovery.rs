//! Recovery code: mã dùng một lần để đăng nhập (hay tắt MFA) khi mất thiết bị TOTP.
//!
//! Mỗi lần bật MFA sinh [`COUNT`] mã, mỗi mã 80 bit ngẫu nhiên viết thành 16 ký tự
//! base32 chữ thường chia nhóm (`abcd-efgh-ijkl-mnop`). Database chỉ lưu SHA-256 của
//! mã: 80 bit ngẫu nhiên thì băm nhanh vẫn không đoán ngược được.

use autoxide_kernel::error::Error;
use chrono::{DateTime, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, Set,
};
use uuid::Uuid;

use super::entity::recovery_code::{self, Column, Entity};
use crate::token;

/// Số mã mỗi bộ.
pub(crate) const COUNT: usize = 10;
/// Số byte ngẫu nhiên của một mã: 80 bit = đúng 16 ký tự base32.
const CODE_BYTES: usize = 10;
const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// Thay bộ mã của `user_id` bằng [`COUNT`] mã mới; trả về các mã (chỉ lần này).
pub(crate) async fn replace<C: ConnectionTrait>(
    connection: &C,
    user_id: Uuid,
    now: DateTime<Utc>,
) -> Result<Vec<String>, Error> {
    remove(connection, user_id).await?;
    let mut codes = Vec::with_capacity(COUNT);
    for _ in 0..COUNT {
        let code = generate()?;
        recovery_code::ActiveModel {
            id: Set(Uuid::now_v7()),
            user_id: Set(user_id),
            code_hash: Set(hash(&code)),
            used_at: Set(None),
            created_at: Set(now),
        }
        .insert(connection)
        .await?;
        codes.push(code);
    }
    Ok(codes)
}

/// Dùng mã `code` của `user_id`: `true` nếu mã đúng và chưa dùng. Có điều kiện trong
/// một câu `UPDATE`: hai request dùng cùng mã thì chỉ một bên được nhận.
pub(crate) async fn redeem<C: ConnectionTrait>(
    connection: &C,
    user_id: Uuid,
    code: &str,
    now: DateTime<Utc>,
) -> Result<bool, Error> {
    let Some(normalized) = normalize(code) else {
        return Ok(false);
    };
    let used = Entity::update_many()
        .col_expr(Column::UsedAt, Expr::value(Some(now)))
        .filter(Column::UserId.eq(user_id))
        .filter(Column::CodeHash.eq(hash(&normalized)))
        .filter(Column::UsedAt.is_null())
        .exec(connection)
        .await?;
    Ok(used.rows_affected == 1)
}

/// Số mã chưa dùng của `user_id`.
pub(crate) async fn remaining<C: ConnectionTrait>(
    connection: &C,
    user_id: Uuid,
) -> Result<u64, Error> {
    Ok(Entity::find()
        .filter(Column::UserId.eq(user_id))
        .filter(Column::UsedAt.is_null())
        .count(connection)
        .await?)
}

pub(crate) async fn remove<C: ConnectionTrait>(connection: &C, user_id: Uuid) -> Result<(), Error> {
    Entity::delete_many().filter(Column::UserId.eq(user_id)).exec(connection).await?;
    Ok(())
}

/// Trông giống một recovery code (để phân biệt với mã TOTP 6 chữ số).
pub(crate) fn looks_like_code(code: &str) -> bool {
    normalize(code).is_some()
}

fn generate() -> Result<String, Error> {
    let mut bytes = [0u8; CODE_BYTES];
    getrandom::fill(&mut bytes)
        .map_err(|error| Error::internal(format!("random number generator: {error}")))?;
    let raw = base32(&bytes);
    let groups: Vec<&str> =
        raw.as_bytes().chunks(4).filter_map(|c| std::str::from_utf8(c).ok()).collect();
    Ok(groups.join("-"))
}

/// Mã người dùng gõ về dạng chuẩn: bỏ khoảng trắng và `-`, chữ thường. `None` nếu
/// không phải 16 ký tự base32.
fn normalize(code: &str) -> Option<String> {
    let normalized: String = code
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_lowercase())
        .collect();
    (normalized.len() == 16 && normalized.bytes().all(|b| ALPHABET.contains(&b)))
        .then_some(normalized)
}

fn hash(code: &str) -> String {
    token::sha256(&code.replace('-', ""))
}

/// Base32 (RFC 4648, chữ thường, không đệm) cho đúng 10 byte → 16 ký tự.
fn base32(bytes: &[u8; CODE_BYTES]) -> String {
    let mut output = String::with_capacity(16);
    let mut buffer: u64 = 0;
    let mut bits = 0;
    for byte in bytes {
        buffer = (buffer << 8) | u64::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let index = usize::try_from((buffer >> bits) & 0x1f).unwrap_or(0);
            output.push(char::from(ALPHABET[index]));
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ma_sinh_ra_dung_dang_va_khac_nhau() {
        let first = generate().unwrap();
        let second = generate().unwrap();

        assert_eq!(first.len(), 19);
        assert_eq!(first.matches('-').count(), 3);
        assert!(looks_like_code(&first));
        assert_ne!(first, second);
    }

    #[test]
    fn base32_dung_chuan() {
        // Vector thử của RFC 4648: "fooba" → "MZXW6YTB"; ghép hai lần cho đủ 10 byte.
        assert_eq!(base32(b"foobafooba"), "mzxw6ytbmzxw6ytb");
    }

    #[test]
    fn chuan_hoa_ma_go_tay() {
        assert_eq!(normalize(" ABCD-efgh ijkl-mnop ").as_deref(), Some("abcdefghijklmnop"));
        assert_eq!(normalize("123456"), None);
        assert_eq!(normalize("abcd-efgh-ijkl-mno1"), None);
    }
}
