//! Offline sync (step 69 P2, D-20261119-step69-sync): apps on phones, computers and point-of-sale machines keep
//! a local copy of the data, write while offline, and on reconnecting upload their changes and download what
//! changed.
//!
//! The standard approach of sync systems: a **change feed with a cursor** plus **conditional writes**.
//!
//! - A module declares its syncable record kinds through `Module::sync_resources` ([`SyncResource`]) and adds
//!   [`changes_migration`] to its migrations: the `<module>_changes` table.
//! - Every write of a syncable record also writes a change row with [`SyncLog::changed`] /
//!   [`SyncLog::deleted`] in the SAME transaction (like `Outbox::record`): the change and its row
//!   either both exist or neither does.
//! - The module's endpoint `GET /api/v1/<module>/<resource>/changes?cursor=…` calls
//!   [`SyncLog::changes`]: it returns the CURRENT version of each record changed after the cursor, or a
//!   deletion mark (tombstone), filtered by tenant, by node and by read permission like a normal list.
//! - Writing back once online: `Idempotency-Key` (a resend creates no duplicate) and `If-Match` (conflict
//!   detection → `412` with the current version, [`crate::version::changed_with`]); several commands at
//!   once through `POST /api/v1/kernel/sync/batch` (the kernel's HTTP layer runs each command like a REST
//!   request).
//! - Change rows are kept [`RETENTION_DAYS`] days (a worker deletes old rows); a cursor older than that returns
//!   `410 sync_cursor_expired`: the app reloads from the start.
//!
//! The cursor is the UUIDv7 of the change row: generated in the application, increasing with time, and
//! compared the same way on SQLite and Postgres.

mod migration;

use crate::time::Timestamp;
use jiff::SignedDuration;
use sea_orm::sea_query::{Alias, Expr, ExprTrait, Order, Query};
use sea_orm::{ConnectionTrait, Value};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::access::{OrgTarget, Permission, Principal};
use crate::config::{ConfigReader, Invalid, Setting};
use crate::database::Database;
use crate::error::{Error, ErrorKind, FieldError};
use crate::pagination::{Page, Pagination};
use crate::tenancy::TenantContext;

pub use migration::changes_migration;

/// Number of days the change feed is kept.
pub const RETENTION_DAYS: Setting = Setting::new(
    "AUTOXIDE_SYNC_RETENTION_DAYS",
    "Days the server keeps the change feed of offline sync (1 to 365). A device that has not \
     synced for longer must load its data again (`sync_cursor_expired`).",
)
.with_default("30");

/// Reads [`RETENTION_DAYS`].
pub(crate) fn read_retention(reader: &mut ConfigReader<'_>) -> Result<SignedDuration, Invalid> {
    reader
        .get_with(&RETENTION_DAYS, |raw| crate::config::parse_in_range(raw, 1i64, 365))
        .map(|days| SignedDuration::from_hours(24 * days))
}

/// Default of [`RETENTION_DAYS`], for [`SyncLog::standalone`].
const DEFAULT_RETENTION_DAYS: i64 = 30;

/// Change rows newer than this may still have slower write transactions that have not committed (Postgres
/// allows many concurrent write transactions, and the ID is generated before commit): the cursor returned
/// to the client does not pass this mark, so the next call rereads the tail and misses no late-committing
/// row. Rereading a record is harmless: each entry is the current version, and reapplying gives the same
/// result.
const SETTLE: SignedDuration = SignedDuration::from_secs(10);

/// Default and maximum number of entries in a change page.
const DEFAULT_LIMIT: u32 = 100;
const MAX_LIMIT: u32 = 500;

/// A syncable record kind, declared by a module through `Module::sync_resources`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SyncResource {
    /// Name within the module (lowercase, `_`), e.g. `notes`; the full name is `<module>.<name>`.
    pub name: &'static str,
    /// Path of the list under `/api/v1/<module>`: `""` when it is `/api/v1/<module>` itself, or
    /// `/items`. The changes endpoint is `<path>/changes`; the batch accepts commands under this path.
    pub path: &'static str,
    /// Read permission, scoped by node: filters the change feed like a normal list.
    pub read: Permission,
    /// One English sentence for the app developer.
    pub description: &'static str,
}

