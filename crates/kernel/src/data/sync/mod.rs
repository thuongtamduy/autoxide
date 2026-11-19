//! Đồng bộ khi mất mạng (bước 69 P2, D-20261119-step69-sync): app trên điện thoại, máy tính, máy
//! bán hàng giữ bản sao dữ liệu, ghi tạm khi mất mạng, có mạng lại thì gửi lên và tải phần đã đổi.
//!
//! Cách làm chuẩn của các hệ đồng bộ: **luồng thay đổi theo con trỏ** + **ghi có điều kiện**.
//!
//! - Module khai loại bản ghi đồng bộ được qua `Module::sync_resources` ([`SyncResource`]) và thêm
//!   [`changes_migration`] vào migration của mình: bảng `<module>_changes`.
//! - Mỗi lệnh ghi bản ghi đồng bộ được ghi thêm một dòng thay đổi bằng [`SyncLog::changed`] /
//!   [`SyncLog::deleted`] trong CÙNG transaction (như `Outbox::record`): thay đổi và dòng của nó
//!   cùng có hoặc cùng không.
//! - Endpoint `GET /api/v1/<module>/<resource>/changes?cursor=…` của module gọi
//!   [`SyncLog::changes`]: trả bản HIỆN TẠI của mỗi bản ghi đã đổi sau con trỏ, hay dấu xoá
//!   (tombstone), lọc theo khách, theo nút và theo permission đọc như danh sách thường.
//! - Ghi lại khi có mạng: `Idempotency-Key` (gửi lại không trùng) và `If-Match` (phát hiện xung
//!   đột → `412` kèm bản hiện tại, [`crate::version::changed_with`]); nhiều lệnh một lần qua
//!   `POST /api/v1/kernel/sync/batch` (lớp HTTP của kernel chạy từng lệnh như một request REST).
//! - Dòng thay đổi giữ [`RETENTION_DAYS`] ngày (worker xoá dòng cũ); con trỏ cũ hơn thế trả
//!   `410 sync_cursor_expired`: app tải lại từ đầu.
//!
//! Con trỏ là UUIDv7 của dòng thay đổi — sinh trong ứng dụng, tăng theo thời gian, so sánh giống
//! nhau trên SQLite và Postgres.

mod migration;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;

use chrono::{DateTime, Duration, SubsecRound, Utc};
use sea_orm::sea_query::{Alias, Expr, ExprTrait, Order, Query};
use sea_orm::{ConnectionTrait, Value};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::access::{OrgTarget, Permission, Principal};
use crate::config::{ConfigReader, Invalid, Setting};
use crate::database::Database;
use crate::error::{Error, ErrorKind, FieldError};
use crate::pagination::{Page, Pagination};
use crate::tenancy::TenantContext;

pub use migration::changes_migration;

/// Số ngày giữ luồng thay đổi.
pub const RETENTION_DAYS: Setting = Setting::new(
    "AUTOXIDE_SYNC_RETENTION_DAYS",
    "Days the server keeps the change feed of offline sync (1 to 365). A device that has not \
     synced for longer must load its data again (`sync_cursor_expired`).",
)
.with_default("30");

/// Đọc [`RETENTION_DAYS`].
pub(crate) fn read_retention(reader: &mut ConfigReader<'_>) -> Result<Duration, Invalid> {
    reader
        .get_with(&RETENTION_DAYS, |raw| crate::config::parse_in_range(raw, 1i64, 365))
        .map(Duration::days)
}

/// Mặc định của [`RETENTION_DAYS`], cho [`SyncLog::standalone`].
const DEFAULT_RETENTION_DAYS: i64 = 30;

/// Dòng thay đổi mới hơn chừng này có thể còn transaction ghi chậm hơn chưa commit (Postgres cho
/// nhiều transaction ghi cùng lúc, ID sinh trước khi commit): con trỏ trả cho client không vượt quá
/// mốc này, để lần hỏi sau đọc lại khúc cuối — không bỏ sót dòng commit muộn. Đọc lại một bản ghi
/// không sao: mỗi mục là bản hiện tại, áp lại cho cùng kết quả.
const SETTLE: Duration = Duration::seconds(10);

/// Số mục mặc định và tối đa của một trang thay đổi.
const DEFAULT_LIMIT: u32 = 100;
const MAX_LIMIT: u32 = 500;

/// Một loại bản ghi đồng bộ được, module khai qua `Module::sync_resources`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SyncResource {
    /// Tên trong module (chữ thường, `_`), vd `notes`; tên đầy đủ là `<module>.<name>`.
    pub name: &'static str,
    /// Đường dẫn của danh sách dưới `/api/v1/<module>`: `""` khi chính là `/api/v1/<module>`, hay
    /// `/items`. Endpoint thay đổi là `<đường dẫn>/changes`; lô ghi nhận lệnh dưới đường dẫn này.
    pub path: &'static str,
    /// Permission đọc, có phạm vi theo nút: lọc luồng thay đổi như danh sách thường.
    pub read: Permission,
    /// Một câu tiếng Anh cho người làm app.
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

/// Vấn đề trong khai báo `sync_resources` của module `module` (kernel báo lúc khởi động).
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
    /// Đổi dạng của bản ghi (vd từ model của database sang dạng trả cho client).
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

