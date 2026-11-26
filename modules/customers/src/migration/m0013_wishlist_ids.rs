//! Cột `id` (UUIDv7) của `customers_wishlist_items`, để `GET /me/wishlist` phân trang bằng cursor như mọi
//! danh sách (bước 70 P9, D-20261125-list-pages): khoá chính (`tenant_id`, `profile_id`, `product_id`,
//! `variant_key`) không có một UUID duy nhất cho `fetch_page`.
//!
//! - Dòng có sẵn được cấp ID ở đây, sinh trong ứng dụng (`Uuid::now_v7`, không dùng hàm sinh ID của
//!   database), theo lô 500 dòng, lần lượt theo `added_at` (rồi khoá chính): `now_v7` tăng dần trong một
//!   tiến trình, nên thứ tự ID của dòng cũ đúng thứ tự thêm vào — danh sách theo ID (mới nhất trước) giữ
//!   nghĩa "thêm gần nhất trước" như trước. Dòng mới nhận ID lúc thêm (`added_at` đặt cùng lúc).
//! - Postgres: cột thành `NOT NULL` sau khi điền. SQLite không đổi được cột sang `NOT NULL` mà không dựng
//!   lại bảng: cột để nullable (như `comments_targets.uid`, m0012 của `comments`); ứng dụng luôn ghi ID,
//!   index duy nhất (`tenant_id`, `id`) giữ không trùng.
//! - Index trang (`tenant_id`, `profile_id`, `id`) cho keyset của danh sách một hồ sơ.

use autoxide_kernel::migration::prelude::*;
use sea_orm::{ConnectionTrait, DbBackend};
use uuid::Uuid;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0013_wishlist_ids"
    }
}

const TABLE: &str = "customers_wishlist_items";
const UNIQUE: &str = "customers_wishlist_items_id_key";
const PAGE: &str = "customers_wishlist_items_page";
/// Số dòng mỗi lô khi điền ID.
const BATCH: u64 = 500;