impl SyncResource {
    pub const fn new(
        name: &'static str,
        path: &'static str,
        read: Permission,
        description: &'static str,
    ) -> Self {
        SyncResource { name, path, read, description }
    }
}

/// A problem in the `sync_resources` declaration of module `module` (the kernel reports it at startup).
pub(crate) fn declaration_problems(module: &str, resources: &[SyncResource]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut names = HashSet::new();
    for resource in resources {
        if !crate::module::is_identifier(resource.name) {
            problems.push(format!(
                "sync resource `{}` of module `{module}`: names are lowercase letters, digits and `_`",
                resource.name
            ));
        }
        if !names.insert(resource.name) {
            problems.push(format!(
                "module `{module}` declares sync resource `{}` twice",
                resource.name
            ));
        }
        let path_ok = resource.path.is_empty()
            || (resource.path.starts_with('/')
                && !resource.path.ends_with('/')
                && !resource.path.contains(['{', '}', '?', '#']));
        if !path_ok {
            problems.push(format!(
                "sync resource `{module}.{}`: `path` is `\"\"` or a fixed path like `/items`",
                resource.name
            ));
        }
        if resource.read.name().split_once('.').map(|(owner, _)| owner) != Some(module) {
            problems.push(format!(
                "sync resource `{module}.{}`: `read` must be a permission of module `{module}`",
                resource.name
            ));
        }
    }
    problems
}

/// Problems in the `sync_resources` declarations of every module, including a read permission nobody declares.
pub(crate) fn module_problems(
    modules: &[Box<dyn crate::module::Module>],
    permissions: &crate::access::PermissionCatalog,
) -> Vec<String> {
    let mut problems = Vec::new();
    for module in modules {
        let declared = module.sync_resources();
        problems.extend(declaration_problems(module.name(), &declared));
        for resource in declared {
            if !permissions.contains(resource.read.name()) {
                problems.push(format!(
                    "sync resource `{}.{}` reads with permission `{}`, which no module declares",
                    module.name(),
                    resource.name,
                    resource.read.name()
                ));
            }
        }
    }
    problems
}

/// `(module name, declaration)` of every module, in registration order.
pub(crate) fn declared(
    modules: &[Box<dyn crate::module::Module>],
) -> Vec<(&'static str, SyncResource)> {
    modules
        .iter()
        .flat_map(|module| {
            let name = module.name();
            module.sync_resources().into_iter().map(move |resource| (name, resource))
        })
        .collect()
}

/// The change feeds of the modules that declare `sync_resources`, so a worker can clean up expired rows.
pub(crate) fn logs(
    modules: &[Box<dyn crate::module::Module>],
    keep: SignedDuration,
) -> Vec<SyncLog> {
    modules
        .iter()
        .filter(|module| !module.sync_resources().is_empty())
        .map(|module| SyncLog::new(module.name(), keep))
        .collect()
}

// Tham số của endpoint thay đổi.
/// Where to start reading a change feed.
#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ChangesQuery {
    /// The `cursor` of the last change you applied. Leave it out the first time: you get every
    /// change the server still keeps (load the list endpoint first — changes carry the current
    /// record, so applying one you already have is harmless).
    pub cursor: Option<String>,
    /// Changes per page, 1 to 500 (default 100).
    pub limit: Option<u32>,
}

// Một bản ghi đã đổi: bản hiện tại, hay dấu xoá.
/// A record that changed: the record as it is now, or a deletion mark.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct SyncChange<T> {
    /// Id of the record.
    pub id: Uuid,
    /// Keep the `cursor` of the last change you applied and send it back to get the changes after
    /// it. It may lag a few seconds behind, so the latest changes can come again: apply them
    /// again, they carry the current record.
    pub cursor: String,
    /// `true`: the record was deleted, or you no longer may read it — remove it from the device.
    pub deleted: bool,
    /// Version of the record (`ETag`): send it in `If-Match` when changing it. Left out when
    /// `deleted`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The record as it is now, as reading it returns it. Left out when `deleted`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
}

