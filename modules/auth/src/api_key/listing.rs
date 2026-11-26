//! Danh sách API key (tách khỏi `service.rs` theo trách nhiệm, D-20261118-file-length): key của bên gọi và key của mọi
//! người cho người quản lý.

use autoxide_kernel::access::{Credential, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use super::entity::api_key;
use super::service::{ApiKey, ApiKeyService, permissions_of, to_api_key};

impl ApiKeyService {
    /// Một trang key của người dùng `user_id`, mới tạo trước (mặc định).
    pub(crate) async fn list(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<ApiKey>, Error> {
        self.list_where(tenant, user_id, None, page).await
    }

    /// Key của bên gọi (`GET /api-keys`, tool `list_api_keys`), theo loại credential (UT-2): phiên đăng nhập thấy mọi key
    /// của mình; API key chỉ thấy chính nó (key quyền thấp không dò được các key khác của cùng chủ); token OAuth hay
    /// loại credential khác: danh sách rỗng — client làm thay người dùng không quản lý credential của họ.
    pub(crate) async fn list_own(
        &self,
        tenant: &TenantContext,
        caller: &Principal,
        page: &PageRequest,
    ) -> Result<Page<ApiKey>, Error> {
        let only = match caller.credential() {
            Credential::Session => None,
            Credential::ApiKey(own) => Some(own),
            // UUID rỗng không phải ID của key nào: trang rỗng.
            _ => Some(Uuid::nil()),
        };
        self.list_where(tenant, caller.user_id(), only, page).await
    }

    async fn list_where(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        only: Option<Uuid>,
        page: &PageRequest,
    ) -> Result<Page<ApiKey>, Error> {
        let connection = tenant.database().read();
        let mut select =
            api_key::Entity::find().filter(api_key::Column::UserId.eq(user_id)).scoped(tenant);
        if let Some(only) = only {
            select = select.filter(api_key::Column::Id.eq(only));
        }
        let page = fetch_page(select, api_key::Column::Id, |row| row.id, page, connection).await?;
        let mut permissions =
            permissions_of(connection, tenant, page.data.iter().map(|row| row.id)).await?;
        Ok(page.map(|row| {
            let granted = permissions.remove(&row.id).unwrap_or_default();
            to_api_key(row, granted)
        }))
    }

    /// Một trang key của mọi người dùng (chỉ của `owner` khi có), mới tạo trước (mặc định). Cần
    /// `auth.api_keys.manage` (D-272).
    pub(crate) async fn list_all(
        &self,
        tenant: &TenantContext,
        caller: &Principal,
        owner: Option<Uuid>,
        status: Option<&str>,
        page: &PageRequest,
    ) -> Result<Page<ApiKey>, Error> {
        // Người duyệt (không quản lý key) chỉ thấy key đang chờ duyệt (bước 54).
        let status = if caller.has(&crate::permissions::API_KEYS_MANAGE) {
            status
        } else if caller.has(&crate::permissions::API_KEYS_APPROVE) {
            Some(super::approval::PENDING)
        } else {
            // Không có quyền nào: lỗi `permission_denied` nêu quyền quản lý.
            caller.require(&crate::permissions::API_KEYS_MANAGE)?;
            status
        };
        let connection = tenant.database().read();
        let mut select = api_key::Entity::find().scoped(tenant);
        if let Some(owner) = owner {
            select = select.filter(api_key::Column::UserId.eq(owner));
        }
        if let Some(status) = status {
            select = select.filter(api_key::Column::Status.eq(status));
        }
        let page = fetch_page(select, api_key::Column::Id, |row| row.id, page, connection).await?;
        let mut permissions =
            permissions_of(connection, tenant, page.data.iter().map(|row| row.id)).await?;
        Ok(page.map(|row| {
            let granted = permissions.remove(&row.id).unwrap_or_default();
            to_api_key(row, granted)
        }))
    }
}
