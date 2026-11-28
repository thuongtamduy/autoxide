//! Shared helpers for the benchmark data generation commands (step 39 B1): `autoxide task <module>.bench_seed`.
//!
//! Each module writes its own tables (architecture rule 1); this part keeps only what every command needs, so
//! they accept the same parameters and refuse in the same way:
//!
//! - the confirmation flag [`CONFIRM`]: without it the command refuses, since benchmark data must never be
//!   mixed into a real database;
//! - the ID file (`ids=<path>`, JSON `{ "<key>": ["<uuid>", ...] }`): a later command reads the IDs an earlier
//!   command wrote (e.g. `content` takes images from `media`, `comments` takes articles from `content`);
//! - batched writes with `insert_many`, split by column count so the SQL statement's parameter limit is not
//!   exceeded (SQLite 32,766, Postgres 65,535), one short transaction per batch.
//!
//! The commands are compiled only when the cargo feature `bench-seed` of the `autoxide` crate is enabled
//! (release builds do not have it); this kernel part is small and does not touch a real database unless a
//! command calls it.

use std::path::{Path, PathBuf};

use sea_orm::{ActiveModelTrait, EntityTrait, Iterable};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::database::Database;
use crate::error::Error;
use crate::task::{TaskArgs, TaskArgument};
use crate::tenancy::{DatabaseRegistry, TenantContext};

/// Required value of the `confirm` parameter.
pub const CONFIRM: &str = "yes-this-is-a-benchmark-database";

/// Key in each command's ID file (a later command reads the earlier command's key).
pub const USERS_KEY: &str = "users";
pub const MEDIA_KEY: &str = "media";
pub const ENTRIES_KEY: &str = "entries";

/// Maximum rows per transaction.
pub const ROWS_PER_TRANSACTION: usize = 1_000;

/// Maximum parameters of one `INSERT` statement (below SQLite's limit of 32,766).
const MAX_PARAMETERS: usize = 30_000;

/// Parameters every `bench_seed` command accepts.
pub fn arguments() -> Vec<TaskArgument> {
    vec![
        TaskArgument::required(
            "confirm",
            "Must be `yes-this-is-a-benchmark-database`: the task writes made-up data for load \
             tests and must never run on a database with real data.",
        ),
        TaskArgument::optional("count", "How many records to make (each task has its default)."),
        TaskArgument::optional(
            "ids",
            "Path of a JSON file shared by the seed tasks: earlier tasks write the ids they made, \
             later tasks read them (default `bench-ids.json`).",
        ),
        TaskArgument::optional("tenant", "Slug of the customer to fill (default `default`)."),
    ]
}

/// Validated parameters of a `bench_seed` command.
#[derive(Debug, Clone)]
pub struct SeedArgs {
    pub count: u64,
    pub ids: PathBuf,
    pub tenant: String,
}

impl SeedArgs {
    /// Checks the confirmation flag and reads the parameters; `default_count` when `count` is not given.
    pub fn parse(args: &TaskArgs, default_count: u64) -> Result<Self, Error> {
        if args.get("confirm") != Some(CONFIRM) {
            return Err(refused(
                "pass `confirm=yes-this-is-a-benchmark-database`: this task writes made-up data",
            ));
        }
        let count = match args.get("count") {
            None => default_count,
            Some(raw) => raw
                .parse()
                .map_err(|_| refused(&format!("`count` must be a whole number, not `{raw}`")))?,
        };
        Ok(SeedArgs {
            count,
            ids: PathBuf::from(args.get("ids").unwrap_or("bench-ids.json")),
            tenant: args.get("tenant").unwrap_or("default").to_owned(),
        })
    }

    /// Tenant to fill.
    pub async fn tenant(&self, databases: &DatabaseRegistry) -> Result<TenantContext, Error> {
        databases.tenant_by_slug(&self.tenant).await
    }
}

