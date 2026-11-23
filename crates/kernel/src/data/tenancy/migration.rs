//! Upgrading an existing table to a tenant-level table (spec 33 "Table upgrade helper", D-253 item 5): adds
//! `tenant_id` NOT NULL **with no DEFAULT**, assigns old data to the default tenant, and changes unique keys
//! / the primary key to start with `tenant_id`.
//!
//! Approach: **rebuild the table**, one code path for both databases (SQLite cannot drop a `DEFAULT` or change
//! the primary key with `ALTER TABLE`):
//!
//! 1. read the columns (name, declared type verbatim, NOT NULL, DEFAULT), primary key, foreign keys and
//!    indexes of the old table from the database catalog;
//! 2. create `<table>__t` with `tenant_id` + every old column and the new primary key; foreign keys point to
//!    `<parent>__t` when the parent table is in the same batch;
//! 3. copy the data (parents before children), assign the default tenant, compare row counts;
//! 4. drop the old tables (children before parents);
//! 5. rename `<table>__t` → the old name (SQLite ≥ 3.26 fixes foreign key references itself; Postgres follows
//!    the OID), and Postgres renames the primary key constraint back;
//! 6. create the declared indexes.
//!
//! The helper **refuses** to run when an old index is neither re-declared (`unique` / `index` /
//! `sweep_index` with the same name) nor explicitly dropped (`drop_index`), so no index is lost silently, and
//! when the new primary key does not contain `tenant_id` (except the key `["id"]`, a globally unique UUID).
//! `down` rebuilds the table without `tenant_id` and **refuses** when data of a tenant other than the default
//! tenant remains: a down migration must not lose data. The migrator runs each migration in a transaction: a
//! failure midway changes nothing.

use std::collections::{BTreeMap, BTreeSet};

use sea_orm::{ConnectionTrait, DbBackend, QueryResult, Statement};

use super::DEFAULT_TENANT;
use crate::migration::prelude::*;

/// One table to upgrade, with its keys and indexes after the upgrade.
#[derive(Debug, Clone)]
pub struct TenantTable {
    name: String,
    primary_key: Option<Vec<&'static str>>,
    indexes: Vec<Declared>,
    drops: Vec<String>,
}

#[derive(Debug, Clone)]
struct Declared {
    name: String,
    columns: Vec<&'static str>,
    unique: bool,
}

impl TenantTable {
    pub fn new(name: impl Into<String>) -> Self {
        TenantTable { name: name.into(), primary_key: None, indexes: Vec::new(), drops: Vec::new() }
    }

