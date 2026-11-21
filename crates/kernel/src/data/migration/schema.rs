//! Hàm dựng cột cho migration (`schema::uuid("owner_id")`…). Chép đúng định nghĩa của `sea-orm-migration` 2.0
//! (MIT OR Apache-2.0) mà module đã dùng, để câu `CREATE TABLE` sinh ra y hệt trước (D-20261121-own-migrations).

use sea_orm::sea_query::{ColumnDef, IntoIden};

/// Khoá chính UUID (không null).
pub fn pk_uuid<T: IntoIden>(name: T) -> ColumnDef {
    uuid(name).primary_key().take()
}

pub fn uuid<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).uuid().not_null().take()
}

pub fn uuid_null<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).uuid().null().take()
}

pub fn string<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).string().not_null().take()
}

pub fn string_null<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).string().null().take()
}

pub fn string_len<T: IntoIden>(col: T, length: u32) -> ColumnDef {
    ColumnDef::new(col).string_len(length).not_null().take()
}

pub fn string_len_null<T: IntoIden>(col: T, length: u32) -> ColumnDef {
    ColumnDef::new(col).string_len(length).null().take()
}

pub fn text<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).text().not_null().take()
}

pub fn text_null<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).text().null().take()
}

pub fn small_integer<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).small_integer().not_null().take()
}

pub fn small_integer_null<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).small_integer().null().take()
}

pub fn integer<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).integer().not_null().take()
}

pub fn integer_null<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).integer().null().take()
}

pub fn big_integer<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).big_integer().not_null().take()
}

pub fn big_integer_null<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).big_integer().null().take()
}

pub fn boolean<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).boolean().not_null().take()
}

pub fn boolean_null<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).boolean().null().take()
}

pub fn timestamp_with_time_zone<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).timestamp_with_time_zone().not_null().take()
}

pub fn timestamp_with_time_zone_null<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).timestamp_with_time_zone().null().take()
}

pub fn date<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).date().not_null().take()
}

pub fn date_null<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).date().null().take()
}

pub fn binary<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).binary().not_null().take()
}

pub fn binary_null<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).binary().null().take()
}

pub fn blob<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).blob().not_null().take()
}

pub fn blob_null<T: IntoIden>(col: T) -> ColumnDef {
    ColumnDef::new(col).blob().null().take()
}
