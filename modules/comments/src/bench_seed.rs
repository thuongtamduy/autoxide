//! `autoxide task comments.bench_seed` (bước 39 B1): sinh bình luận đã hiện trên các bài của
//! `content`, có luồng trả lời, cho bài đo đường đọc công khai trên dữ liệu cỡ thật.
//!
//! Chỉ biên dịch khi bật cargo feature `bench-seed` (và trong test). Ghi thẳng bảng của module
//! theo lô, đúng định dạng như đường đăng bình luận (`search_key`, `match_key`, `content_hash`,
//! `root_id`, độ sâu, hồ sơ của người dùng), và số đếm khớp dữ liệu: `reply_count` của bình luận
//! gốc, `published_count` / `published_thread_count` của đích. Làm từng đích một nên biết trước số
//! trả lời của mỗi bình luận gốc, không phải sửa lại dòng đã ghi. Không phát event.

use autoxide_kernel::time::Duration;
use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::bench_seed::{self, Pseudo, SeedArgs};
use autoxide_kernel::db::{Queries as _, TenantKey};
use autoxide_kernel::error::Error;
use autoxide_kernel::task::{Task, TaskArgs, TaskRun};
use autoxide_kernel::tenancy::{DatabaseRegistry, TenantContext};
use uuid::Uuid;

use crate::entity::{comment, profile, target};
use crate::service::Status;
use crate::text::{Format, name_key};

/// Loại đích của bài viết trong sổ đối tượng.
const TARGET_TYPE: &str = "content.entry";
/// Phần bình luận gốc (còn lại là trả lời), tính theo phần nghìn.
const ROOT_PER_MILLE: u64 = 600;
/// Phần bình luận của người dùng đăng nhập (còn lại là khách), theo phần nghìn.
const USER_PER_MILLE: u64 = 700;

pub(crate) fn task(databases: DatabaseRegistry) -> Task {
    bench_seed::arguments().into_iter().fold(
        Task::new(
            "bench_seed",
            "Make `count` made-up published comments (default 1000000) with reply threads on the \
             posts listed under `entries` in the `ids` file (written by `content.bench_seed`); \
             signed-in authors come from its `users` ids. Refuses to run when the customer \
             already has comments.",
            Arc::new(Seed { databases }),
        ),
        Task::argument,
    )
}

struct Seed {
    databases: DatabaseRegistry,
}

/// Các dòng chờ ghi.
#[derive(Default)]
struct Batch {
    comments: Vec<comment::Model>,
    targets: Vec<target::Model>,
}

#[autoxide_kernel::async_trait]
impl TaskRun for Seed {
    async fn run(&self, args: &TaskArgs) -> Result<String, Error> {
        let args = SeedArgs::parse(args, 1_000_000)?;
        let tenant = args.tenant(&self.databases).await?;
        let existing = tenant.db().read().find::<comment::Model>().count().await?;
        if existing > 0 {
            return Err(bench_seed::refused(&format!(
                "the customer already has {existing} comments; seed only a fresh database"
            )));
        }
        let entries = bench_seed::read_ids(&args.ids, bench_seed::ENTRIES_KEY)?;
        if entries.is_empty() {
            return Err(bench_seed::refused("there are no entries to comment on"));
        }
        let users = bench_seed::read_ids(&args.ids, bench_seed::USERS_KEY).unwrap_or_default();
        let now = Timestamp::now();
        let users = profiles(&tenant, &users, now).await?;
        let made = comments(&tenant, &entries, &users, args.count, now).await?;
        Ok(format!("Made {made} published comments on {} posts.", entries.len()))
    }
}

/// Hồ sơ bình luận (tên hiển thị) cho mọi người dùng; trả (ID, tên).
async fn profiles(
    tenant: &TenantContext,
    users: &[Uuid],
    now: Timestamp,
) -> Result<Vec<(Uuid, String)>, Error> {
    let mut random = Pseudo::new(0x9F0);
    let named: Vec<(Uuid, String)> = users
        .iter()
        .map(|id| (*id, format!("{} {}", random.sentence(2), &id.simple().to_string()[24..])))
        .collect();
    let rows = named
        .iter()
        .map(|(id, name)| profile::Model {
            tenant_id: autoxide_kernel::db::TenantKey::new(tenant.id()),
            user_id: *id,
            display_name: name.clone(),
            name_key: name_key(name),
            created_at: now,
            updated_at: now,
        })
        .collect();
    bench_seed::insert_records(&tenant.db(), rows).await?;
    Ok(named)
}