    /// Unique index (the first column must be `tenant_id`).
    pub fn unique<const N: usize>(
        mut self,
        name: impl Into<String>,
        columns: [&'static str; N],
    ) -> Self {
        self.indexes.push(Declared { name: name.into(), columns: columns.to_vec(), unique: true });
        self
    }

    /// Ordinary index (the first column must be `tenant_id`).
    pub fn index<const N: usize>(
        mut self,
        name: impl Into<String>,
        columns: [&'static str; N],
    ) -> Self {
        self.indexes.push(Declared { name: name.into(), columns: columns.to_vec(), unique: false });
        self
    }

    /// Index that serves whole-database scans (expiry, queues): does not start with `tenant_id`; the name
    /// ends with `_sweep_idx`.
    pub fn sweep_index<const N: usize>(
        mut self,
        name: impl Into<String>,
        columns: [&'static str; N],
    ) -> Self {
        self.indexes.push(Declared { name: name.into(), columns: columns.to_vec(), unique: false });
        self
    }

    /// New primary key (only when changing it; must contain `tenant_id`).
    pub fn primary_key<const N: usize>(mut self, columns: [&'static str; N]) -> Self {
        self.primary_key = Some(columns.to_vec());
        self
    }

    /// Explicitly drops the old index `name` (must be stated).
    pub fn drop_index(mut self, name: impl Into<String>) -> Self {
        self.drops.push(name.into());
        self
    }

    /// Checks the declarations: ordinary / unique indexes start with `tenant_id`, sweep indexes are named `_sweep_idx`.
    fn check(&self) -> Result<(), DbErr> {
        for index in &self.indexes {
            let sweep = index.name.ends_with("_sweep_idx");
            let scoped = index.columns.first() == Some(&"tenant_id");
            if sweep == scoped || index.unique && sweep {
                return Err(custom(format!(
                    "{}: index `{}` must start with `tenant_id`, or be a non-unique index \
                     named `…_sweep_idx` that does not",
                    self.name, index.name
                )));
            }
        }
        if let Some(key) = &self.primary_key
            && key.as_slice() != ["id"]
            && !key.contains(&"tenant_id")
        {
            return Err(custom(format!(
                "{}: the new primary key must contain `tenant_id`",
                self.name
            )));
        }
        Ok(())
    }
}

/// Migration `name` that adds the index `<table>_tenant_list_idx (tenant_id, id)` for each table in `tables`.
///
/// A tenant's list sorted by `id` (`fetch_page` with `.scoped(&tenant)`) needs this index: without it, SQLite
/// reads every row of the tenant and sorts them in a temporary structure, and Postgres scans the primary key
/// skipping other tenants' rows, getting slower as the table grows (measured in step 33 C, D-302). A table
/// with such a list and no index starting with `(tenant_id, id)` yet gets it through this helper.
pub fn list_index(name: &'static str, tables: Vec<&'static str>) -> Box<dyn MigrationTrait> {
    Box::new(ListIndex { name, tables })
}

struct ListIndex {
    name: &'static str,
    tables: Vec<&'static str>,
}

impl MigrationName for ListIndex {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for ListIndex {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in &self.tables {
            manager
                .create_index(
                    // Đường dẫn đủ: tên `Index` trong file này là kiểu riêng.
                    sea_orm::sea_query::Index::create()
                        .name(format!("{table}_tenant_list_idx"))
                        .table(Alias::new(*table))
                        .col("tenant_id")
                        .col("id")
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in &self.tables {
            manager
                .drop_index(
                    sea_orm::sea_query::Index::drop()
                        .name(format!("{table}_tenant_list_idx"))
                        .table(Alias::new(*table))
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}

/// Migration `name` that upgrades the tables `tables` (parents before children when foreign keys link them).
pub fn adopt(name: &'static str, tables: Vec<TenantTable>) -> Box<dyn MigrationTrait> {
    Box::new(Adopt { name, tables })
}

struct Adopt {
    name: &'static str,
    tables: Vec<TenantTable>,
}

impl MigrationName for Adopt {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Adopt {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let connection = manager.get_connection();
        let backend = manager.get_database_backend();
        let mut plans = Vec::new();
        for table in &self.tables {
            table.check()?;
            let old = describe(connection, backend, &table.name).await?;
            if old.columns.iter().any(|column| column.name == "tenant_id") {
                return Err(custom(format!("{}: already has `tenant_id`", table.name)));
            }
            let declared: BTreeSet<&str> = table
                .indexes
                .iter()
                .map(|index| index.name.as_str())
                .chain(table.drops.iter().map(String::as_str))
                .collect();
            for index in &old.indexes {
                if index.inline {
                    // Ràng buộc unique viết liền trong `CREATE TABLE` (không có tên ổn định): phải
                    // có index unique khai thay, cùng cột sau `tenant_id`.
                    let replaced = table.indexes.iter().any(|declared| {
                        declared.unique && declared.columns[1..] == index.columns_str()[..]
                    });
                    if !replaced {
                        return Err(custom(format!(
                            "{}: the unique constraint on ({}) is not declared again; add \
                             `.unique(…, [\"tenant_id\", …])`",
                            table.name,
                            index.columns.join(", ")
                        )));
                    }
                } else if !declared.contains(index.name.as_str()) {
                    return Err(custom(format!(
                        "{}: index `{}` is not declared again; declare it (`unique` / `index` / \
                         `sweep_index` with the same name) or drop it on purpose (`drop_index`)",
                        table.name, index.name
                    )));
                }
            }
            let mut columns = vec![Column {
                name: "tenant_id".to_owned(),
                kind: uuid_type(backend).to_owned(),
                not_null: true,
                default: None,
            }];
            columns.extend(old.columns.iter().cloned());
            let primary_key = match &table.primary_key {
                Some(key) => key.iter().map(|column| (*column).to_owned()).collect(),
                None if old.primary_key == ["id"] => old.primary_key.clone(),
                None => {
                    return Err(custom(format!(
                        "{}: the primary key ({}) must start with `tenant_id`; declare it with \
                         `.primary_key([\"tenant_id\", …])`",
                        table.name,
                        old.primary_key.join(", ")
                    )));
                }
            };
            let indexes = table
                .indexes
                .iter()
                .map(|index| Index {
                    name: index.name.clone(),
                    columns: index.columns.iter().map(|column| (*column).to_owned()).collect(),
                    unique: index.unique,
                    inline: false,
                })
                .collect();
            plans.push(Plan {
                table: table.name.clone(),
                columns,
                primary_key,
                foreign_keys: old.foreign_keys,
                indexes,
                copy: Copy::AddDefaultTenant,
            });
        }
        refuse_outside_references(connection, backend, &plans).await?;
        rebuild(connection, backend, plans).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let connection = manager.get_connection();
        let backend = manager.get_database_backend();
        let mut plans = Vec::new();
        for table in &self.tables {
            let others = count(
                connection,
                backend,
                &format!(
                    "SELECT COUNT(*) FROM {} WHERE tenant_id <> {}",
                    quote(&table.name),
                    placeholder(backend, 1)
                ),
                vec![DEFAULT_TENANT.as_uuid().into()],
            )
            .await?;
            if others > 0 {
                return Err(custom(format!(
                    "{}: {others} row(s) belong to tenants other than the default one; moving \
                     back would lose them. Delete or move those tenants first",
                    table.name
                )));
            }
            let current = describe(connection, backend, &table.name).await?;
            let without = |columns: &[String]| -> Vec<String> {
                columns.iter().filter(|column| *column != "tenant_id").cloned().collect()
            };
            let columns =
                current.columns.into_iter().filter(|column| column.name != "tenant_id").collect();
            let indexes = current
                .indexes
                .iter()
                .map(|index| Index {
                    name: index.name.clone(),
                    columns: without(&index.columns),
                    unique: index.unique,
                    inline: false,
                })
                .filter(|index| !index.columns.is_empty())
                .collect();
            plans.push(Plan {
                table: table.name.clone(),
                columns,
                primary_key: without(&current.primary_key),
                foreign_keys: current.foreign_keys,
                indexes,
                copy: Copy::DropTenant,
            });
        }
        refuse_outside_references(connection, backend, &plans).await?;
        rebuild(connection, backend, plans).await
    }
}

fn custom(message: String) -> DbErr {
    DbErr::Custom(format!("tenancy: {message}"))
}

/// UUID column type as `schema::uuid` creates it on each database.
fn uuid_type(backend: DbBackend) -> &'static str {
    match backend {
        DbBackend::Postgres => "uuid",
        _ => "uuid_text",
    }
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn placeholder(backend: DbBackend, position: usize) -> String {
    match backend {
        DbBackend::Postgres => format!("${position}"),
        _ => "?".to_owned(),
    }
}

#[derive(Debug, Clone)]
struct Column {
    name: String,
    /// Declared type verbatim (e.g. `varchar(64)`, `timestamp with time zone`).
    kind: String,
    not_null: bool,
    default: Option<String>,
}

#[derive(Debug, Clone)]
struct ForeignKey {
    columns: Vec<String>,
    table: String,
    referenced: Vec<String>,
    /// `ON DELETE …` / `ON UPDATE …` (already written out), may be empty.
    actions: String,
}

#[derive(Debug, Clone)]
struct Index {
    name: String,
    columns: Vec<String>,
    unique: bool,
    /// Unique constraint written inline in `CREATE TABLE` (not a `CREATE INDEX`).
    inline: bool,
}

impl Index {
    fn columns_str(&self) -> Vec<&str> {
        self.columns.iter().map(String::as_str).collect()
    }
}

#[derive(Debug)]
struct Described {
    columns: Vec<Column>,
    primary_key: Vec<String>,
    foreign_keys: Vec<ForeignKey>,
    /// Indexes other than the primary key.
    indexes: Vec<Index>,
}

#[derive(Debug, Clone, Copy)]
enum Copy {
    AddDefaultTenant,
    DropTenant,
}

#[derive(Debug)]
struct Plan {
    table: String,
    columns: Vec<Column>,
    primary_key: Vec<String>,
    foreign_keys: Vec<ForeignKey>,
    indexes: Vec<Index>,
    copy: Copy,
}

async fn rows<C: ConnectionTrait>(
    connection: &C,
    backend: DbBackend,
    sql: String,
) -> Result<Vec<QueryResult>, DbErr> {
    connection.query_all_raw(Statement::from_string(backend, sql)).await
}

async fn execute<C: ConnectionTrait>(
    connection: &C,
    backend: DbBackend,
    sql: String,
) -> Result<(), DbErr> {
    connection.execute_raw(Statement::from_string(backend, sql)).await.map(|_| ())
}

async fn count<C: ConnectionTrait>(
    connection: &C,
    backend: DbBackend,
    sql: &str,
    values: Vec<sea_orm::Value>,
) -> Result<i64, DbErr> {
    let statement = crate::database::statement(backend, sql, values);
    let row = connection.query_one_raw(statement).await?;
    row.ok_or_else(|| custom("COUNT returned no row".to_owned()))?.try_get_by_index::<i64>(0)
}

/// Table / index names contain only letters, digits and `_` (the module naming rule), so they can be placed
/// directly into a catalog read statement safely.
fn checked(name: &str) -> Result<&str, DbErr> {
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Ok(name)
    } else {
        Err(custom(format!("unexpected name `{name}`")))
    }
}

/// Reads the columns, primary key, foreign keys and indexes of `table` from the database catalog.
async fn describe<C: ConnectionTrait>(
    connection: &C,
    backend: DbBackend,
    table: &str,
) -> Result<Described, DbErr> {
    let table = checked(table)?;
    match backend {
        DbBackend::Postgres => describe_postgres(connection, backend, table).await,
        _ => describe_sqlite(connection, backend, table).await,
    }
}

async fn describe_sqlite<C: ConnectionTrait>(
    connection: &C,
    backend: DbBackend,
    table: &str,
) -> Result<Described, DbErr> {
    let found = rows(
        connection,
        backend,
        format!(
            "SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info('{table}') \
             ORDER BY cid"
        ),
    )
    .await?;
    if found.is_empty() {
        return Err(custom(format!("table `{table}` does not exist")));
    }
    let mut columns = Vec::new();
    let mut keyed = Vec::new();
    for row in &found {
        let name: String = row.try_get_by_index(0)?;
        let position: i32 = row.try_get_by_index(4)?;
        if position > 0 {
            keyed.push((position, name.clone()));
        }
        columns.push(Column {
            name,
            kind: row.try_get_by_index(1)?,
            not_null: row.try_get_by_index::<i32>(2)? == 1,
            default: row.try_get_by_index(3)?,
        });
    }
    keyed.sort();
    let primary_key = keyed.into_iter().map(|(_, name)| name).collect();

    let mut foreign_keys: BTreeMap<i32, ForeignKey> = BTreeMap::new();
    for row in rows(
        connection,
        backend,
        format!(
            "SELECT id, \"table\", \"from\", \"to\", on_update, on_delete \
             FROM pragma_foreign_key_list('{table}') ORDER BY id, seq"
        ),
    )
    .await?
    {
        let id: i32 = row.try_get_by_index(0)?;
        let on_update: String = row.try_get_by_index(4)?;
        let on_delete: String = row.try_get_by_index(5)?;
        let entry = foreign_keys.entry(id).or_insert_with(|| ForeignKey {
            columns: Vec::new(),
            table: String::new(),
            referenced: Vec::new(),
            actions: format!("ON DELETE {on_delete} ON UPDATE {on_update}"),
        });
        entry.table = row.try_get_by_index(1)?;
        entry.columns.push(row.try_get_by_index(2)?);
        entry.referenced.push(row.try_get_by_index(3)?);
    }

    let mut indexes = Vec::new();
    for row in rows(
        connection,
        backend,
        format!("SELECT name, \"unique\", origin FROM pragma_index_list('{table}') ORDER BY name"),
    )
    .await?
    {
        let name: String = row.try_get_by_index(0)?;
        let origin: String = row.try_get_by_index(2)?;
        if origin == "pk" {
            continue;
        }
        let columns = rows(
            connection,
            backend,
            format!("SELECT name FROM pragma_index_info('{}') ORDER BY seqno", checked(&name)?),
        )
        .await?
        .iter()
        .map(|row| row.try_get_by_index::<String>(0))
        .collect::<Result<Vec<_>, _>>()?;
        indexes.push(Index {
            name,
            columns,
            unique: row.try_get_by_index::<i32>(1)? == 1,
            inline: origin == "u",
        });
    }
    Ok(Described {
        columns,
        primary_key,
        foreign_keys: foreign_keys.into_values().collect(),
        indexes,
    })
}

async fn describe_postgres<C: ConnectionTrait>(
    connection: &C,
    backend: DbBackend,
    table: &str,
) -> Result<Described, DbErr> {
    let relation = format!(
        "(SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = current_schema() AND c.relname = '{table}')"
    );
    let found = rows(
        connection,
        backend,
        format!(
            "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), \
             CASE WHEN a.attnotnull THEN 1 ELSE 0 END, pg_get_expr(d.adbin, d.adrelid) \
             FROM pg_attribute a LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
             WHERE a.attrelid = {relation} AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum"
        ),
    )
    .await?;
    if found.is_empty() {
        return Err(custom(format!("table `{table}` does not exist")));
    }
    let columns = found
        .iter()
        .map(|row| {
            Ok(Column {
                name: row.try_get_by_index(0)?,
                kind: row.try_get_by_index(1)?,
                not_null: row.try_get_by_index::<i32>(2)? == 1,
                default: row.try_get_by_index(3)?,
            })
        })
        .collect::<Result<Vec<_>, DbErr>>()?;

    // Index (kể cả khoá chính và ràng buộc unique), cột theo thứ tự.
    let found = rows(
        connection,
        backend,
        format!(
            "SELECT ic.relname::text, CASE WHEN ix.indisunique THEN 1 ELSE 0 END, \
             CASE WHEN ix.indisprimary THEN 1 ELSE 0 END, \
             CASE WHEN EXISTS (SELECT 1 FROM pg_constraint k WHERE k.conindid = ix.indexrelid \
               AND k.contype = 'u') THEN 1 ELSE 0 END, a.attname::text \
             FROM pg_index ix JOIN pg_class ic ON ic.oid = ix.indexrelid \
             JOIN LATERAL unnest(ix.indkey) WITH ORDINALITY AS k(attnum, position) ON true \
             JOIN pg_attribute a ON a.attrelid = ix.indrelid AND a.attnum = k.attnum \
             WHERE ix.indrelid = {relation} ORDER BY ic.relname, k.position"
        ),
    )
    .await?;
    let mut primary_key = Vec::new();
    let mut indexes: Vec<Index> = Vec::new();
    for row in &found {
        let name: String = row.try_get_by_index(0)?;
        let column: String = row.try_get_by_index(4)?;
        if row.try_get_by_index::<i32>(2)? == 1 {
            primary_key.push(column);
            continue;
        }
        match indexes.last_mut() {
            Some(index) if index.name == name => index.columns.push(column),
            _ => indexes.push(Index {
                name,
                columns: vec![column],
                unique: row.try_get_by_index::<i32>(1)? == 1,
                inline: row.try_get_by_index::<i32>(3)? == 1,
            }),
        }
    }

    let mut foreign_keys = Vec::new();
    for row in rows(
        connection,
        backend,
        format!(
            "SELECT (SELECT relname::text FROM pg_class WHERE oid = c.confrelid), \
             (SELECT string_agg(a.attname::text, ',' ORDER BY k.position) \
               FROM unnest(c.conkey) WITH ORDINALITY AS k(attnum, position) \
               JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.attnum), \
             (SELECT string_agg(a.attname::text, ',' ORDER BY k.position) \
               FROM unnest(c.confkey) WITH ORDINALITY AS k(attnum, position) \
               JOIN pg_attribute a ON a.attrelid = c.confrelid AND a.attnum = k.attnum), \
             c.confdeltype::text, c.confupdtype::text \
             FROM pg_constraint c WHERE c.conrelid = {relation} AND c.contype = 'f' ORDER BY c.conname"
        ),
    )
    .await?
    {
        let action = |code: String| match code.as_str() {
            "c" => "CASCADE",
            "n" => "SET NULL",
            "d" => "SET DEFAULT",
            "r" => "RESTRICT",
            _ => "NO ACTION",
        };
        let columns: String = row.try_get_by_index(1)?;
        let referenced: String = row.try_get_by_index(2)?;
        foreign_keys.push(ForeignKey {
            table: row.try_get_by_index(0)?,
            columns: columns.split(',').map(str::to_owned).collect(),
            referenced: referenced.split(',').map(str::to_owned).collect(),
            actions: format!(
                "ON DELETE {} ON UPDATE {}",
                action(row.try_get_by_index(3)?),
                action(row.try_get_by_index(4)?)
            ),
        });
    }
    Ok(Described { columns, primary_key, foreign_keys, indexes })
}

/// A table outside the batch with a foreign key pointing into a table in the batch is refused: dropping the
/// old parent table would cascade-delete (or break) the child table's data. Parent and child must be upgraded
/// in the same batch.
async fn refuse_outside_references<C: ConnectionTrait>(
    connection: &C,
    backend: DbBackend,
    plans: &[Plan],
) -> Result<(), DbErr> {
    let names: Vec<String> = plans.iter().map(|plan| format!("'{}'", plan.table)).collect();
    let sql = match backend {
        DbBackend::Postgres => format!(
            "SELECT DISTINCT r.relname::text FROM pg_constraint c \
             JOIN pg_class r ON r.oid = c.conrelid JOIN pg_class t ON t.oid = c.confrelid \
             JOIN pg_namespace n ON n.oid = t.relnamespace \
             WHERE c.contype = 'f' AND n.nspname = current_schema() AND t.relname IN ({list}) \
             AND r.relname NOT IN ({list})",
            list = names.join(", ")
        ),
        _ => format!(
            "SELECT DISTINCT m.name FROM sqlite_master m JOIN pragma_foreign_key_list(m.name) f \
             WHERE m.type = 'table' AND f.\"table\" IN ({list}) AND m.name NOT IN ({list})",
            list = names.join(", ")
        ),
    };
    let outside: Vec<String> = rows(connection, backend, sql)
        .await?
        .iter()
        .map(|row| row.try_get_by_index::<String>(0))
        .collect::<Result<_, _>>()?;
    if outside.is_empty() {
        Ok(())
    } else {
        Err(custom(format!(
            "table(s) {} have foreign keys into this batch; adopt them in the same migration",
            outside.join(", ")
        )))
    }
}

/// Rebuilds the tables according to `plans` (parents before children in the list).
async fn rebuild<C: ConnectionTrait>(
    connection: &C,
    backend: DbBackend,
    plans: Vec<Plan>,
) -> Result<(), DbErr> {
    let in_batch: BTreeSet<String> = plans.iter().map(|plan| plan.table.clone()).collect();
    let temporary = |table: &str| format!("{table}__t");
    let list = |columns: &[String]| {
        columns.iter().map(|column| quote(column)).collect::<Vec<_>>().join(", ")
    };
    // 2–3: tạo bảng mới, chép dữ liệu (cha trước con).
    for plan in &plans {
        let mut parts: Vec<String> = plan
            .columns
            .iter()
            .map(|column| {
                let mut part = format!("{} {}", quote(&column.name), column.kind);
                if column.not_null {
                    part.push_str(" NOT NULL");
                }
                if let Some(default) = &column.default {
                    part.push_str(&format!(" DEFAULT {default}"));
                }
                part
            })
            .collect();
        if !plan.primary_key.is_empty() {
            parts.push(format!("PRIMARY KEY ({})", list(&plan.primary_key)));
        }
        for key in &plan.foreign_keys {
            let target = if in_batch.contains(&key.table) {
                temporary(&key.table)
            } else {
                key.table.clone()
            };
            parts.push(format!(
                "FOREIGN KEY ({}) REFERENCES {} ({}) {}",
                list(&key.columns),
                quote(&target),
                list(&key.referenced),
                key.actions
            ));
        }
        execute(
            connection,
            backend,
            format!("CREATE TABLE {} ({})", quote(&temporary(&plan.table)), parts.join(", ")),
        )
        .await?;
        let copied: Vec<String> =
            plan.columns.iter().filter(|c| c.name != "tenant_id").map(|c| c.name.clone()).collect();
        let (sql, values) = match plan.copy {
            Copy::AddDefaultTenant => (
                format!(
                    "INSERT INTO {} (\"tenant_id\", {cols}) SELECT {}, {cols} FROM {}",
                    quote(&temporary(&plan.table)),
                    placeholder(backend, 1),
                    quote(&plan.table),
                    cols = list(&copied)
                ),
                vec![DEFAULT_TENANT.as_uuid().into()],
            ),
            Copy::DropTenant => (
                format!(
                    "INSERT INTO {} ({cols}) SELECT {cols} FROM {}",
                    quote(&temporary(&plan.table)),
                    quote(&plan.table),
                    cols = list(&copied)
                ),
                Vec::new(),
            ),
        };
        connection.execute_raw(crate::database::statement(backend, sql, values)).await?;
        let before = count(
            connection,
            backend,
            &format!("SELECT COUNT(*) FROM {}", quote(&plan.table)),
            Vec::new(),
        )
        .await?;
        let after = count(
            connection,
            backend,
            &format!("SELECT COUNT(*) FROM {}", quote(&temporary(&plan.table))),
            Vec::new(),
        )
        .await?;
        if before != after {
            return Err(custom(format!(
                "{}: copied {after} of {before} row(s); nothing was changed",
                plan.table
            )));
        }
    }
    // 4: xoá bảng cũ, con trước cha.
    for plan in plans.iter().rev() {
        execute(connection, backend, format!("DROP TABLE {}", quote(&plan.table))).await?;
    }
    // 5–6: đổi tên, tạo index.
    for plan in &plans {
        execute(
            connection,
            backend,
            format!(
                "ALTER TABLE {} RENAME TO {}",
                quote(&temporary(&plan.table)),
                quote(&plan.table)
            ),
        )
        .await?;
        if backend == DbBackend::Postgres && !plan.primary_key.is_empty() {
            execute(
                connection,
                backend,
                format!(
                    "ALTER TABLE {} RENAME CONSTRAINT {} TO {}",
                    quote(&plan.table),
                    quote(&format!("{}_pkey", temporary(&plan.table))),
                    quote(&format!("{}_pkey", plan.table))
                ),
            )
            .await?;
        }
        for index in &plan.indexes {
            execute(
                connection,
                backend,
                format!(
                    "CREATE {}INDEX {} ON {} ({})",
                    if index.unique { "UNIQUE " } else { "" },
                    quote(&index.name),
                    quote(&plan.table),
                    list(&index.columns)
                ),
            )
            .await?;
        }
    }
    Ok(())
}