/// Error when a command refuses to run (missing flag, database already has data).
pub fn refused(reason: &str) -> Error {
    Error::bad_request("bench_seed_refused", "The benchmark seed task refused to run: {reason}.")
        .with_param("reason", reason)
}

/// IDs written under key `key` of file `path`; when the file or key does not exist the error says which command
/// must run first.
pub fn read_ids(path: &Path, key: &str) -> Result<Vec<Uuid>, Error> {
    let all = read_file(path)?;
    let Some(Value::Array(values)) = all.get(key) else {
        return Err(refused(&format!(
            "`{}` has no `{key}` ids yet; run the task that makes them first",
            path.display()
        )));
    };
    values
        .iter()
        .map(|value| {
            value.as_str().and_then(|raw| Uuid::parse_str(raw).ok()).ok_or_else(|| {
                refused(&format!("`{key}` in `{}` is not a uuid list", path.display()))
            })
        })
        .collect()
}

/// Writes `ids` under key `key` of file `path` (keeping the other keys).
pub fn write_ids(path: &Path, key: &str, ids: &[Uuid]) -> Result<(), Error> {
    let mut all = if path.exists() { read_file(path)? } else { Map::new() };
    all.insert(key.to_owned(), ids.iter().map(|id| Value::String(id.to_string())).collect());
    let text = serde_json::to_string(&all).map_err(Error::internal)?;
    std::fs::write(path, text).map_err(Error::internal)
}

fn read_file(path: &Path) -> Result<Map<String, Value>, Error> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| refused(&format!("cannot read `{}`: {error}", path.display())))?;
    serde_json::from_str(&text)
        .map_err(|error| refused(&format!("`{}` is not a JSON object: {error}", path.display())))
}

/// Rows per `INSERT` statement for a table with `columns` columns.
fn rows_per_statement(columns: usize) -> usize {
    (MAX_PARAMETERS / columns.max(1)).clamp(1, ROWS_PER_TRANSACTION)
}

/// Writes `rows` into the table of `E`: one transaction per [`ROWS_PER_TRANSACTION`] rows, within which the
/// `INSERT` statement is split by column count. Behaves the same on SQLite and Postgres.
pub async fn insert_all<E, A>(database: &Database, rows: Vec<A>) -> Result<(), Error>
where
    E: EntityTrait,
    A: ActiveModelTrait<Entity = E> + Send,
{
    let per_statement = rows_per_statement(E::Column::iter().count());
    let mut rows = rows.into_iter().peekable();
    while rows.peek().is_some() {
        let transaction = database.begin_write().await?;
        let mut written = 0;
        while written < ROWS_PER_TRANSACTION && rows.peek().is_some() {
            let take = per_statement.min(ROWS_PER_TRANSACTION - written);
            let chunk: Vec<A> = rows.by_ref().take(take).collect();
            written += chunk.len();
            E::insert_many(chunk).exec_without_returning(&transaction).await?;
        }
        transaction.commit().await?;
    }
    Ok(())
}