impl<T> SyncChange<T> {
    /// Converts the record's shape (e.g. from the database model to the shape returned to the client).
    pub fn map<U>(self, convert: impl FnOnce(T) -> U) -> SyncChange<U> {
        SyncChange {
            id: self.id,
            cursor: self.cursor,
            deleted: self.deleted,
            version: self.version,
            data: self.data.map(convert),
        }
    }
}

/// The record a module reloads for the change feed: ID, version (`updated_at`) and the shape returned to the
/// client.
pub type Loaded<T> = Vec<(Uuid, Timestamp, T)>;

/// Where a module writes and reads its own change feed. Obtained with `StartContext::sync_log` at
/// startup; cheap to clone.
#[derive(Debug, Clone)]
pub struct SyncLog {
    table: Arc<str>,
    keep: SignedDuration,
}

impl SyncLog {
    pub(crate) fn new(module: &str, keep: SignedDuration) -> Self {
        SyncLog { table: table_name(module).into(), keep }
    }

    /// A standalone `SyncLog` (keeps the default 30 days), for module tests.
    pub fn standalone(module: &str) -> Self {
        SyncLog::new(module, SignedDuration::from_hours(24 * DEFAULT_RETENTION_DAYS))
    }

    /// Record `id` (kind `resource`, at node / owner `target`) has just been created or edited. Call it
    /// with the transaction that is writing the change.
    pub async fn changed<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        resource: &SyncResource,
        id: Uuid,
        target: OrgTarget,
    ) -> Result<(), Error> {
        self.insert(connection, tenant, resource, id, target, false).await
    }

    /// Record `id` has just been deleted: the client receives a deletion mark. `target` is the node / owner
    /// at deletion time.
    pub async fn deleted<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        resource: &SyncResource,
        id: Uuid,
        target: OrgTarget,
    ) -> Result<(), Error> {
        self.insert(connection, tenant, resource, id, target, true).await
    }

    async fn insert<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        resource: &SyncResource,
        id: Uuid,
        target: OrgTarget,
        deleted: bool,
    ) -> Result<(), Error> {
        let mut insert = Query::insert();
        insert
            .into_table(Alias::new(&*self.table))
            .columns(
                [
                    "tenant_id",
                    "id",
                    "resource",
                    "record_id",
                    "org_id",
                    "owner_id",
                    "deleted",
                    "changed_at",
                ]
                .map(Alias::new),
            )
            .values([
                tenant.id().as_uuid().into(),
                Uuid::now_v7().into(),
                resource.name.into(),
                id.into(),
                Some(target.org.as_uuid()).into(),
                target.owner.into(),
                deleted.into(),
                Timestamp::now().into(),
            ])
            .map_err(|error| Error::internal(format!("sync change insert: {error}")))?;
        connection.execute(&insert).await?;
        Ok(())
    }

    /// One page of changes of kind `resource` after the cursor of `query`, within the read permission scope
    /// of `principal`. `load` receives the IDs of the records that still exist (not deleted) and returns
    /// the current version of those the module still lets the caller read (filtered by tenant); a record
    /// not returned becomes a deletion mark.
    pub async fn changes<T, F, Fut>(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        resource: &SyncResource,
        query: &ChangesQuery,
        load: F,
    ) -> Result<Page<SyncChange<T>>, Error>
    where
        F: FnOnce(Vec<Uuid>) -> Fut,
        Fut: Future<Output = Result<Loaded<T>, Error>>,
    {
        let scope = principal.scope_condition(
            &resource.read,
            change::Column::OrgId,
            Some(change::Column::OwnerId),
        )?;
        let (cursor, limit) = self.check(query)?;
        let alias = Alias::new(change::ALIAS);
        let column = |name: &str| Expr::col((alias.clone(), Alias::new(name)));
        let mut select = Query::select();
        select
            .columns(["id", "record_id", "deleted"].map(|name| (alias.clone(), Alias::new(name))))
            .from_as(Alias::new(&*self.table), alias.clone())
            .and_where(column("tenant_id").eq(tenant.id().as_uuid()))
            .and_where(column("resource").eq(resource.name))
            .cond_where(scope)
            .order_by((alias.clone(), Alias::new("id")), Order::Asc)
            .limit(u64::from(limit) + 1);
        if let Some(cursor) = cursor {
            select.and_where(column("id").gt(cursor));
        }
        let rows = tenant.database().read().query_all(&select).await?;
        let mut rows = rows
            .iter()
            .map(|row| {
                Ok::<_, Error>((
                    row.try_get::<Uuid>("", "id")?,
                    row.try_get::<Uuid>("", "record_id")?,
                    row.try_get::<bool>("", "deleted")?,
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let more = rows.len() > limit as usize;
        rows.truncate(limit as usize);
        let next = more.then(|| rows.last().map(|(id, _, _)| id.to_string())).flatten();

        // Mỗi bản ghi một mục, ở vị trí lần đổi cuối của nó trong trang.
        let mut last: HashMap<Uuid, usize> = HashMap::new();
        for (index, (_, record, _)) in rows.iter().enumerate() {
            last.insert(*record, index);
        }
        let alive: Vec<Uuid> = rows
            .iter()
            .enumerate()
            .filter(|(index, (_, record, deleted))| !deleted && last.get(record) == Some(index))
            .map(|(_, (_, record, _))| *record)
            .collect();
        let mut loaded: HashMap<Uuid, (Timestamp, T)> = if alive.is_empty() {
            HashMap::new()
        } else {
            load(alive)
                .await?
                .into_iter()
                .map(|(id, version, data)| (id, (version, data)))
                .collect()
        };
        let settled = settled_cursor(Timestamp::now() - SETTLE);
        let mut items = Vec::with_capacity(last.len());
        for (index, (change_id, record, _)) in rows.iter().enumerate() {
            if last.get(record) != Some(&index) {
                continue;
            }
            let cursor = std::cmp::min(*change_id, settled).to_string();
            let item = match loaded.remove(record) {
                Some((version, data)) => SyncChange {
                    id: *record,
                    cursor,
                    deleted: false,
                    version: Some(crate::version::etag(version)),
                    data: Some(data),
                },
                None => {
                    SyncChange { id: *record, cursor, deleted: true, version: None, data: None }
                }
            };
            items.push(item);
        }
        Ok(Page { data: items, pagination: Pagination::new(limit, next) })
    }

    /// The cursor (checked to be within the retention period) and the number of entries of the page.
    fn check(&self, query: &ChangesQuery) -> Result<(Option<Uuid>, u32), Error> {
        let mut problems = Vec::new();
        let limit = query.limit.unwrap_or(DEFAULT_LIMIT);
        if !(1..=MAX_LIMIT).contains(&limit) {
            problems.push(
                FieldError::new("limit", "range", "must be between {min} and {max}")
                    .with_param("min", 1)
                    .with_param("max", MAX_LIMIT),
            );
        }
        let cursor = match query.cursor.as_deref().map(str::trim).filter(|raw| !raw.is_empty()) {
            None => None,
            Some(raw) => {
                match Uuid::parse_str(raw).ok().and_then(|id| Some((id, cursor_time(id)?))) {
                    Some((id, at)) => {
                        if problems.is_empty() && at < Timestamp::now() - self.keep {
                            return Err(expired());
                        }
                        Some(id)
                    }
                    None => {
                        problems.push(FieldError::new(
                            "cursor",
                            "invalid",
                            "must be the `cursor` of a change this endpoint returned",
                        ));
                        None
                    }
                }
            }
        };
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        Ok((cursor, limit))
    }

    /// Deletes change rows (all tenants) older than the retention period on `database`. Called periodically
    /// by a worker.
    pub(crate) async fn purge(&self, database: &Database) -> Result<u64, Error> {
        let before = Timestamp::now() - self.keep;
        let mut delete = Query::delete();
        delete
            .from_table(Alias::new(&*self.table))
            .and_where(Expr::col(Alias::new("changed_at")).lt(Value::from(before)));
        Ok(database.write().execute(&delete).await?.rows_affected())
    }
}

/// Change table of module `module`.
pub(crate) fn table_name(module: &str) -> String {
    format!("{module}_changes")
}

/// Error 410: the cursor is older than the retention period; reload from the start.
fn expired() -> Error {
    Error::new(
        ErrorKind::Gone,
        "sync_cursor_expired",
        "This cursor is older than the changes the server keeps. Load the list again, then ask \
         for changes without `cursor`.",
    )
}

/// The time of the cursor (UUIDv7); `None` when it is not a UUIDv7.
fn cursor_time(id: Uuid) -> Option<Timestamp> {
    if id.get_version_num() != 7 {
        return None;
    }
    let (seconds, nanos) = id.get_timestamp()?.to_unix();
    Timestamp::from_unix(i64::try_from(seconds).ok()?, nanos)
}

/// The smallest UUIDv7 of millisecond `at`: every row from that millisecond on is greater than it.
fn settled_cursor(at: Timestamp) -> Uuid {
    let millis = u64::try_from(at.as_unix_millis()).unwrap_or_default();
    let mut bytes = [0u8; 16];
    bytes[..6].copy_from_slice(&millis.to_be_bytes()[2..]);
    bytes[6] = 0x70;
    bytes[8] = 0x80;
    Uuid::from_bytes(bytes)
}

/// A "virtual" entity for building the scope condition (`Principal::scope_condition` needs the columns of
/// an entity): the read query names the `<module>_changes` table with the alias [`ALIAS`], so the column
/// `sync_change.org_id` points to the right table. No table has this name.
mod change {
    use sea_orm::entity::prelude::*;

    pub(super) const ALIAS: &str = "sync_change";

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "sync_change")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub org_id: Option<Uuid>,
        pub owner_id: Option<Uuid>,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn con_tro_on_dinh_nho_hon_moi_dong_cung_mili_giay() {
        let now = Timestamp::now();
        let cursor = settled_cursor(now);
        assert_eq!(cursor.get_version_num(), 7);
        let later = Uuid::now_v7();
        assert!(cursor <= later);
        let at = cursor_time(cursor).unwrap();
        assert_eq!(at.as_unix_millis(), now.as_unix_millis());
    }

    #[test]
    fn con_tro_sai_hay_qua_cu() {
        let log = SyncLog::standalone("notes");
        let bad = ChangesQuery { cursor: Some("abc".into()), limit: Some(0) };
        let error = log.check(&bad).unwrap_err();
        assert_eq!(error.fields().len(), 2);
        let old = settled_cursor(Timestamp::now() - SignedDuration::from_hours(24 * 31));
        let error =
            log.check(&ChangesQuery { cursor: Some(old.to_string()), limit: None }).unwrap_err();
        assert_eq!((error.kind(), error.code()), (ErrorKind::Gone, "sync_cursor_expired"));
        let fresh = Uuid::now_v7();
        let (cursor, limit) =
            log.check(&ChangesQuery { cursor: Some(fresh.to_string()), limit: None }).unwrap();
        assert_eq!((cursor, limit), (Some(fresh), DEFAULT_LIMIT));
        // UUID không phải v7 (vd v4) không là con trỏ.
        let v4 = Uuid::from_u128(0x1234_5678_9abc_4def_8123_4567_89ab_cdef);
        assert!(log.check(&ChangesQuery { cursor: Some(v4.to_string()), limit: None }).is_err());
    }

    #[test]
    fn khai_bao_sai_bi_bao() {
        const READ: Permission = Permission::new("notes.read", "Read.");
        const OTHER: Permission = Permission::new("media.read", "Read.");
        let good = SyncResource::new("notes", "", READ, "Notes.");
        assert!(declaration_problems("notes", &[good]).is_empty());
        let bad = [
            SyncResource::new("Notes", "items", READ, "x"),
            SyncResource::new("a", "/a/", OTHER, "x"),
            SyncResource::new("a", "/{id}", READ, "x"),
        ];
        assert_eq!(declaration_problems("notes", &bad).len(), 6);
    }
}
