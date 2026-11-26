//! Cột `id` (UUIDv7) của `notifications_templates`, để `GET /templates` phân trang bằng cursor như mọi danh
//! sách (bước 70 P9, D-20261125-list-pages): khoá chính (`tenant_id`, `kind`, `channel`, `locale`) không có
//! một UUID duy nhất cho keyset.
//!
//! - Dòng có sẵn được cấp ID ở đây, sinh trong ứng dụng (`Uuid::now_v7`, không dùng hàm sinh ID của
//!   database), theo lô 500 dòng, lần lượt theo (`kind`, `channel`, `locale`): `now_v7` tăng dần trong một
//!   tiến trình, nên trong một loại, thứ tự ID của dòng cũ đúng thứ tự kênh / ngôn ngữ như danh sách cũ. Dòng
//!   mới nhận ID lúc ghi lần đầu; ghi lại (upsert) giữ ID.
//! - Postgres: cột thành `NOT NULL` sau khi điền. SQLite không đổi được cột sang `NOT NULL` mà không dựng
//!   lại bảng: cột để nullable (như `comments_targets.uid`, m0012 của `comments`); ứng dụng luôn ghi ID,
//!   index duy nhất (`tenant_id`, `id`) giữ không trùng (kể cả khi tách khách chép mẫu sang khách mới).
//! - Index trang (`tenant_id`, `kind`, `id`): danh sách theo loại rồi ID.

use autoxide_kernel::migration::prelude::*;
use sea_orm::{ConnectionTrait, DbBackend};
use uuid::Uuid;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0011_template_ids"
    }
}

const TABLE: &str = "notifications_templates";
const UNIQUE: &str = "notifications_templates_id_key";
const PAGE: &str = "notifications_templates_page";
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
            // Dòng chưa có ID; dòng đã điền ra khỏi điều kiện nên không cần OFFSET.
            let select = Query::select()
                .columns(["tenant_id", "kind", "channel", "locale"])
                .from(TABLE)
                .and_where(Expr::col("id").is_null())
                .order_by("kind", Order::Asc)
                .order_by("channel", Order::Asc)
                .order_by("locale", Order::Asc)
                .order_by("tenant_id", Order::Asc)
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
                let kind: String = row.try_get("", "kind")?;
                let channel: String = row.try_get("", "channel")?;
                let locale: String = row.try_get("", "locale")?;
                let update = Query::update()
                    .table(TABLE)
                    .value("id", Uuid::now_v7())
                    .and_where(Expr::col("tenant_id").eq(tenant))
                    .and_where(Expr::col("kind").eq(kind))
                    .and_where(Expr::col("channel").eq(channel))
                    .and_where(Expr::col("locale").eq(locale))
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
        manager.create_index(index(PAGE, &["tenant_id", "kind", "id"], false)).await
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

    use super::*;

    /// Câu thay thế có sẵn (hơn một lô, hai khách) được cấp ID khác nhau, trong một loại theo kênh rồi ngôn ngữ;
    /// ID duy nhất trong khách; `down` bỏ cột. Cả SQLite và Postgres.
    #[tokio::test]
    async fn cau_cu_duoc_cap_id_khac_nhau() {
        TestDatabase::run(async |test| {
            let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
            let mut reader = ConfigReader::new(&source);
            let config = Config::read(&mut reader);
            let database =
                Database::connect(&reader.finish(config).unwrap().0.database).await.unwrap();
            let mut before = crate::migration::all();
            before.pop();
            Migrator::new(vec![ModuleMigrations::new("notifications", before)])
                .unwrap()
                .up(&database)
                .await
                .unwrap();
            let tenants = [Uuid::now_v7(), Uuid::now_v7()];
            let columns =
                ["tenant_id", "kind", "channel", "locale", "title", "body", "short", "updated_at"];
            let mut count = 0u64;
            for tenant in tenants {
                for step in 0..(BATCH / 2 + 3) {
                    for locale in ["vi", "en"] {
                        let insert = Query::insert()
                            .into_table(Alias::new(TABLE))
                            .columns(columns.map(Alias::new))
                            .values_panic([
                                tenant.into(),
                                format!("shop.kind_{step:04}").into(),
                                "email".into(),
                                locale.into(),
                                "T".into(),
                                "B".into(),
                                "S".into(),
                                Timestamp::now().into(),
                            ])
                            .to_owned();
                        database.write().execute(&insert).await.unwrap();
                        count += 1;
                    }
                }
            }
            assert!(count > BATCH);
            Migrator::new(vec![ModuleMigrations::new("notifications", crate::migration::all())])
                .unwrap()
                .up(&database)
                .await
                .unwrap();
            // `id` đặt tên rõ bằng `AS`: SQLite đặt tên cột kết quả của `SELECT "id"` (cột thêm bằng `ALTER
            // TABLE`) là `"id"` kèm ngoặc kép.
            let select = Query::select()
                .expr_as(Expr::col("id"), Alias::new("id"))
                .columns(["tenant_id", "kind", "locale"])
                .from(TABLE)
                .to_owned();
            let rows = database.read().query_all(&select).await.unwrap();
            assert_eq!(rows.len() as u64, count);
            let nulls = Query::select()
                .column("kind")
                .from(TABLE)
                .and_where(Expr::col("id").is_null())
                .to_owned();
            assert!(database.read().query_all(&nulls).await.unwrap().is_empty());
            let mut ids: Vec<Uuid> = Vec::new();
            for row in &rows {
                let id: Uuid = row.try_get("", "id").unwrap();
                assert_eq!(id.get_version_num(), 7);
                ids.push(id);
            }
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len() as u64, count, "every row has its own id");
            // Trong một loại của một khách: `en` (cấp trước theo thứ tự chữ) có ID nhỏ hơn `vi`.
            let id_of = |kind: &str, locale: &str| -> Uuid {
                rows.iter()
                    .find(|row| {
                        row.try_get::<Uuid>("", "tenant_id").unwrap() == tenants[0]
                            && row.try_get::<String>("", "kind").unwrap() == kind
                            && row.try_get::<String>("", "locale").unwrap() == locale
                    })
                    .map(|row| row.try_get::<Uuid>("", "id").unwrap())
                    .unwrap()
            };
            assert!(id_of("shop.kind_0001", "en") < id_of("shop.kind_0001", "vi"));

            // Trùng ID trong cùng khách bị chặn.
            let taken = id_of("shop.kind_0000", "en");
            let duplicate = Query::update()
                .table(TABLE)
                .value("id", taken)
                .and_where(Expr::col("tenant_id").eq(tenants[0]))
                .and_where(Expr::col("kind").eq("shop.kind_0002"))
                .and_where(Expr::col("locale").eq("en"))
                .to_owned();
            assert!(database.write().execute(&duplicate).await.is_err());

            let migrator = Migrator::new(vec![ModuleMigrations::new(
                "notifications",
                crate::migration::all(),
            )])
            .unwrap();
            migrator.down(&database, "notifications", 1).await.unwrap();
            let select = Query::select().column("kind").from(TABLE).to_owned();
            assert_eq!(database.read().query_all(&select).await.unwrap().len() as u64, count);
        })
        .await
        .unwrap();
    }
}
