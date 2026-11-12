//! Index cho job dọn dẹp (D-155): `auth_refresh_token(expires_at)` và `(revoked_at)`. Job dọn
//! hằng giờ xoá theo hai cột này; thiếu index thì mỗi lần quét cả bảng (mỗi lần refresh thêm
//! một dòng, bảng lớn nhanh) và, trên SQLite, giữ kết nối ghi duy nhất suốt lúc quét.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0021_add_cleanup_indexes"
    }
}

const INDEXES: [(&str, &str, &[&str]); 2] = [
    ("auth_refresh_token_expires_idx", "auth_refresh_token", &["expires_at"]),
    ("auth_refresh_token_revoked_idx", "auth_refresh_token", &["revoked_at"]),
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (name, table, columns) in INDEXES {
            let mut index = Index::create();
            index.name(name).table(table);
            for column in columns {
                index.col(*column);
            }
            manager.create_index(index.to_owned()).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (name, table, _) in INDEXES {
            manager.drop_index(Index::drop().name(name).table(table).to_owned()).await?;
        }
        Ok(())
    }
}
