//! `autoxide task auth.bench_seed` (bước 39 B1): sinh người dùng cho bài đo trên dữ liệu cỡ thật.
//!
//! Chỉ biên dịch khi bật cargo feature `bench-seed` (và trong test). Ghi thẳng bảng `auth_user`
//! của chính module theo lô, đúng định dạng như `UserService::create` (email chuẩn hoá,
//! `search_text`, realm), nhưng không phát event: dữ liệu giả không cần audit hay mail chào.
//! Mọi người dùng chung một mật khẩu (`BENCH_PASSWORD`), hash MỘT lần: hash Argon2id cho mỗi người
//! sẽ tốn hàng phút mà không đo được gì.

use std::sync::Arc;

use autoxide_kernel::bench_seed::{self, Pseudo, SeedArgs};
use autoxide_kernel::error::Error;
use autoxide_kernel::task::{Task, TaskArgs, TaskRun};
use autoxide_kernel::tenancy::{DatabaseRegistry, ScopeExt};
use chrono::{DateTime, Duration, SubsecRound, Utc};
use sea_orm::{EntityTrait, PaginatorTrait, Set};
use uuid::Uuid;

use crate::password::Passwords;
use crate::user::entity;

/// Mật khẩu của mọi người dùng sinh ra (bài đo đăng nhập dùng nó).
pub(crate) const BENCH_PASSWORD: &str = "correct horse battery staple";

/// Khoá trong file ID.
pub(crate) const IDS_KEY: &str = bench_seed::USERS_KEY;

pub(crate) fn task(databases: DatabaseRegistry) -> Task {
    bench_seed::arguments().into_iter().fold(
        Task::new(
            "bench_seed",
            "Make `count` made-up users (default 5000) for load tests, all with the password \
             `correct horse battery staple`, and write their ids to the `ids` file. Refuses to run \
             unless the customer has at most one user.",
            Arc::new(Seed { databases }),
        ),
        Task::argument,
    )
}

struct Seed {
    databases: DatabaseRegistry,
}

#[async_trait::async_trait]
impl TaskRun for Seed {
    async fn run(&self, args: &TaskArgs) -> Result<String, Error> {
        let args = SeedArgs::parse(args, 5_000)?;
        let tenant = args.tenant(&self.databases).await?;
        let existing =
            entity::Entity::find().scoped(&tenant).count(tenant.database().read()).await?;
        if existing > 1 {
            return Err(bench_seed::refused(&format!(
                "the customer already has {existing} users; seed only a fresh database"
            )));
        }
        let hash = Passwords::new().hash(BENCH_PASSWORD.to_owned()).await?;
        let rows = rows(tenant.id().as_uuid(), args.count, &hash, now());
        let ids: Vec<Uuid> = rows.iter().filter_map(|row| row.id.try_as_ref().copied()).collect();
        bench_seed::insert_all(tenant.database(), rows).await?;
        bench_seed::write_ids(&args.ids, IDS_KEY, &ids)?;
        Ok(format!("Made {} users; ids written to `{}`.", ids.len(), args.ids.display()))
    }
}

/// Dòng `auth_user` của `count` người dùng giả, tạo rải trong hai năm trước `now`.
fn rows(tenant_id: Uuid, count: u64, hash: &str, now: DateTime<Utc>) -> Vec<entity::ActiveModel> {
    let mut random = Pseudo::new(0xA17E);
    (0..count)
        .map(|index| {
            let email = format!("bench-user-{index}@example.com");
            let name = format!("{} {index}", random.sentence(2));
            let created =
                now - Duration::minutes(i64::try_from(random.below(1_051_200)).unwrap_or(0));
            entity::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant_id),
                search_text: Set(autoxide_kernel::search::fold(&format!("{email} {name}"))),
                email: Set(email),
                name: Set(name),
                password_hash: Set(Some(hash.to_owned())),
                disabled_at: Set(None),
                email_verified_at: Set(Some(created)),
                created_at: Set(created),
                updated_at: Set(created),
                last_sign_in_at: Set(None),
                locale: Set(None),
                time_zone: Set(None),
                realm: Set(entity::realm_str(autoxide_kernel::access::Realm::Staff).to_owned()),
            }
        })
        .collect()
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::bench_seed::CONFIRM;
    use autoxide_kernel::tenancy::DatabaseRegistry;
    use autoxide_testkit::TestDatabase;

    use super::*;

    #[tokio::test]
    async fn sinh_nguoi_dung_roi_tu_choi_chay_lan_hai() {
        TestDatabase::run(async |test| {
            let database = crate::testing::database(test).await;
            let ids = std::env::temp_dir().join(format!("autoxide-bench-{}.json", Uuid::now_v7()));
            let task = task(DatabaseRegistry::single(database.clone()));
            let raw = [
                format!("confirm={CONFIRM}"),
                "count=1500".to_owned(),
                format!("ids={}", ids.display()),
            ];
            let report = task.run(&task.parse(&raw).unwrap()).await.unwrap();
            assert!(report.starts_with("Made 1500 users"), "{report}");
            assert_eq!(bench_seed::read_ids(&ids, IDS_KEY).unwrap().len(), 1500);
            assert_eq!(entity::Entity::find().count(database.read()).await.unwrap(), 1500);
            // Lần hai: database đã có dữ liệu → từ chối.
            let error = task.run(&task.parse(&raw).unwrap()).await.unwrap_err();
            assert_eq!(error.code(), "bench_seed_refused");
            std::fs::remove_file(ids).unwrap();
        })
        .await
        .unwrap();
    }
}
