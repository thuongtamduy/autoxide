//! `alt` theo ngôn ngữ (D-202): bảng `media_alt_translations`, một dòng cho mỗi (item, ngôn ngữ).
//! `media_item.alt` giữ nguyên nghĩa là bản của ngôn ngữ mặc định; đọc theo một ngôn ngữ thì lấy
//! bản dịch nếu có, không thì lùi về bản mặc định.
//!
//! Đặt / xoá bản dịch là một thay đổi của item: cùng `If-Match`, cùng event `media.item_updated`
//! (ảnh chụp trước / sau kèm `alts`), `updated_at` của item đổi nên `ETag` đổi theo.

use std::collections::{BTreeMap, HashMap};

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::TenantContext;
use chrono::{DateTime, Utc};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, Set};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::alt_translation;
use crate::entity::item::{Column, Entity};
use crate::events::{self, Change};
use crate::permissions::MEDIA_WRITE;
use crate::service::{ALT_MAX_LENGTH, Item, MediaService, in_trash, item_not_found, now, snapshot};

/// Bản dịch `alt` của các item `ids`: item → (ngôn ngữ → alt). Một câu query.
pub(crate) async fn alts_of<C: ConnectionTrait>(
    connection: &C,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, BTreeMap<String, String>>, Error> {
    let mut found: HashMap<Uuid, BTreeMap<String, String>> = HashMap::new();
    if ids.is_empty() {
        return Ok(found);
    }
    let rows = alt_translation::Entity::find()
        .filter(alt_translation::Column::MediaId.is_in(ids.to_vec()))
        .all(connection)
        .await?;
    for row in rows {
        found.entry(row.media_id).or_default().insert(row.locale, row.alt);
    }
    Ok(found)
}

/// `alt` bằng ngôn ngữ `locale` của các item `ids` (chỉ item có bản dịch). Một câu query.
pub(crate) async fn alts_in<C: ConnectionTrait>(
    connection: &C,
    ids: &[Uuid],
    locale: &str,
) -> Result<HashMap<Uuid, String>, Error> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(alt_translation::Entity::find()
        .filter(alt_translation::Column::MediaId.is_in(ids.to_vec()))
        .filter(alt_translation::Column::Locale.eq(locale))
        .all(connection)
        .await?
        .into_iter()
        .map(|row| (row.media_id, row.alt))
        .collect())
}

/// Ngôn ngữ không nằm trong `AUTOXIDE_MEDIA_ALT_LOCALES`.
fn unsupported_locale(locales: &[String]) -> Error {
    Error::new(
        ErrorKind::Validation,
        "media.unsupported_locale",
        "Text alternatives can be translated only into the languages this server lists in \
         AUTOXIDE_MEDIA_ALT_LOCALES: {locales}.",
    )
    .with_param("locales", locales.join(", "))
}

fn with_alts(mut snapshot: Value, alts: &BTreeMap<String, String>) -> Value {
    if let Some(object) = snapshot.as_object_mut() {
        object.insert("alts".to_owned(), json!(alts));
    }
    snapshot
}

impl MediaService {
    /// Ngôn ngữ `raw` đúng cách viết đã khai (không phân biệt hoa thường), hay lỗi.
    pub(crate) fn alt_locale(&self, raw: &str) -> Result<String, Error> {
        self.config
            .alt_locales
            .iter()
            .find(|known| known.eq_ignore_ascii_case(raw.trim()))
            .cloned()
            .ok_or_else(|| unsupported_locale(&self.config.alt_locales))
    }

    /// Đặt (`Some`) hay xoá (`None`) `alt` bằng ngôn ngữ `locale` của item `id`. Cần
    /// `media.write`; item trong thùng rác thì phải khôi phục trước. `expected`: phiên bản người
    /// gọi đã đọc (`If-Match`). Xoá bản dịch không có thì không đổi gì.
    pub(crate) async fn set_alt(
        &self,
        principal: &Principal,
        id: Uuid,
        locale: &str,
        alt: Option<&str>,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Item, Error> {
        principal.require(&MEDIA_WRITE)?;
        let locale = self.alt_locale(locale)?;
        let alt = alt.map(str::trim);
        if alt.is_some_and(|alt| alt.chars().count() > ALT_MAX_LENGTH) {
            return Err(Error::validation(vec![FieldError::new(
                "alt",
                "length",
                format!("must be at most {ALT_MAX_LENGTH} characters long"),
            )]));
        }

        let transaction = self.database.begin_write().await?;
        let before =
            Entity::find_by_id(id).one(&transaction).await?.ok_or_else(|| item_not_found(id))?;
        if before.trashed_at.is_some() {
            return Err(in_trash(id));
        }
        if expected.is_some_and(|expected| expected != before.updated_at) {
            return Err(autoxide_kernel::version::changed());
        }
        let alts_before = alts_of(&transaction, &[id]).await?.remove(&id).unwrap_or_default();
        let mut alts_after = alts_before.clone();
        match alt {
            Some(alt) => {
                alts_after.insert(locale.clone(), alt.to_owned());
            }
            None => {
                alts_after.remove(&locale);
            }
        }
        if alts_after == alts_before {
            transaction.rollback().await?;
            return Ok(before);
        }

        // Đổi `updated_at` có điều kiện phiên bản ngay trong câu `UPDATE` (D-084): hai lệnh cùng
        // lúc thì một bên nhận 412.
        let now = now();
        let mut row = before.clone().into_active_model();
        row.updated_at = Set(now);
        let update = Entity::update(row)
            .validate()?
            .filter(Column::TrashedAt.is_null())
            .filter(Column::UpdatedAt.eq(before.updated_at));
        let after = match update.exec(&transaction).await {
            Ok(after) => after,
            Err(sea_orm::DbErr::RecordNotUpdated) => {
                return Err(self.why_not_changed(id, &transaction).await);
            }
            Err(error) => return Err(error.into()),
        };
        match alt {
            Some(alt) => {
                let row = alt_translation::ActiveModel {
                    media_id: Set(id),
                    locale: Set(locale.clone()),
                    alt: Set(alt.to_owned()),
                    updated_at: Set(now),
                };
                alt_translation::Entity::insert(row)
                    .on_conflict(
                        OnConflict::columns([
                            alt_translation::Column::MediaId,
                            alt_translation::Column::Locale,
                        ])
                        .update_columns([
                            alt_translation::Column::Alt,
                            alt_translation::Column::UpdatedAt,
                        ])
                        .to_owned(),
                    )
                    .exec_without_returning(&transaction)
                    .await?;
            }
            None => {
                alt_translation::Entity::delete_by_id((id, locale.clone()))
                    .exec(&transaction)
                    .await?;
            }
        }
        let event = events::ItemUpdated(Change {
            before: Some(with_alts(snapshot(&before), &alts_before)),
            after: Some(with_alts(snapshot(&after), &alts_after)),
        });
        self.outbox
            .record(
                &transaction,
                &TenantContext::legacy_default(&self.database),
                Actor::from(principal),
                events::item_subject(id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(media_item_id = %id, locale = %locale, removed = alt.is_none(), "media alt translation changed");
        Ok(after)
    }
}

/// Xoá mọi bản dịch của item `id` (khi xoá hẳn item), trong transaction của lệnh xoá.
pub(crate) async fn delete_alts<C: ConnectionTrait>(connection: &C, id: Uuid) -> Result<(), Error> {
    alt_translation::Entity::delete_many()
        .filter(alt_translation::Column::MediaId.eq(id))
        .exec(connection)
        .await?;
    Ok(())
}
