//! Index cho các danh sách của `auth` có phân trang keyset (bước 70 P9 mục 3, D-20261125-list-pages): cột lọc +
//! cột sắp + id. Uỷ quyền nhận từ khách cấp trên (lọc theo khách con), liên kết đăng nhập của một người, nhà
//! cung cấp đăng nhập đang bật. `auth_identity_user_idx` (tenant_id, user_id) là tiền tố của index mới nên bỏ
//! (`down` tạo lại). Các danh sách khác đã có index khớp: khoá chính (tenant_id, id); role theo tên — unique
//! (tenant_id, name); nút của membership — unique (tenant_id, user_id, org_id, role_id); lần cấp hỗ trợ theo
//! khách — (tenant_id, target_tenant_id, id).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0047_list_page_indexes"
    }
}

/// `(bảng, tên index, cột)` thêm.
const ADDED: [(&str, &str, &[&str]); 3] = [
    ("auth_delegation", "auth_delegation_child_page_idx", &["tenant_id", "child_tenant_id", "id"]),
    ("auth_identity", "auth_identity_user_page_idx", &["tenant_id", "user_id", "id"]),
    ("auth_oidc_provider", "auth_oidc_provider_enabled_idx", &["tenant_id", "enabled", "id"]),
];

fn index(table: &str, name: &str, columns: &[&str]) -> IndexCreateStatement {
    let mut index = Index::create();
    index.name(name).table(Alias::new(table));
    for column in columns {
        index.col(Alias::new(*column));
    }
    index.to_owned()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, name, columns) in ADDED {
            manager.create_index(index(table, name, columns)).await?;
        }
        manager
            .drop_index(
                Index::drop().name("auth_identity_user_idx").table("auth_identity").to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_index(index(
                "auth_identity",
                "auth_identity_user_idx",
                &["tenant_id", "user_id"],
            ))
            .await?;
        for (table, name, _) in ADDED {
            manager
                .drop_index(Index::drop().name(name).table(Alias::new(table)).to_owned())
                .await?;
        }
        Ok(())
    }
}