fn index(name: &str, columns: &[&str], unique: bool) -> IndexCreateStatement {
    let mut index = Index::create();
    index.name(name).table(Alias::new(TABLE));
    if unique {
        index.unique();
    }
    for column in columns {
        index.col(Alias::new(*column));
    }
    index.to_owned()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(Table::alter().table(TABLE).add_column(schema::uuid_null("id")).to_owned())
            .await?;
        let connection = manager.get_connection();
        loop {
            // Dòng chưa có ID, cũ nhất trước; dòng đã điền ra khỏi điều kiện nên không cần OFFSET.
            let select = Query::select()
                .columns(["tenant_id", "profile_id", "product_id", "variant_key"])
                .from(TABLE)
                .and_where(Expr::col("id").is_null())
                .order_by("added_at", Order::Asc)
                .order_by("tenant_id", Order::Asc)
                .order_by("profile_id", Order::Asc)
                .order_by("product_id", Order::Asc)
                .order_by("variant_key", Order::Asc)
                .limit(BATCH)
                .to_owned();
            let rows = connection.query_all(&select).await?;
            if rows.is_empty() {
                break;
            }
            // Lô phải tiến: không dòng nào được cập nhật thì vòng sẽ lặp mãi trên cùng các dòng — báo lỗi thay vì treo.
            let mut updated = 0;
            for row in rows {
                let tenant: Uuid = row.try_get("", "tenant_id")?;
                let profile: Uuid = row.try_get("", "profile_id")?;
                let product: Uuid = row.try_get("", "product_id")?;
                let variant: String = row.try_get("", "variant_key")?;
                let update = Query::update()
                    .table(TABLE)
                    .value("id", Uuid::now_v7())
                    .and_where(Expr::col("tenant_id").eq(tenant))
                    .and_where(Expr::col("profile_id").eq(profile))
                    .and_where(Expr::col("product_id").eq(product))
                    .and_where(Expr::col("variant_key").eq(variant))
                    .to_owned();
                updated += connection.execute(&update).await?.rows_affected();
            }
            if updated == 0 {
                return Err(DbErr::Custom(
                    "filling the new `id` column made no progress: a batch of rows without an id matched no \
                     update"
                        .to_owned(),
                ));
            }
        }
        if manager.get_database_backend() == DbBackend::Postgres {
            manager
                .alter_table(
                    Table::alter()
                        .table(TABLE)
                        .modify_column(ColumnDef::new("id").uuid().not_null())
                        .to_owned(),
                )
                .await?;
        }
        manager.create_index(index(UNIQUE, &["tenant_id", "id"], true)).await?;
        manager.create_index(index(PAGE, &["tenant_id", "profile_id", "id"], false)).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for name in [PAGE, UNIQUE] {
            manager
                .drop_index(Index::drop().name(name).table(Alias::new(TABLE)).to_owned())
                .await?;
        }
        manager.alter_table(Table::alter().table(TABLE).drop_column("id").to_owned()).await
    }
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
    use autoxide_kernel::database::Database;
    use autoxide_kernel::migration::{Migrator, ModuleMigrations};
    use autoxide_kernel::time::Timestamp;
    use autoxide_testkit::TestDatabase;
    use jiff::SignedDuration;

    use super::*;

    // `id` đặt tên rõ bằng `AS`: SQLite đặt tên cột kết quả của `SELECT "id"` (cột thêm bằng `ALTER TABLE`)
    // là `"id"` kèm ngoặc kép. Dòng còn `NULL` kiểm riêng ở [`nulls`].
    async fn ids(database: &Database) -> Vec<(Uuid, Uuid)> {
        let select = Query::select()
            .expr_as(Expr::col("id"), Alias::new("id"))
            .column("product_id")
            .from(TABLE)
            .order_by("added_at", Order::Asc)
            .to_owned();
        let rows = database.read().query_all(&select).await.unwrap();
        rows.iter()
            .map(|row| (row.try_get("", "id").unwrap(), row.try_get("", "product_id").unwrap()))
            .collect()
    }

    async fn nulls(database: &Database) -> usize {
        let select = Query::select()
            .column("product_id")
            .from(TABLE)
            .and_where(Expr::col("id").is_null())
            .to_owned();
        database.read().query_all(&select).await.unwrap().len()
    }

    /// Dòng có sẵn (hơn một lô) được cấp ID khác nhau, thứ tự ID đúng thứ tự `added_at`; cột duy nhất trong
    /// khách; `down` bỏ cột. Cả SQLite và Postgres.
    #[tokio::test]
    async fn dong_cu_duoc_cap_id_theo_thu_tu_them() {
        TestDatabase::run(async |test| {
            let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
            let mut reader = ConfigReader::new(&source);
            let config = Config::read(&mut reader);
            let database =
                Database::connect(&reader.finish(config).unwrap().0.database).await.unwrap();
            let mut before = crate::migration::all();
            before.pop();
            Migrator::new(vec![ModuleMigrations::new("customers", before)])
                .unwrap()
                .up(&database)
                .await
                .unwrap();
            let tenant = Uuid::now_v7();
            let profile = Uuid::now_v7();
            let start = Timestamp::now() - SignedDuration::from_hours(48);
            // Hơn một lô (`BATCH`), thêm theo thứ tự ngược với thời điểm để thứ tự ghi không trùng thứ tự `added_at`.
            let count = BATCH + 7;
            let mut products = Vec::new();
            for step in (0..count).rev() {
                let product = Uuid::now_v7();
                let added = start
                    + SignedDuration::from_secs(<i64 as TryFrom<u64>>::try_from(step).unwrap());
                products.push((added, product));
                let insert = Query::insert()
                    .into_table(Alias::new(TABLE))
                    .columns(
                        ["tenant_id", "profile_id", "product_id", "variant_key", "added_at"]
                            .map(Alias::new),
                    )
                    .values_panic([
                        tenant.into(),
                        profile.into(),
                        product.into(),
                        "".into(),
                        added.into(),
                    ])
                    .to_owned();
                database.write().execute(&insert).await.unwrap();
            }
            Migrator::new(vec![ModuleMigrations::new("customers", crate::migration::all())])
                .unwrap()
                .up(&database)
                .await
                .unwrap();
            assert_eq!(nulls(&database).await, 0);
            let found = ids(&database).await;
            assert_eq!(found.len() as u64, count);
            products.sort();
            let in_order: Vec<Uuid> = products.iter().map(|(_, product)| *product).collect();
            assert_eq!(found.iter().map(|(_, product)| *product).collect::<Vec<_>>(), in_order);
            let assigned: Vec<Uuid> = found.iter().map(|(id, _)| *id).collect();
            assert!(assigned.iter().all(|id| id.get_version_num() == 7));
            assert!(
                assigned.windows(2).all(|pair| pair[0] < pair[1]),
                "ID order is the added order"
            );

            // Dòng mới nhận ID của ứng dụng; trùng ID trong cùng khách bị chặn.
            let fresh = Uuid::now_v7();
            let insert = |id: Uuid, product: Uuid| {
                Query::insert()
                    .into_table(Alias::new(TABLE))
                    .columns(
                        ["id", "tenant_id", "profile_id", "product_id", "variant_key", "added_at"]
                            .map(Alias::new),
                    )
                    .values_panic([
                        id.into(),
                        tenant.into(),
                        profile.into(),
                        product.into(),
                        "".into(),
                        Timestamp::now().into(),
                    ])
                    .to_owned()
            };
            database.write().execute(&insert(fresh, Uuid::now_v7())).await.unwrap();
            assert!(database.write().execute(&insert(fresh, Uuid::now_v7())).await.is_err());
            assert_eq!(ids(&database).await.last().unwrap().0, fresh);

            let migrator =
                Migrator::new(vec![ModuleMigrations::new("customers", crate::migration::all())])
                    .unwrap();
            migrator.down(&database, "customers", 1).await.unwrap();
            let select = Query::select().column("product_id").from(TABLE).to_owned();
            assert_eq!(database.read().query_all(&select).await.unwrap().len() as u64, count + 1);
        })
        .await
        .unwrap();
    }
}
