//! Bước thiết lập của `content` (bước 51 P10): website có ít nhất một trang đã đăng. Tính từ dữ
//! liệu, không có nút "xong".

use autoxide_kernel::access::Permission;
use autoxide_kernel::error::Error;
use autoxide_kernel::setup::{SetupInfo, SetupStep};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

use crate::entity::entry::{Column, Entity};
use crate::permissions::CONTENT_PUBLISH;

pub(crate) struct FirstPage;

#[async_trait::async_trait]
impl SetupStep for FirstPage {
    fn info(&self) -> SetupInfo {
        SetupInfo::new(
            "content.first_page",
            "Publish the first page of the website",
            "content.entries",
            30,
        )
    }

    fn permission(&self) -> Permission {
        CONTENT_PUBLISH
    }

    async fn done(&self, tenant: &TenantContext) -> Result<bool, Error> {
        let published = Entity::find()
            .scoped(tenant)
            .filter(Column::Kind.eq("page"))
            .filter(Column::Status.eq("published"))
            .count(tenant.database().read())
            .await?;
        Ok(published > 0)
    }
}
