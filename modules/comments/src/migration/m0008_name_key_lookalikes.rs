//! Tính lại `comments_profiles.name_key` theo `text::name_key`: chữ Kirin / Hy Lạp trông như
//! chữ Latin thành chữ Latin, để tên giả chữ Latin không chiếm được tên người khác.
//!
//! Hai hồ sơ có sẵn mà nay ra cùng khoá (hiếm: một người đã giả tên người kia) thì hồ sơ đến sau
//! giữ khoá cũ — migration không đổi tên của ai; lần tới người đó đổi tên thì bị kiểm như mới.

use std::collections::HashSet;

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0008_name_key_lookalikes"
    }
}

#[autoxide_kernel::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Dùng chính hàm của module: đổi cách tính khoá về sau thì phải thêm migration tính lại
        // như migration này.
        rewrite(manager, crate::text::name_key).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Về cách của m0001: `words_key`.
        rewrite(manager, crate::text::words_key).await
    }
}

async fn rewrite(
    manager: &SchemaManager<'_>,
    compute: impl Fn(&str) -> String,
) -> Result<(), DbErr> {
    let connection = manager.get_connection();
    // Theo thứ tự tạo: hồ sơ đến trước giữ tên khi hai hồ sơ ra cùng khoá.
    let select = Query::select()
        .columns(["user_id", "display_name", "name_key"])
        .from("comments_profiles")
        .order_by("created_at", Order::Asc)
        .order_by("user_id", Order::Asc)
        .to_owned();
    let rows = connection.query_all(&select).await?;
    let mut profiles = Vec::with_capacity(rows.len());
    for row in rows {
        let user_id: uuid::Uuid = row.try_get("", "user_id")?;
        let name: String = row.try_get("", "display_name")?;
        let key: String = row.try_get("", "name_key")?;
        profiles.push((user_id, compute(&name), key));
    }
    // Một hồ sơ chỉ nhận khoá mới khi khoá đó chưa ai giữ — kể cả khoá hiện tại của mọi hồ sơ
    // khác. Như vậy không `UPDATE` nào chạm index unique (trên Postgres, một lệnh lỗi làm hỏng
    // cả transaction của migration), không cần thứ tự đổi hay khoá tạm.
    let mut taken: HashSet<String> =
        profiles.iter().map(|(_, _, current)| current.clone()).collect();
    for (user_id, wanted, current) in profiles {
        if wanted != current && taken.insert(wanted.clone()) {
            set_key(connection, user_id, &wanted).await?;
        }
    }
    Ok(())
}

async fn set_key<C: ConnectionTrait>(
    connection: &C,
    user_id: uuid::Uuid,
    key: &str,
) -> Result<(), DbErr> {
    let update = Query::update()
        .table("comments_profiles")
        .value("name_key", key)
        .and_where(Expr::col("user_id").eq(user_id))
        .to_owned();
    connection.execute(&update).await?;
    Ok(())
}
