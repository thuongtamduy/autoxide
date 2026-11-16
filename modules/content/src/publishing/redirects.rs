//! Chuyển hướng: tự động (đường dẫn công khai đổi → 301 tới mục; gỡ hẳn → 410) và làm tay
//! (301 / 302 / 307 / 308 / 410, tới một mục hay một địa chỉ). Cần `content.manage`.
//! Đích là mục thì trỏ theo ID, nên đổi đường dẫn của mục đích không làm chuỗi chuyển hướng.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
};
use serde_json::json;
use uuid::Uuid;

use crate::entity::{entry, published, redirect};
use crate::entries::Status;
use crate::events::{self, Change};
use crate::permissions::CONTENT_MANAGE;
use crate::publishing::paths;
use crate::service::{ContentService, invalid, now};

pub(crate) type Redirect = redirect::Model;

/// Mã trạng thái được nhận.
const STATUSES: [i32; 5] = [301, 302, 307, 308, 410];
/// Đi theo chuỗi chuyển hướng tối đa chừng này bước khi tìm vòng lặp.
const MAX_HOPS: usize = 10;

/// Một chuyển hướng làm tay (tạo hay thay toàn bộ).
#[derive(Debug, Clone)]
pub(crate) struct RedirectInput {
    pub locale: Option<String>,
    pub from_path: String,
    pub to_entry_id: Option<Uuid>,
    pub to_url: Option<String>,
    pub status_code: i32,
}

/// Đường dẫn trả về cho website: mục công khai, chuyển hướng, hay đã gỡ.
#[derive(Debug, Clone)]
pub(crate) enum Resolved {
    Entry(Box<published::Model>),
    Redirect { status: i32, location: String },
    Gone,
}

