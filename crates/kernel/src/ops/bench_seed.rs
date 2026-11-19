//! Đồ dùng chung cho lệnh sinh dữ liệu đo (bước 39 B1): `autoxide task <module>.bench_seed`.
//!
//! Mỗi module tự ghi bảng của mình (luật kiến trúc 1); phần này chỉ giữ những gì mọi lệnh đều
//! cần, để chúng nhận cùng tham số và cùng cách từ chối:
//!
//! - cờ xác nhận [`CONFIRM`]: thiếu thì lệnh từ chối — dữ liệu đo không bao giờ được trộn vào
//!   database thật;
//! - file ID (`ids=<đường dẫn>`, JSON `{ "<khoá>": ["<uuid>", …] }`): lệnh sau đọc ID lệnh trước
//!   ghi (vd `content` lấy ảnh của `media`, `comments` lấy bài của `content`);
//! - ghi theo lô bằng `insert_many`, chia lô theo số cột để không vượt giới hạn tham số của câu
//!   SQL (SQLite 32 766, Postgres 65 535), mỗi lô một transaction ngắn.
//!
//! Lệnh chỉ biên dịch khi bật cargo feature `bench-seed` của crate `autoxide` (bản phát hành
//! không có); phần kernel này nhỏ, không chạm database thật nếu không có lệnh gọi.

use std::path::{Path, PathBuf};

use sea_orm::{ActiveModelTrait, EntityTrait, Iterable};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::database::Database;
use crate::error::Error;
use crate::task::{TaskArgs, TaskArgument};
use crate::tenancy::{DatabaseRegistry, TenantContext};

/// Giá trị bắt buộc của tham số `confirm`.
pub const CONFIRM: &str = "yes-this-is-a-benchmark-database";

/// Khoá trong file ID của từng lệnh (lệnh sau đọc khoá của lệnh trước).
pub const USERS_KEY: &str = "users";
pub const MEDIA_KEY: &str = "media";
pub const ENTRIES_KEY: &str = "entries";

/// Số dòng tối đa mỗi transaction.
pub const ROWS_PER_TRANSACTION: usize = 1_000;

/// Số tham số tối đa của một câu `INSERT` (dưới giới hạn của SQLite, 32 766).
const MAX_PARAMETERS: usize = 30_000;

/// Tham số mọi lệnh `bench_seed` nhận.
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

/// Tham số đã kiểm của một lệnh `bench_seed`.
#[derive(Debug, Clone)]
pub struct SeedArgs {
    pub count: u64,
    pub ids: PathBuf,
    pub tenant: String,
}

impl SeedArgs {
    /// Kiểm cờ xác nhận và đọc tham số; `default_count` khi không đưa `count`.
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

    /// Khách cần điền.
    pub async fn tenant(&self, databases: &DatabaseRegistry) -> Result<TenantContext, Error> {
        databases.tenant_by_slug(&self.tenant).await
    }
}

/// Lỗi khi lệnh từ chối chạy (thiếu cờ, database đã có dữ liệu).
pub fn refused(reason: &str) -> Error {
    Error::bad_request("bench_seed_refused", "The benchmark seed task refused to run: {reason}.")
        .with_param("reason", reason)
}

/// ID đã ghi dưới khoá `key` của file `path`; file hay khoá chưa có thì lỗi nói lệnh nào cần chạy
/// trước.
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

/// Ghi `ids` dưới khoá `key` của file `path` (giữ các khoá khác).
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

/// Số dòng mỗi câu `INSERT` cho bảng có `columns` cột.
fn rows_per_statement(columns: usize) -> usize {
    (MAX_PARAMETERS / columns.max(1)).clamp(1, ROWS_PER_TRANSACTION)
}

/// Ghi `rows` vào bảng của `E`: mỗi [`ROWS_PER_TRANSACTION`] dòng một transaction, trong đó chia
/// câu `INSERT` theo số cột. Chạy như nhau trên SQLite và Postgres.
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

/// Bộ sinh số giả ngẫu nhiên tất định (SplitMix64): cùng hạt giống cho cùng bộ dữ liệu, để hai
/// lần đo so được với nhau. Không dùng cho gì cần bí mật.
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

    /// Số trong `0..bound` (`bound` > 0).
    pub fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound.max(1)
    }

    /// Một phần tử của `items` (không rỗng).
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            return None;
        }
        let index = usize::try_from(self.below(items.len() as u64)).unwrap_or(0);
        items.get(index)
    }

    /// Một câu giả gồm `words` từ.
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

/// Kho từ cho văn bản giả: có từ tiếng Việt có dấu để bài đo đi qua đường bỏ dấu khi tìm.
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