/// Chia `count` bình luận cho các bài (lệch: bài cặp đôi một nhiều một ít, tổng đúng `count`),
/// ghi theo lô. Trả số bình luận đã ghi.
async fn comments(
    tenant: &TenantContext,
    entries: &[Uuid],
    users: &[(Uuid, String)],
    count: u64,
    now: Timestamp,
) -> Result<u64, Error> {
    let mut random = Pseudo::new(0xC0FFEE);
    let targets = entries.len() as u64;
    let (base, extra) = (count / targets, count % targets);
    let mut batch = Batch::default();
    let mut made = 0;
    let mut swing = 0;
    for (index, entry) in entries.iter().enumerate() {
        let index = index as u64;
        // Cặp bài (chẵn, lẻ) lệch cùng một lượng: tổng không đổi.
        if index.is_multiple_of(2) {
            swing = if index + 1 < targets { random.below(base / 2 + 1) } else { 0 };
        }
        let share = if index.is_multiple_of(2) { base + swing } else { base - swing };
        let share = share + u64::from(index < extra);
        if share == 0 {
            continue;
        }
        made += thread(tenant, &mut random, &mut batch, *entry, users, share, now)?;
        if batch.comments.len() >= bench_seed::ROWS_PER_TRANSACTION {
            flush(tenant, std::mem::take(&mut batch)).await?;
        }
    }
    flush(tenant, batch).await?;
    Ok(made)
}

/// `share` bình luận của bài `entry`: bình luận gốc trước, trả lời gắn vào gốc ngẫu nhiên.
fn thread(
    tenant: &TenantContext,
    random: &mut Pseudo,
    batch: &mut Batch,
    entry: Uuid,
    users: &[(Uuid, String)],
    share: u64,
    now: Timestamp,
) -> Result<u64, Error> {
    let tenant_id = TenantKey::new(tenant.id());
    let target_id = entry.to_string();
    let roots = (share * ROOT_PER_MILLE / 1000).max(1);
    let started =
        now - Duration::from_mins(i64::try_from(random.below(1_000_000)).unwrap_or(0) + 10_000);
    let mut rows: Vec<comment::Model> = Vec::new();
    for position in 0..share {
        let at = started + Duration::from_secs(i64::try_from(position * 37).unwrap_or(0));
        let words = 12 + usize::try_from(random.below(30)).unwrap_or(0);
        let body = random.sentence(words);
        let (user_id, name) = match random.pick(users) {
            Some((id, _)) if random.below(1000) < USER_PER_MILLE => (Some(*id), None),
            _ => (None, Some(format!("Guest {}", random.below(100_000)))),
        };
        let id = Uuid::now_v7();
        let (root_id, depth) = if position < roots {
            (id, 0)
        } else {
            let root = usize::try_from(random.below(roots)).unwrap_or(0);
            (rows.get(root).map_or(id, |root| root.id), 1)
        };
        let shown = name.as_deref().unwrap_or_default();
        rows.push(comment::Model {
            id,
            tenant_id,
            target_type: TARGET_TYPE.to_owned(),
            target_id: target_id.clone(),
            root_id,
            parent_id: (depth > 0).then_some(root_id),
            reply_to_id: None,
            depth,
            author_kind: if user_id.is_some() { "user" } else { "guest" }.to_owned(),
            user_id,
            author_name: name.clone(),
            author_email: None,
            email_verified: false,
            search_key: autoxide_kernel::search::fold(&format!("{body} {shown}")),
            match_key: autoxide_kernel::search::exact_key(&format!("{body} {shown}")),
            content_hash: autoxide_kernel::antispam::content_hash(&body),
            body,
            format: Format::Text.as_str().to_owned(),
            status: Status::Published.as_str().to_owned(),
            hold_reason: None,
            edit_token_hash: None,
            edited_at: None,
            published_at: Some(at),
            ip_fingerprint: None,
            locale: None,
            badges: String::new(),
            reply_count: 0,
            report_count: 0,
            reports_held_at: None,
            restricted_status: None,
            created_at: at,
            updated_at: at,
        });
    }
    // Số trả lời của mỗi gốc: biết đủ rồi mới ghi.
    let replies: Vec<Uuid> =
        rows.iter().filter(|row| row.depth > 0).map(|row| row.root_id).collect();
    for root in replies {
        if let Some(row) = rows.iter_mut().find(|row| row.id == root) {
            row.reply_count += 1;
        }
    }
    let published = i32::try_from(rows.len()).map_err(Error::internal)?;
    let threads = i32::try_from(roots).map_err(Error::internal)?;
    batch.targets.push(target::Model {
        tenant_id,
        target_type: TARGET_TYPE.to_owned(),
        target_id,
        uid: Some(Uuid::now_v7()),
        state: "open".to_owned(),
        hidden: false,
        closes_at: None,
        published_count: published,
        pending_count: 0,
        published_thread_count: threads,
        pending_thread_count: 0,
        created_at: started,
        updated_at: now,
    });
    batch.comments.extend(rows);
    Ok(u64::try_from(published).unwrap_or(0))
}