impl ContentService {
    pub(crate) async fn list_redirects(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        locale: Option<&str>,
        source: Option<&str>,
        page: &PageRequest,
    ) -> Result<Page<Redirect>, Error> {
        principal.require(&CONTENT_MANAGE)?;
        let mut select = redirect::Entity::find().scoped(tenant);
        if let Some(locale) = locale {
            select = select.filter(redirect::Column::Locale.eq(locale));
        }
        if let Some(source) = source {
            select = select.filter(redirect::Column::Source.eq(source));
        }
        fetch_page(select, redirect::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    pub(crate) async fn get_redirect(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Redirect, Error> {
        principal.require(&CONTENT_MANAGE)?;
        redirect::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| redirect_not_found(id))
    }

    /// Tạo chuyển hướng làm tay. Chặn vòng lặp và đường dẫn đang có nội dung công khai.
    pub(crate) async fn create_redirect(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: RedirectInput,
    ) -> Result<Redirect, Error> {
        principal.require(&CONTENT_MANAGE)?;
        let now = now();
        let row =
            self.check_redirect(tenant, Uuid::now_v7(), input, principal.user_id(), now).await?;
        let transaction = tenant.database().begin_write().await?;
        // Khoá chung của ngôn ngữ: hai chuyển hướng trỏ vào nhau ghi cùng lúc không thể cùng
        // qua bước kiểm vòng lặp (Postgres); cùng khoá với mọi thay đổi đường dẫn.
        paths::lock_tree(&transaction, tenant, &row.locale).await?;
        self.check_loop(tenant, &transaction, &row).await?;
        let saved = row.into_active_model().insert(&transaction).await.map_err(|error| {
            if crate::entries::is_unique_violation(&error) {
                Error::conflict(
                    "content.redirect_exists",
                    "A redirect from this address already exists; change it instead.",
                )
            } else {
                error.into()
            }
        })?;
        let change =
            Change { before: None, after: Some(redirect_summary(&saved)), published: None };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::subject("redirect", saved.id),
                &events::RedirectChanged(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(saved)
    }

    /// Thay chuyển hướng `id` (đích, mã); nó thành chuyển hướng làm tay.
    pub(crate) async fn update_redirect(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        input: RedirectInput,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Redirect, Error> {
        principal.require(&CONTENT_MANAGE)?;
        let before = self.get_redirect(tenant, principal, id).await?;
        let now = now();
        let mut row = self.check_redirect(tenant, id, input, principal.user_id(), now).await?;
        row.created_at = before.created_at;
        let transaction = tenant.database().begin_write().await?;
        // Khoá chung của ngôn ngữ: hai chuyển hướng trỏ vào nhau ghi cùng lúc không thể cùng
        // qua bước kiểm vòng lặp (Postgres); cùng khoá với mọi thay đổi đường dẫn.
        paths::lock_tree(&transaction, tenant, &row.locale).await?;
        self.check_loop(tenant, &transaction, &row).await?;
        let mut active = row.into_active_model().reset_all();
        active.id = sea_orm::ActiveValue::Unchanged(id);
        let update = redirect::Entity::update(active)
            .validate()?
            .scoped(tenant)
            .filter(redirect::Column::UpdatedAt.eq(expected.unwrap_or(before.updated_at)));
        let after = match update.exec(&transaction).await {
            Ok(after) => after,
            Err(sea_orm::DbErr::RecordNotUpdated) => {
                return Err(autoxide_kernel::version::changed());
            }
            Err(error) if crate::entries::is_unique_violation(&error) => {
                return Err(Error::conflict(
                    "content.redirect_exists",
                    "A redirect from this address already exists.",
                ));
            }
            Err(error) => return Err(error.into()),
        };
        let change = Change {
            before: Some(redirect_summary(&before)),
            after: Some(redirect_summary(&after)),
            published: None,
        };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::subject("redirect", id),
                &events::RedirectChanged(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(after)
    }

    pub(crate) async fn delete_redirect(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        principal.require(&CONTENT_MANAGE)?;
        let transaction = tenant.database().begin_write().await?;
        let before = redirect::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| redirect_not_found(id))?;
        redirect::Entity::delete_by_id(id).scoped(tenant).exec(&transaction).await?;
        let change =
            Change { before: Some(redirect_summary(&before)), after: None, published: None };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::subject("redirect", id),
                &events::RedirectChanged(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Kiểm đầu vào, mọi lỗi một lần; trả dòng sẵn để ghi.
    async fn check_redirect(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        input: RedirectInput,
        by: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Redirect, Error> {
        let mut problems = Vec::new();
        let locale = self.check_locale(input.locale.as_deref(), "locale", &mut problems);
        let from = paths::normalize(&input.from_path);
        if from.is_none() {
            problems.push(FieldError::new(
                "from_path",
                "invalid",
                "must be a path starting with `/`, without `?`, `#`, `.` or `..` segments",
            ));
        }
        if !STATUSES.contains(&input.status_code) {
            problems.push(FieldError::new(
                "status_code",
                "invalid",
                "must be 301, 302, 307, 308 or 410",
            ));
        }
        let to_url = input.to_url.map(|url| url.trim().to_owned()).filter(|url| !url.is_empty());
        let targets = usize::from(input.to_entry_id.is_some()) + usize::from(to_url.is_some());
        if input.status_code == 410 && targets > 0 {
            problems.push(FieldError::new("status_code", "invalid", "410 (gone) takes no target"));
        } else if input.status_code != 410 && targets != 1 {
            problems.push(FieldError::new(
                "to_entry_id",
                "required",
                "give exactly one of `to_entry_id` and `to_url`",
            ));
        }
        if let Some(url) = &to_url {
            let lower = url.to_ascii_lowercase();
            let valid = (lower.starts_with("https://")
                || lower.starts_with("http://")
                || paths::is_site_link(url))
                && url.len() <= 2048
                && !url.chars().any(|c| c.is_whitespace() || c.is_control());
            if !valid {
                problems.push(FieldError::new(
                    "to_url",
                    "invalid",
                    "must be an `https://` address or a path starting with a single `/`",
                ));
            }
        }
        if let Some(target) = input.to_entry_id {
            match entry::Entity::find_by_id(target)
                .scoped(tenant)
                .one(tenant.database().read())
                .await?
            {
                None => problems.push(FieldError::new(
                    "to_entry_id",
                    "not_found",
                    "no entry has this id",
                )),
                // Website ghép tiền tố ngôn ngữ của chuyển hướng vào đường dẫn đích: đích khác
                // ngôn ngữ sẽ dẫn người đọc tới một trang không có.
                Some(found) if found.locale != locale => problems.push(FieldError::new(
                    "to_entry_id",
                    "other_locale",
                    "the entry is in another language; point to its translation in this language",
                )),
                Some(_) => {}
            }
        }
        let Some(from) = from.filter(|_| problems.is_empty()) else {
            return Err(invalid(problems));
        };
        if paths::path_used(tenant.database().read(), tenant, &locale, &from, Uuid::nil()).await? {
            return Err(Error::conflict(
                "content.path_taken",
                "A published entry is at `{path}`; a redirect there would never be used.",
            )
            .with_param("path", &from));
        }
        Ok(Redirect {
            id,
            tenant_id: tenant.id().as_uuid(),
            locale,
            from_path: from,
            to_entry_id: input.to_entry_id,
            to_url,
            status_code: input.status_code,
            source: "manual".to_owned(),
            created_by: Some(by),
            created_at: now,
            updated_at: now,
        })
    }

    /// Đi theo chuyển hướng từ đích của `row`; quay về đường dẫn gốc thì là vòng lặp.
    async fn check_loop<C: ConnectionTrait>(
        &self,
        tenant: &TenantContext,
        connection: &C,
        row: &Redirect,
    ) -> Result<(), Error> {
        let mut next = self.target_path(tenant, connection, row).await?;
        for _ in 0..MAX_HOPS {
            let Some(path) = next else { return Ok(()) };
            if path == row.from_path {
                return Err(Error::conflict(
                    "content.redirect_loop",
                    "This redirect would lead back to its own address.",
                ));
            }
            let found = redirect::Entity::find()
                .scoped(tenant)
                .filter(redirect::Column::Locale.eq(row.locale.as_str()))
                .filter(redirect::Column::FromPath.eq(path.as_str()))
                .one(connection)
                .await?;
            next = match found {
                Some(found) => self.target_path(tenant, connection, &found).await?,
                None => None,
            };
        }
        Err(Error::conflict(
            "content.redirect_loop",
            "This redirect starts a chain of more than 10 redirects.",
        ))
    }

    /// Đường dẫn trong site mà chuyển hướng `row` trỏ tới, nếu có.
    async fn target_path<C: ConnectionTrait>(
        &self,
        tenant: &TenantContext,
        connection: &C,
        row: &Redirect,
    ) -> Result<Option<String>, Error> {
        if let Some(id) = row.to_entry_id {
            return Ok(published::Entity::find()
                .scoped(tenant)
                .filter(published::Column::EntryId.eq(id))
                .one(connection)
                .await?
                .map(|row| row.path));
        }
        Ok(row.to_url.as_deref().and_then(paths::normalize))
    }

    /// Tra đường dẫn `path` của ngôn ngữ `locale`: bản công khai → chuyển hướng → không có.
    /// Không cần đăng nhập; không bao giờ trả bản nháp.
    pub(crate) async fn resolve(
        &self,
        tenant: &TenantContext,
        locale: &str,
        path: &str,
    ) -> Result<Resolved, Error> {
        let not_found = || {
            Error::not_found("content.not_found", "Nothing is published at `{path}`.")
                .with_param("path", path)
        };
        let mut problems = Vec::new();
        let locale = self.check_locale(Some(locale), "locale", &mut problems);
        let normalized = paths::normalize(path);
        if normalized.is_none() {
            problems.push(FieldError::new("path", "invalid", "must be a path starting with `/`"));
        }
        let Some(path) = normalized.filter(|_| problems.is_empty()) else {
            return Err(Error::validation(problems));
        };
        let read = tenant.database().read();
        if let Some(found) = published::Entity::find()
            .scoped(tenant)
            .filter(published::Column::Locale.eq(locale.as_str()))
            .filter(published::Column::Path.eq(path.as_str()))
            .one(read)
            .await?
        {
            return Ok(Resolved::Entry(Box::new(found)));
        }
        let Some(found) = redirect::Entity::find()
            .scoped(tenant)
            .filter(redirect::Column::Locale.eq(locale.as_str()))
            .filter(redirect::Column::FromPath.eq(path.as_str()))
            .one(read)
            .await?
        else {
            return Err(not_found());
        };
        if found.status_code == 410 {
            return Ok(Resolved::Gone);
        }
        let location = match (found.to_entry_id, found.to_url) {
            (Some(id), _) => {
                match published::Entity::find()
                    .scoped(tenant)
                    .filter(published::Column::EntryId.eq(id))
                    .one(read)
                    .await?
                {
                    Some(row) => Some(row.path),
                    // Đích đã lưu trữ: gỡ hẳn, đường dẫn cũ cũng "đã gỡ" như đường dẫn của nó.
                    None if entry::Entity::find_by_id(id)
                        .scoped(tenant)
                        .one(read)
                        .await?
                        .is_some_and(|entry| entry.status == Status::Archived.as_str()) =>
                    {
                        return Ok(Resolved::Gone);
                    }
                    None => None,
                }
            }
            (None, url) => url,
        };
        match location {
            Some(location) => Ok(Resolved::Redirect { status: found.status_code, location }),
            // Đích không còn công khai: coi như không có gì ở đây.
            None => Err(not_found()),
        }
    }
}

fn redirect_summary(row: &Redirect) -> serde_json::Value {
    json!({
        "locale": row.locale,
        "from_path": row.from_path,
        "to_entry_id": row.to_entry_id,
        "to_url": row.to_url,
        "status_code": row.status_code,
        "source": row.source,
    })
}

pub(crate) fn redirect_not_found(id: Uuid) -> Error {
    Error::not_found("content.redirect_not_found", "No redirect has the id `{id}`.")
        .with_param("id", id)
}