/// Like [`insert_all`], for records of the `db` layer, in the tenant of `db`: batched inserts (as many rows per
/// statement as both databases allow for the record's columns, a transaction every so many rows).
pub async fn insert_records<R>(db: &crate::db::TenantDb<'_>, rows: Vec<R>) -> Result<(), Error>
where
    R: crate::db::Record,
    R::Scope: crate::db::AcceptedBy<crate::db::InTenant>,
{
    let per_statement = rows_per_statement(R::COLUMNS.len());
    let mut rows = rows.into_iter().peekable();
    while rows.peek().is_some() {
        let transaction = db.begin_write().await?;
        let mut written = 0;
        while written < ROWS_PER_TRANSACTION && rows.peek().is_some() {
            let take = per_statement.min(ROWS_PER_TRANSACTION - written);
            let chunk: Vec<R> = rows.by_ref().take(take).collect();
            written += chunk.len();
            transaction.insert_many(chunk).exec().await?;
        }
        transaction.commit().await?;
    }
    Ok(())
}

/// Deterministic pseudo-random number generator (SplitMix64): the same seed gives the same dataset, so two
/// measurements can be compared. Not for anything that needs secrecy.
#[derive(Debug, Clone)]
pub struct Pseudo(u64);

impl Pseudo {
    pub fn new(seed: u64) -> Self {
        Pseudo(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..bound` (`bound` > 0).
    pub fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound.max(1)
    }

    /// An element of `items` (non-empty).
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            return None;
        }
        let index = usize::try_from(self.below(items.len() as u64)).unwrap_or(0);
        items.get(index)
    }

    /// A fake sentence of `words` words.
    pub fn sentence(&mut self, words: usize) -> String {
        let mut text = Vec::with_capacity(words);
        for _ in 0..words {
            if let Some(word) = self.pick(WORDS) {
                text.push(*word);
            }
        }
        text.join(" ")
    }
}

/// Word bank for fake text: includes Vietnamese words with diacritics so the benchmark goes through the diacritic-stripping search path.
const WORDS: &[&str] = &[
    "autoxide",
    "backend",
    "rust",
    "server",
    "database",
    "sqlite",
    "postgres",
    "content",
    "media",
    "comment",
    "garden",
    "river",
    "mountain",
    "coffee",
    "market",
    "street",
    "morning",
    "evening",
    "travel",
    "recipe",
    "music",
    "history",
    "science",
    "design",
    "photo",
    "story",
    "guide",
    "review",
    "season",
    "festival",
    "Hà Nội",
    "Sài Gòn",
    "Đà Nẵng",
    "phở",
    "cà phê",
    "bánh mì",
    "mùa thu",
    "đường phố",
    "chợ",
    "sông",
    "núi",
    "biển",
    "trà",
    "sách",
    "nhạc",
    "ảnh",
    "người",
    "thành phố",
    "làng",
    "buổi sáng",
    "hướng dẫn",
    "đánh giá",
    "lịch sử",
    "khoa học",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chia_lo_theo_so_cot() {
        assert_eq!(rows_per_statement(10), 1_000);
        assert_eq!(rows_per_statement(45), 666);
        assert_eq!(rows_per_statement(0), 1_000);
    }

    #[test]
    fn thieu_co_xac_nhan_thi_tu_choi() {
        let task = arguments()
            .into_iter()
            .fold(crate::task::Task::new("x", "x", std::sync::Arc::new(Noop)), |task, argument| {
                task.argument(argument)
            });
        let args = task.parse(&["confirm=yes".to_owned()]).unwrap();
        assert_eq!(SeedArgs::parse(&args, 5).unwrap_err().code(), "bench_seed_refused");
        let args = task.parse(&[format!("confirm={CONFIRM}"), "count=7".to_owned()]).unwrap();
        let parsed = SeedArgs::parse(&args, 5).unwrap();
        assert_eq!((parsed.count, parsed.tenant.as_str()), (7, "default"));
    }

    #[test]
    fn file_id_giu_khoa_khac() {
        let dir = std::env::temp_dir().join(format!("autoxide-bench-ids-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ids.json");
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        write_ids(&path, "users", &[a]).unwrap();
        write_ids(&path, "media", &[b]).unwrap();
        assert_eq!(read_ids(&path, "users").unwrap(), vec![a]);
        assert_eq!(read_ids(&path, "media").unwrap(), vec![b]);
        assert_eq!(read_ids(&path, "entries").unwrap_err().code(), "bench_seed_refused");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn so_gia_tat_dinh() {
        let (mut a, mut b) = (Pseudo::new(7), Pseudo::new(7));
        assert_eq!(a.next_u64(), b.next_u64());
        assert!(a.below(10) < 10);
        assert!(!a.sentence(5).is_empty());
    }

    struct Noop;

    #[async_trait::async_trait]
    impl crate::task::TaskRun for Noop {
        async fn run(&self, _: &TaskArgs) -> Result<String, Error> {
            Ok(String::new())
        }
    }
}