async fn flush(tenant: &TenantContext, batch: Batch) -> Result<(), Error> {
    bench_seed::insert_records(&tenant.db(), batch.targets).await?;
    bench_seed::insert_records(&tenant.db(), batch.comments).await
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::bench_seed::CONFIRM;
    use autoxide_testkit::TestDatabase;

    use super::*;

    #[tokio::test]
    async fn sinh_binh_luan_so_dem_khop() {
        TestDatabase::run(async |test| {
            let harness = crate::testing::service(test, crate::testing::config()).await;
            let ids = std::env::temp_dir().join(format!("autoxide-bench-{}.json", Uuid::now_v7()));
            let entries: Vec<Uuid> = (0..7).map(|_| Uuid::now_v7()).collect();
            let users: Vec<Uuid> = (0..4).map(|_| Uuid::now_v7()).collect();
            bench_seed::write_ids(&ids, bench_seed::ENTRIES_KEY, &entries).unwrap();
            bench_seed::write_ids(&ids, bench_seed::USERS_KEY, &users).unwrap();
            let task = task(harness.service.databases.clone());
            let raw = [
                format!("confirm={CONFIRM}"),
                "count=2500".to_owned(),
                format!("ids={}", ids.display()),
            ];
            let report = task.run(&task.parse(&raw).unwrap()).await.unwrap();
            assert!(report.starts_with("Made 2500 published comments on 7 posts"), "{report}");
            let all = autoxide_testkit::all_tenants(&harness.database);
            let read = all.read();
            assert_eq!(read.find::<comment::Model>().count().await.unwrap(), 2500);
            assert_eq!(read.find::<profile::Model>().count().await.unwrap(), 4);
            // Số đếm của đích và của bình luận gốc khớp dòng thật.
            let mut total = 0;
            for row in read.find::<target::Model>().all().await.unwrap() {
                let of = |depth: i32| {
                    read.find::<comment::Model>()
                        .filter(comment::Model::TARGET_ID.eq(row.target_id.as_str().to_owned()))
                        .filter(comment::Model::DEPTH.eq(depth))
                        .count()
                };
                let (roots, replies) = (of(0).await.unwrap(), of(1).await.unwrap());
                assert_eq!(i64::from(row.published_count), roots + replies);
                assert_eq!(i64::from(row.published_thread_count), roots);
                total += roots + replies;
            }
            assert_eq!(total, 2500);
            let roots = read
                .find::<comment::Model>()
                .filter(comment::Model::DEPTH.eq(0))
                .all()
                .await
                .unwrap();
            let replies: i64 = roots.iter().map(|root| i64::from(root.reply_count)).sum();
            assert_eq!(
                replies,
                read.find::<comment::Model>()
                    .filter(comment::Model::DEPTH.eq(1))
                    .count()
                    .await
                    .unwrap()
            );
            // Lần hai: đã có bình luận → từ chối.
            let error = task.run(&task.parse(&raw).unwrap()).await.unwrap_err();
            assert_eq!(error.code(), "bench_seed_refused");
            std::fs::remove_file(ids).unwrap();
        })
        .await
        .unwrap();
    }
}
