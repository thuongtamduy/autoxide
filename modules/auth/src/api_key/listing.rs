//! Danh sách API key (tách khỏi `service.rs` theo trách nhiệm, D-20261118-file-length): key của bên gọi và key của mọi
//! người cho người quản lý. Đọc qua lớp `db` của kernel (D-20261126-kernel-data-layer): handle `tenant.db()` tự lọc theo
//! khách.

use std::collections::HashMap;

use autoxide_kernel::access::{Credential, Principal};
use autoxide_kernel::db::{Queries, Record, TenantKey, TenantRead};
use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Page, PageRequest};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::time::Timestamp;
use uuid::Uuid;

use super::entity::api_key;
use super::service::{ApiKey, ApiKeyService, to_api_key};

/// Một dòng của `auth_api_key`, như danh sách cần.
#[derive(Record, Clone, Debug, PartialEq, Eq)]
#[record(table = "auth_api_key", scope = tenant)]
pub(crate) struct KeyRow {
    #[record(primary_key)]
    pub id: Uuid,
    pub tenant_id: TenantKey,
    pub user_id: Uuid,
    pub name: String,
    pub prefix: String,
    pub key_hash: String,
    pub expires_at: Option<Timestamp>,
    pub allowed_ips: Option<String>,
    pub rotated_from: Option<Uuid>,
    pub org_id: Option<Uuid>,
    pub last_used_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub kind: String,
    pub status: String,
    pub approved_by: Option<Uuid>,
    pub approved_at: Option<Timestamp>,
}

/// Một permission đã trao cho một key (`auth_api_key_permission`).
#[derive(Record, Clone, Debug, PartialEq, Eq)]
#[record(table = "auth_api_key_permission", scope = tenant)]
pub(crate) struct KeyPermissionRow {
    #[record(primary_key)]
    pub id: Uuid,
    pub tenant_id: TenantKey,
    pub api_key_id: Uuid,
    pub permission: String,
}

impl From<KeyRow> for api_key::Model {
    fn from(row: KeyRow) -> Self {
        api_key::Model {
            id: row.id,
            tenant_id: row.tenant_id.as_uuid(),
            user_id: row.user_id,
            name: row.name,
            prefix: row.prefix,
            key_hash: row.key_hash,
            expires_at: row.expires_at,
            allowed_ips: row.allowed_ips,
            rotated_from: row.rotated_from,
            org_id: row.org_id,
            last_used_at: row.last_used_at,
            created_at: row.created_at,
            kind: row.kind,
            status: row.status,
            approved_by: row.approved_by,
            approved_at: row.approved_at,
        }
    }
}

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
        let read = tenant.db().read();
        let mut find = read.find::<KeyRow>().filter(KeyRow::USER_ID.eq(user_id));
        if let Some(only) = only {
            find = find.filter(KeyRow::ID.eq(only));
        }
        let page = find.page(KeyRow::ID, page).await?;
        with_permissions(&read, page).await
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
        let read = tenant.db().read();
        let mut find = read.find::<KeyRow>();
        if let Some(owner) = owner {
            find = find.filter(KeyRow::USER_ID.eq(owner));
        }
        if let Some(status) = status {
            find = find.filter(KeyRow::STATUS.eq(status.to_owned()));
        }
        let page = find.page(KeyRow::ID, page).await?;
        with_permissions(&read, page).await
    }
}

/// Gắn permission đã trao vào từng key của trang (một truy vấn cho cả trang, sắp theo tên permission).
async fn with_permissions(
    read: &TenantRead<'_>,
    page: Page<KeyRow>,
) -> Result<Page<ApiKey>, Error> {
    let ids: Vec<Uuid> = page.data.iter().map(|row| row.id).collect();
    let rows = read
        .find::<KeyPermissionRow>()
        .filter(KeyPermissionRow::API_KEY_ID.is_in(ids))
        .order_asc(KeyPermissionRow::PERMISSION)
        .all()
        .await?;
    let mut granted: HashMap<Uuid, Vec<String>> = HashMap::new();
    for row in rows {
        granted.entry(row.api_key_id).or_default().push(row.permission);
    }
    Ok(page.map(|row| {
        let permissions = granted.remove(&row.id).unwrap_or_default();
        to_api_key(row.into(), permissions)
    }))
}