/// Bản ghi module tải lại cho luồng thay đổi: ID, phiên bản (`updated_at`) và dạng trả cho client.
pub type Loaded<T> = Vec<(Uuid, DateTime<Utc>, T)>;

/// Nơi một module ghi và đọc luồng thay đổi của mình. Lấy bằng `StartContext::sync_log` lúc khởi
/// động; clone rẻ.
#[derive(Debug, Clone)]
pub struct SyncLog {
    table: Arc<str>,
    keep: Duration,
}

impl SyncLog {
    pub(crate) fn new(module: &str, keep: Duration) -> Self {
        SyncLog { table: table_name(module).into(), keep }
    }

    /// `SyncLog` đứng riêng (giữ mặc định 30 ngày) — cho test của module.
    pub fn standalone(module: &str) -> Self {
        SyncLog::new(module, Duration::days(DEFAULT_RETENTION_DAYS))
    }

    /// Bản ghi `id` (loại `resource`, ở nút / người sở hữu `target`) vừa được tạo hay sửa. Gọi với
    /// transaction đang ghi thay đổi.
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

    /// Bản ghi `id` vừa bị xoá: client nhận dấu xoá. `target` là nút / người sở hữu lúc xoá.
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
                Utc::now().trunc_subsecs(6).into(),
            ])
            .map_err(|error| Error::internal(format!("sync change insert: {error}")))?;
        connection.execute(&insert).await?;
        Ok(())
    }

    /// Một trang thay đổi của loại `resource` sau con trỏ của `query`, trong phạm vi permission
    /// đọc của `principal`. `load` nhận ID các bản ghi còn (chưa xoá) và trả bản hiện tại của
    /// những bản module còn cho người gọi đọc (lọc theo khách); bản không trả về thành dấu xoá.
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
        let mut loaded: HashMap<Uuid, (DateTime<Utc>, T)> = if alive.is_empty() {
            HashMap::new()
        } else {
            load(alive)
                .await?
                .into_iter()
                .map(|(id, version, data)| (id, (version, data)))
                .collect()
        };
        let settled = settled_cursor(Utc::now() - SETTLE);
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

    /// Con trỏ (đã kiểm còn trong thời hạn giữ) và số mục của trang.
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
                        if problems.is_empty() && at < Utc::now() - self.keep {
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

    /// Xoá dòng thay đổi (mọi khách) cũ hơn thời hạn giữ trên `database`. Worker gọi định kỳ.
    pub(crate) async fn purge(&self, database: &Database) -> Result<u64, Error> {
        let before = Utc::now() - self.keep;
        let mut delete = Query::delete();
        delete
            .from_table(Alias::new(&*self.table))
            .and_where(Expr::col(Alias::new("changed_at")).lt(Value::from(before)));
        Ok(database.write().execute(&delete).await?.rows_affected())
    }
}

/// Bảng thay đổi của module `module`.
pub(crate) fn table_name(module: &str) -> String {
    format!("{module}_changes")
}

/// Lỗi 410: con trỏ cũ hơn thời hạn giữ — tải lại từ đầu.
fn expired() -> Error {
    Error::new(
        ErrorKind::Gone,
        "sync_cursor_expired",
        "This cursor is older than the changes the server keeps. Load the list again, then ask \
         for changes without `cursor`.",
    )
}

/// Thời điểm của con trỏ (UUIDv7); `None` khi không phải UUIDv7.
fn cursor_time(id: Uuid) -> Option<DateTime<Utc>> {
    if id.get_version_num() != 7 {
        return None;
    }
    let (seconds, nanos) = id.get_timestamp()?.to_unix();
    DateTime::from_timestamp(i64::try_from(seconds).ok()?, nanos)
}

/// UUIDv7 nhỏ nhất của mili giây `at`: mọi dòng từ mili giây đó trở đi lớn hơn nó.
fn settled_cursor(at: DateTime<Utc>) -> Uuid {
    let millis = u64::try_from(at.timestamp_millis()).unwrap_or_default();
    let mut bytes = [0u8; 16];
    bytes[..6].copy_from_slice(&millis.to_be_bytes()[2..]);
    bytes[6] = 0x70;
    bytes[8] = 0x80;
    Uuid::from_bytes(bytes)
}

/// Entity "ảo" để dựng điều kiện phạm vi (`Principal::scope_condition` cần cột của một entity):
/// câu đọc gọi bảng `<module>_changes` bằng bí danh [`ALIAS`], nên cột `sync_change.org_id` trỏ
/// đúng bảng. Không có bảng nào tên này.
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
        let now = Utc::now();
        let cursor = settled_cursor(now);
        assert_eq!(cursor.get_version_num(), 7);
        let later = Uuid::now_v7();
        assert!(cursor <= later);
        let at = cursor_time(cursor).unwrap();
        assert_eq!(at.timestamp_millis(), now.timestamp_millis());
    }

    #[test]
    fn con_tro_sai_hay_qua_cu() {
        let log = SyncLog::standalone("notes");
        let bad = ChangesQuery { cursor: Some("abc".into()), limit: Some(0) };
        let error = log.check(&bad).unwrap_err();
        assert_eq!(error.fields().len(), 2);
        let old = settled_cursor(Utc::now() - Duration::days(31));
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
