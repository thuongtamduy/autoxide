//! Số liệu gộp của thư viện cho trang quản trị (đề bài giao diện B5, D-267): tóm tắt cả thư viện,
//! số item trong từng thư mục, tên người tải lên. Mỗi thứ là một câu truy vấn cho cả trang —
//! không gọi lặp theo từng dòng.

use std::collections::HashMap;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use sea_orm::sea_query::{Expr, Func};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use uuid::Uuid;

use crate::entity::item::{Column, Entity};
use crate::permissions::MEDIA_READ;
use crate::service::MediaService;

/// Loại đối tượng "tài khoản người dùng" trong sổ đối tượng của kernel (module `auth` đăng ký;
/// chương trình không có `auth` thì không có tên). `media` không phụ thuộc `auth` (D-046).
const USER_KIND: &str = "auth.user";

/// Tóm tắt thư viện (item ngoài thùng rác).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LibrarySummary {
    pub items: i64,
    pub bytes: i64,
    pub images_missing_alt: i64,
    pub trashed_items: i64,
}

impl MediaService {
    /// Tổng số item, tổng dung lượng, số ảnh thiếu `alt` (ngoài thùng rác) và số item trong thùng
    /// rác. Cần `media.read`.
    pub(crate) async fn summary(&self, principal: &Principal) -> Result<LibrarySummary, Error> {
        principal.require(&MEDIA_READ)?;
        let read = self.database.read();
        // `SUM` của cột BIGINT trên Postgres ra NUMERIC: ép về BIGINT để đọc như nhau trên hai
        // database; không có dòng nào thì `SUM` là NULL.
        let (items, bytes): (i64, Option<i64>) = Entity::find()
            .select_only()
            .expr(Func::count(Expr::col(Column::Id)))
            .expr(Func::cast_as(Func::sum(Expr::col(Column::Size)), "BIGINT"))
            .filter(Column::TrashedAt.is_null())
            .into_tuple()
            .one(read)
            .await?
            .unwrap_or_default();
        let images_missing_alt = Self::count(
            Entity::find()
                .filter(Column::TrashedAt.is_null())
                .filter(crate::service::alt_missing()),
            read,
        )
        .await?;
        let trashed_items =
            Self::count(Entity::find().filter(Column::TrashedAt.is_not_null()), read).await?;
        Ok(LibrarySummary { items, bytes: bytes.unwrap_or(0), images_missing_alt, trashed_items })
    }

    async fn count(
        select: sea_orm::Select<Entity>,
        read: &sea_orm::DatabaseConnection,
    ) -> Result<i64, Error> {
        let count: Option<i64> = select
            .select_only()
            .expr(Func::count(Expr::col(Column::Id)))
            .into_tuple()
            .one(read)
            .await?;
        Ok(count.unwrap_or(0))
    }

    /// Số item (ngoài thùng rác) nằm thẳng trong từng thư mục `folders` (không tính thư mục con);
    /// thư mục không có item thì vắng trong kết quả.
    pub(crate) async fn folder_item_counts(
        &self,
        folders: &[Uuid],
    ) -> Result<HashMap<Uuid, i64>, Error> {
        if folders.is_empty() {
            return Ok(HashMap::new());
        }
        let rows: Vec<(Uuid, i64)> = Entity::find()
            .select_only()
            .column(Column::FolderId)
            .expr(Func::count(Expr::col(Column::Id)))
            .filter(Column::TrashedAt.is_null())
            .filter(Column::FolderId.is_in(folders.iter().copied()))
            .group_by(Column::FolderId)
            .into_tuple()
            .all(self.database.read())
            .await?;
        Ok(rows.into_iter().collect())
    }

    /// Tên hiển thị của người tải lên `ids`, tra theo lô qua sổ đối tượng. Người đã bị xoá, hay
    /// chương trình không có module tài khoản: vắng trong kết quả.
    pub(crate) async fn uploader_names(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, String>, Error> {
        if ids.is_empty() || !self.resources.knows(USER_KIND) {
            return Ok(HashMap::new());
        }
        let keys: Vec<String> = ids.iter().map(ToString::to_string).collect();
        let found = self.resources.resolve_many(tenant, USER_KIND, &keys, None).await?;
        Ok(found
            .into_iter()
            .filter_map(|(id, resource)| Some((id.parse().ok()?, resource.title)))
            .collect())
    }
}
