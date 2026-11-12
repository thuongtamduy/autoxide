//! Bảng `media_item` (item), `media_folder` (thư mục) và `media_lock`. Mọi bảng của
//! module `media` tên `media_…`.
//!
//! Hai quyết định chạy giống nhau trên cả hai database (D-086):
//!
//! - Thư mục gốc là UUID toàn số 0, không phải `NULL`: unique index coi hai `NULL` là
//!   khác nhau (cả SQLite lẫn Postgres), nên hai thư mục gốc cùng tên sẽ lọt.
//! - `name_key` (tên viết thường, để tìm và sắp) so sánh theo byte: SQLite mặc định đã
//!   vậy (`BINARY`); Postgres cần `COLLATE "C"`, không thì sắp theo locale của máy chủ.

use autoxide_kernel::migration::prelude::*;
use sea_orm::DbBackend;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0001_create_media"
    }
}

/// Cột `name_key`: chữ so sánh theo byte trên cả hai database.
fn name_key(backend: DbBackend) -> ColumnDef {
    let mut column = schema::text("name_key");
    if backend == DbBackend::Postgres {
        column.extra("COLLATE \"C\"");
    }
    column
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        manager
            .create_table(
                Table::create()
                    .table("media_folder")
                    .col(schema::pk_uuid("id"))
                    // Thư mục cha; thư mục gốc là UUID toàn số 0 (không `NULL`, xem trên).
                    .col(schema::uuid("parent_id"))
                    .col(schema::string_len("name", 255))
                    .col(name_key(backend))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        // Trong một thư mục cha không có hai thư mục cùng tên (không phân biệt hoa thường):
        // database chặn, kể cả khi hai request tạo cùng lúc. Cũng phục vụ liệt kê con.
        manager
            .create_index(
                Index::create()
                    .name("media_folder_parent_name_unique")
                    .table("media_folder")
                    .col("parent_id")
                    .col("name_key")
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("media_folder_name_idx")
                    .table("media_folder")
                    .col("name_key")
                    .col("id")
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table("media_item")
                    .col(schema::pk_uuid("id"))
                    // Thư mục chứa item; gốc là UUID toàn số 0. Không foreign key: thư mục
                    // chỉ bị xoá khi trống, service kiểm trong transaction ghi.
                    .col(schema::uuid("folder_id"))
                    .col(schema::string_len("name", 255))
                    .col(name_key(backend))
                    .col(schema::string_len("content_type", 255))
                    .col(schema::big_integer("size"))
                    .col(schema::string_len("sha256", 64))
                    .col(schema::integer_null("width"))
                    .col(schema::integer_null("height"))
                    .col(schema::text("alt"))
                    .col(schema::text("title"))
                    .col(schema::text("caption"))
                    .col(schema::string_len("visibility", 16))
                    // Phiên bản nội dung: khoá nơi lưu là `media/<id>/<version_id>`.
                    .col(schema::uuid("version_id"))
                    // Người tải lên: chỉ giữ ID, không foreign key sang `auth`. `NULL` khi
                    // tài khoản đó đã bị xoá.
                    .col(schema::uuid_null("uploaded_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .col(schema::timestamp_with_time_zone_null("trashed_at"))
                    .to_owned(),
            )
            .await?;
        // Mỗi index phục vụ một bộ lọc hay một cách sắp của `GET /media`, hoặc một job.
        for (name, columns) in [
            ("media_item_folder_idx", &["folder_id", "id"][..]),
            ("media_item_name_idx", &["name_key", "id"][..]),
            ("media_item_size_idx", &["size", "id"][..]),
            ("media_item_sha256_idx", &["sha256"][..]),
            ("media_item_trashed_idx", &["trashed_at"][..]),
            ("media_item_uploaded_by_idx", &["uploaded_by"][..]),
        ] {
            let mut index = Index::create();
            index.name(name).table("media_item");
            for column in columns {
                index.col(*column);
            }
            manager.create_index(index.to_owned()).await?;
        }

        // Một dòng duy nhất: lệnh đổi cây thư mục (chuyển thư mục) cập nhật dòng này trước,
        // nên chạy lần lượt — hai lệnh chuyển chéo nhau không tạo được vòng lặp (D-086).
        manager
            .create_table(
                Table::create()
                    .table("media_lock")
                    .col(schema::string_len("name", 64).primary_key())
                    .col(schema::big_integer("counter"))
                    .to_owned(),
            )
            .await?;
        let mut lock = Query::insert();
        lock.into_table("media_lock")
            .columns(["name", "counter"])
            .values(["folder_tree".into(), 0i64.into()])
            .map_err(|error| DbErr::Custom(format!("cannot build the folder lock row: {error}")))?;
        manager.exec_stmt(lock.to_owned()).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in ["media_lock", "media_item", "media_folder"] {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}
