//! The service: ALL the logic lives here. REST handlers and MCP tools only take input, check the
//! permission and call one method, so both ways give the same result.
//!
//! Every method takes the `TenantContext` of the customer: the database comes from it
//! (`tenant.database()`), and every query is filtered with `.scoped(tenant)`. Reads go through
//! `read()`, writes through one short `begin_write()` transaction that also records the event —
//! the change and its audit line exist together or not at all.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Delivered, EventHandler, Outbox};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::privacy::deleted_user;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set, SqlErr,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::Bookmarks;
use crate::config::BookmarksConfig;
use crate::entity::{self, Column, Entity};
use crate::events::{self, Change};

pub(crate) type Bookmark = entity::Model;

/// Input for a new bookmark, not yet checked.
#[derive(Debug, Clone)]
pub(crate) struct NewBookmark {
    pub url: String,
    pub title: String,
}

const TITLE_MAX_LENGTH: usize = 200;
const URL_MAX_LENGTH: usize = 2000;

// The service keeps no database: the database belongs to the tenant of each call.
pub(crate) struct BookmarkService {
    outbox: Outbox,
    config: BookmarksConfig,
}

impl BookmarkService {
    pub(crate) fn new(outbox: Outbox, config: BookmarksConfig) -> Self {
        BookmarkService { outbox, config }
    }

    pub(crate) fn config(&self) -> &BookmarksConfig {
        &self.config
    }

    /// Creates a bookmark of the caller. The same link twice: `bookmarks.url_taken`.
    pub(crate) async fn create(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewBookmark,
    ) -> Result<Bookmark, Error> {
        let (url, title) = check(&input)?;
        let transaction = tenant.database().begin_write().await?;
        let bookmark = entity::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            owner_id: Set(principal.user_id()),
            url: Set(url.clone()),
            title: Set(title),
            created_at: Set(Timestamp::now()),
        }
        .insert(&transaction)
        .await
        // The unique index decides, not a query before the insert: two requests arriving
        // together cannot both pass.
        .map_err(|error| match error.sql_err() {
            Some(SqlErr::UniqueConstraintViolation(_)) => {
                Error::conflict("bookmarks.url_taken", "You already keep the link `{url}`.")
                    .with_param("url", &url)
            }
            _ => Error::from(error),
        })?;
        let event =
            events::BookmarkCreated(Change { before: None, after: Some(snapshot(&bookmark)) });
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::subject(bookmark.id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(bookmark_id = %bookmark.id, "bookmark created");
        Ok(bookmark)
    }

    /// One page of the caller's bookmarks, newest first unless the request says otherwise.
    pub(crate) async fn list(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<Bookmark>, Error> {
        let select = Entity::find().filter(Column::OwnerId.eq(principal.user_id())).scoped(tenant);
        fetch_page(select, Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Bookmark `id` of the caller. Somebody else's bookmark answers as if it did not exist.
    pub(crate) async fn get(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Bookmark, Error> {
        Entity::find_by_id(id)
            .filter(Column::OwnerId.eq(principal.user_id()))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| not_found(id))
    }

    /// Deletes bookmark `id` of the caller.
    pub(crate) async fn delete(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let bookmark = Entity::find_by_id(id)
            .filter(Column::OwnerId.eq(principal.user_id()))
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| not_found(id))?;
        Entity::delete_many().filter(Column::Id.eq(id)).scoped(tenant).exec(&transaction).await?;
        let event =
            events::BookmarkDeleted(Change { before: Some(snapshot(&bookmark)), after: None });
        self.outbox
            .record(&transaction, tenant, Actor::from(principal), events::subject(id), &event)
            .await?;
        transaction.commit().await?;
        Ok(())
    }
}

/// An account was deleted: delete its bookmarks, in the tenant of the event. Events are delivered
/// again until the handler succeeds, so it must be safe to run twice — the second time there is
/// nothing left to delete.
#[async_trait::async_trait]
impl EventHandler for BookmarkService {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        forget_user(event).await
    }
}

/// The same deletion while the module is turned off (dormant mode, `Module::dormant`): it needs only the
/// tenant's database, so it works without the service.
pub(crate) struct DormantBookmarks;

#[async_trait::async_trait]
impl EventHandler for DormantBookmarks {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        forget_user(event).await
    }
}

async fn forget_user(event: &Delivered) -> Result<(), Error> {
    let Some(user_id) = deleted_user(event) else {
        return Ok(());
    };
    let tenant = &event.tenant;
    let deleted = Entity::delete_many()
        .filter(Column::OwnerId.eq(user_id))
        .scoped(tenant)
        .exec(tenant.database().write())
        .await?;
    tracing::info!(user_id = %user_id, bookmarks = deleted.rows_affected, "bookmarks of a deleted user removed");
    Ok(())
}

#[async_trait::async_trait]
impl Bookmarks for BookmarkService {
    async fn count(&self, tenant: &TenantContext, owner_id: Uuid) -> Result<u64, Error> {
        let found = Entity::find()
            .filter(Column::OwnerId.eq(owner_id))
            .scoped(tenant)
            .count(tenant.database().read())
            .await?;
        Ok(found)
    }
}

/// Checks the whole input and reports every problem at once. Returns the trimmed link and title.
fn check(input: &NewBookmark) -> Result<(String, String), Error> {
    let (url, title) = (input.url.trim(), input.title.trim());
    let mut problems = Vec::new();
    let is_web = url.starts_with("http://") || url.starts_with("https://");
    if !is_web || url.chars().count() > URL_MAX_LENGTH {
        problems.push(
            FieldError::new(
                "url",
                "format",
                "must be an http:// or https:// address of at most {url_max_length} characters",
            )
            .with_param("url_max_length", URL_MAX_LENGTH),
        );
    }
    if !(1..=TITLE_MAX_LENGTH).contains(&title.chars().count()) {
        problems.push(
            FieldError::new(
                "title",
                "length",
                "must be 1 to {title_max_length} characters, not only spaces",
            )
            .with_param("title_max_length", TITLE_MAX_LENGTH),
        );
    }
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    Ok((url.to_owned(), title.to_owned()))
}

// The `code` is the contract with clients and the i18n key: the English text here is the default,
// the Vietnamese one is in `locales/vi.json`.
fn not_found(id: Uuid) -> Error {
    Error::not_found("bookmarks.bookmark_not_found", "You have no bookmark `{id}`.")
        .with_param("id", id)
}

/// What the audit log keeps about a bookmark.
fn snapshot(bookmark: &Bookmark) -> Value {
    json!({"url": bookmark.url, "title": bookmark.title})
}
