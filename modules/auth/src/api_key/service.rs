//! Service của API key: tạo, liệt kê, thu hồi, và xác thực request mang API key.
//!
//! Quyền thật của một key = permission ghi trên key ∩ permission HIỆN TẠI của chủ
//! key. Chủ key mất một quyền (đổi role) thì key mất theo ngay; chủ key bị khoá thì
//! key không dùng được.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, Set,
};
use uuid::Uuid;

use super::entity::{api_key, api_key_permission};
use crate::events;
use crate::role::RoleService;
use crate::token;
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::tenancy::{
    DEFAULT_TENANT, OrgId, OrgTree, ScopeExt, TenantContext, TenantId, root_of,
};
use serde_json::json;
use uuid::fmt::Simple;

/// Mọi API key bắt đầu bằng chuỗi này: nhìn là biết đây là API key (không phải access
/// token), và công cụ quét secret nhận ra được khi key lỡ bị commit.
///
/// Dạng key (bước 33, D-285): `axk1_<ID khách: 32 chữ hex thường>_<phần ngẫu nhiên>` — khách đọc
/// được theo độ dài cố định để chọn database TRƯỚC khi tra bản băm (phần ngẫu nhiên là base64url,
/// có thể chứa `_`, nên không tách theo `_`). Key cấp trước bước 33 có dạng `axk_<phần ngẫu nhiên>`
/// ([`LEGACY_API_KEY_PREFIX`]) và thuộc khách mặc định.
pub(crate) const API_KEY_PREFIX: &str = "axk1_";
/// Tiền tố của key cấp trước bước 33 (không mang khách).
pub(crate) const LEGACY_API_KEY_PREFIX: &str = "axk_";

/// Số byte ngẫu nhiên của một key: 256 bit.
const KEY_BYTES: usize = 32;
/// Số ký tự đầu của phần ngẫu nhiên được lưu và hiện ra để nhận diện key (sau `axk1_…`).
const PREFIX_RANDOM_CHARS: usize = 8;
const NAME_MAX_LENGTH: usize = 100;
const MAX_PERMISSIONS: usize = 200;
/// Số key tối đa của một người dùng.
const MAX_KEYS_PER_USER: u64 = 100;
/// Ghi `last_used_at` nhiều nhất một lần mỗi chừng này giây cho mỗi key: biết key còn
/// được dùng mà không biến mỗi request thành một lần ghi.
const LAST_USED_RESOLUTION_SECONDS: i64 = 60;

/// Một API key (không có phần bí mật).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApiKey {
    pub id: Uuid,
    /// Chủ của key.
    pub user_id: Uuid,
    pub name: String,
    /// Vài ký tự đầu của key, để nhận diện.
    pub prefix: String,
    /// Theo thứ tự tên.
    pub permissions: Vec<String>,
    /// Nút của hàng rào (bước 34): key chỉ có quyền trong nhánh này; nút gốc = không rào.
    pub org_id: Uuid,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Key vừa tạo, kèm phần bí mật — chỉ có ở lúc tạo.
#[derive(Debug, Clone)]
pub(crate) struct CreatedApiKey {
    pub key: ApiKey,
    pub secret: String,
}

#[derive(Debug, Clone)]
pub(crate) struct NewApiKey {
    pub name: String,
    pub permissions: Vec<String>,
    /// Nút của hàng rào; `None`: nút gốc (không rào). Phải là nút người tạo làm việc ở đó.
    pub org_id: Option<Uuid>,
    /// `None`: không hết hạn.
    pub expires_at: Option<DateTime<Utc>>,
    /// Mật khẩu hiện tại của người tạo: access token bị lộ không đủ để tạo key sống lâu
    /// hơn nó (D-079). Rỗng khi xác nhận bằng `second_factor`.
    pub current_password: String,
    /// Xác nhận lại bằng mã TOTP hay passkey thay cho mật khẩu (D-272).
    pub second_factor: Option<Reauth>,
}

/// Cách xác nhận lại không dùng mật khẩu.
#[derive(Debug, Clone)]
pub(crate) enum Reauth {
    /// Mã 6 số của ứng dụng xác thực (đã bật TOTP).
    Totp(String),
    /// Câu trả lời của passkey cho nghi thức `ceremony_id` (từ `POST /login/passkey/options`).
    Passkey { ceremony_id: Uuid, credential: serde_json::Value },
    /// Token xác nhận lại từ `POST /me/reauth` (D-308).
    Token(String),
}

pub(crate) struct ApiKeyService {
    roles: Arc<RoleService>,
    sessions: Arc<crate::session::SessionService>,
    outbox: Outbox,
    mfa: Arc<crate::mfa::MfaService>,
    passkeys: Arc<crate::passkey::PasskeyService>,
}

impl ApiKeyService {
    pub(crate) fn new(
        roles: Arc<RoleService>,
        sessions: Arc<crate::session::SessionService>,
        outbox: Outbox,
        mfa: Arc<crate::mfa::MfaService>,
        passkeys: Arc<crate::passkey::PasskeyService>,
    ) -> Self {
        ApiKeyService { roles, sessions, outbox, mfa, passkeys }
    }

    /// Tạo key cho `creator`, với permission nằm trong số permission `creator` đang có.
    pub(crate) async fn create(
        &self,
        tenant: &TenantContext,
        creator: &Principal,
        input: NewApiKey,
    ) -> Result<CreatedApiKey, Error> {
        // Tính năng theo gói (bước 35): khách bị gói giới hạn cần `auth.api_keys`.
        self.roles.entitlements().require_feature(tenant.id(), &super::API_KEYS_FEATURE).await?;
        if creator.api_key_id().is_some() {
            return Err(Error::forbidden(
                "auth.api_key_cannot_create_keys",
                "An API key cannot create API keys: a leaked key must not be able to make \
                 more. Sign in with a password to create keys.",
            ));
        }
        let now = now();
        // Kiểm đầu vào (gom mọi lỗi, kể cả thiếu mật khẩu) TRƯỚC khi kiểm mật khẩu. Xác nhận bằng
        // mã hay passkey thì không cần mật khẩu, nhưng không được gửi cả hai (D-272).
        let blank = match (&input.second_factor, input.current_password.trim().is_empty()) {
            (None, _) => FieldError::blank("current_password", &input.current_password),
            (Some(_), true) => None,
            (Some(_), false) => Some(FieldError::new(
                "current_password",
                "conflict",
                "give either the current password or a second factor, not both",
            )),
        };
        let org_id = input.org_id.unwrap_or(root_of(tenant.id()).as_uuid());
        let mut checked = self.check(creator, input.name, input.permissions, input.expires_at, now);
        if let Some(problem) = fence_problem(tenant, creator, org_id).await? {
            checked = match checked {
                Ok(_) => Err(Error::validation(vec![problem])),
                Err(error) if error.kind() == ErrorKind::Validation => {
                    let mut problems = error.fields().to_vec();
                    problems.push(problem);
                    Err(Error::validation(problems))
                }
                Err(error) => Err(error),
            };
        }
        let (name, permissions) = match (checked, blank) {
            (Ok(checked), None) => checked,
            (Ok(_), Some(problem)) => return Err(Error::validation(vec![problem])),
            (Err(error), Some(problem)) if error.kind() == ErrorKind::Validation => {
                let mut problems = error.fields().to_vec();
                problems.push(problem);
                return Err(Error::validation(problems));
            }
            (Err(error), _) => return Err(error),
        };
        let user_id = creator.user_id();
        let verified_hash = match &input.second_factor {
            None => {
                let current = &input.current_password;
                Some(self.sessions.check_password(tenant, user_id, current).await?.1)
            }
            Some(Reauth::Totp(code)) => {
                // Lỗi "mã sai" của MFA chỉ ô `code`; ở đây ô tên là `totp_code`.
                self.mfa.reauthenticate(tenant, user_id, code).await.map_err(|error| {
                    if error.kind() == ErrorKind::Validation {
                        let fields = error
                            .fields()
                            .iter()
                            .map(|field| {
                                let mut field = field.clone();
                                field.field = "totp_code".to_owned();
                                field
                            })
                            .collect();
                        Error::validation(fields)
                    } else {
                        error
                    }
                })?;
                None
            }
            Some(Reauth::Passkey { ceremony_id, credential }) => {
                self.passkeys.reauthenticate(tenant, user_id, *ceremony_id, credential).await?;
                None
            }
            Some(Reauth::Token(token)) => {
                crate::reauth::consume(tenant, user_id, token).await?;
                None
            }
        };
        let existing = api_key::Entity::find()
            .filter(api_key::Column::UserId.eq(user_id))
            .scoped(tenant)
            .count(tenant.database().read())
            .await?;
        if existing >= MAX_KEYS_PER_USER {
            return Err(Error::conflict(
                "auth.api_key_limit",
                "You already have {limit} API keys. Revoke keys you no longer use first.",
            )
            .with_param("limit", MAX_KEYS_PER_USER));
        }

        let random = token::random_token(KEY_BYTES)?;
        let secret = format!("{API_KEY_PREFIX}{}_{random}", tenant_hex(tenant.id()));
        // Mọi key của một khách cùng phần đầu (ID khách): phần nhận diện là đầu phần ngẫu nhiên.
        let start = random.get(..PREFIX_RANDOM_CHARS).unwrap_or_default();
        let prefix = format!("{API_KEY_PREFIX}…{start}");
        let id = Uuid::now_v7();
        let transaction = tenant.database().begin_write().await?;
        // Mật khẩu vừa kiểm phải còn là mật khẩu hiện tại: đặt lại mật khẩu xen giữa thì không
        // tạo key (D-156).
        let verified = verified_hash.as_deref();
        if !crate::session::lock_grant(&transaction, tenant, user_id, verified).await? {
            transaction.rollback().await?;
            return Err(crate::user::invalid_credentials());
        }
        api_key::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            user_id: Set(user_id),
            name: Set(name.clone()),
            prefix: Set(prefix.clone()),
            key_hash: Set(token::sha256(&secret)),
            expires_at: Set(input.expires_at),
            org_id: Set(Some(org_id)),
            last_used_at: Set(None),
            created_at: Set(now),
        }
        .insert(&transaction)
        .await?;
        for permission in &permissions {
            api_key_permission::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                api_key_id: Set(id),
                permission: Set(permission.clone()),
            }
            .insert(&transaction)
            .await?;
        }
        let after = json!({
            "user_id": user_id,
            "name": name,
            "prefix": prefix,
            "permissions": permissions,
            "org_id": org_id,
            "expires_at": input.expires_at,
        });
        let event = events::ApiKeyCreated { before: None, after: Some(after) };
        self.outbox
            .record(&transaction, tenant, Actor::from(creator), key_subject(id), &event)
            .await?;
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, api_key_id = %id, "API key created");
        let key = ApiKey {
            id,
            user_id,
            name,
            prefix,
            permissions,
            org_id,
            expires_at: input.expires_at,
            last_used_at: None,
            created_at: now,
        };
        Ok(CreatedApiKey { key, secret })
    }

    /// Một trang key của người dùng `user_id`, mới tạo trước (mặc định).
    pub(crate) async fn list(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<ApiKey>, Error> {
        let connection = tenant.database().read();
        let select =
            api_key::Entity::find().filter(api_key::Column::UserId.eq(user_id)).scoped(tenant);
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
        page: &PageRequest,
    ) -> Result<Page<ApiKey>, Error> {
        caller.require(&crate::permissions::API_KEYS_MANAGE)?;
        let connection = tenant.database().read();
        let mut select = api_key::Entity::find().scoped(tenant);
        if let Some(owner) = owner {
            select = select.filter(api_key::Column::UserId.eq(owner));
        }
        let page = fetch_page(select, api_key::Column::Id, |row| row.id, page, connection).await?;
        let mut permissions =
            permissions_of(connection, tenant, page.data.iter().map(|row| row.id)).await?;
        Ok(page.map(|row| {
            let granted = permissions.remove(&row.id).unwrap_or_default();
            to_api_key(row, granted)
        }))
    }

    /// Thu hồi key `id` của bất kỳ ai. Cần `auth.api_keys.manage`; event ghi người thu hồi.
    pub(crate) async fn revoke_any(
        &self,
        tenant: &TenantContext,
        caller: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        caller.require(&crate::permissions::API_KEYS_MANAGE)?;
        self.revoke_where(tenant, caller, id, None).await
    }

    /// Thu hồi (xoá) key `id` của bên gọi. Key thôi dùng được ngay.
    pub(crate) async fn revoke(
        &self,
        tenant: &TenantContext,
        caller: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        self.revoke_where(tenant, caller, id, Some(caller.user_id())).await
    }

    /// Thu hồi key `id`; `owner`: chỉ khi key là của người này.
    async fn revoke_where(
        &self,
        tenant: &TenantContext,
        caller: &Principal,
        id: Uuid,
        owner: Option<Uuid>,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let mut find = api_key::Entity::find_by_id(id).scoped(tenant);
        if let Some(owner) = owner {
            find = find.filter(api_key::Column::UserId.eq(owner));
        }
        let row = find.one(&transaction).await?.ok_or_else(|| {
            Error::not_found("auth.api_key_not_found", "You have no API key with id `{id}`.")
                .with_param("id", id)
        })?;
        let user_id = row.user_id;
        api_key::Entity::delete_by_id(id).scoped(tenant).exec(&transaction).await?;
        let before = json!({"user_id": user_id, "name": row.name, "prefix": row.prefix});
        let event = events::ApiKeyRevoked { before: Some(before), after: None };
        self.outbox
            .record(&transaction, tenant, Actor::from(caller), key_subject(id), &event)
            .await?;
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, api_key_id = %id, "API key revoked");
        Ok(())
    }

    /// Bên gọi của một request mang API key `secret`, tra ở khách `tenant` — khách ghi trong key
    /// ([`tenant_of`]). Bản băm là của cả key, nên ghép ID khách khác vào phần ngẫu nhiên của key
    /// này thì không khớp dòng nào.
    pub(crate) async fn authenticate(
        &self,
        tenant: &TenantContext,
        secret: &str,
    ) -> Result<Principal, Error> {
        let now = now();
        let row = api_key::Entity::find()
            .filter(api_key::Column::KeyHash.eq(token::sha256(secret)))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(invalid_api_key)?;
        if row.expires_at.is_some_and(|expires_at| expires_at <= now) {
            return Err(Error::unauthenticated(
                "auth.api_key_expired",
                "This API key has expired. Create a new one.",
            ));
        }
        let (user, owner) =
            self.roles.user_principal(tenant, row.user_id).await?.ok_or_else(invalid_api_key)?;
        if !user.is_active() {
            return Err(crate::user::user_disabled());
        }
        let granted: Vec<String> = api_key_permission::Entity::find()
            .filter(api_key_permission::Column::ApiKeyId.eq(row.id))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?
            .into_iter()
            .map(|row| row.permission)
            .collect();
        // Quyền của key = permission của key ∩ quyền hiện tại của chủ key, giữ nguyên nút và độ
        // sâu của chủ key; key gắn nút thì thêm hàng rào ở nút đó (bước 34).
        let mut principal = owner.only(granted.iter().map(String::as_str)).via_api_key(row.id);
        if let Some(org) = row.org_id {
            principal = principal.with_fence(OrgId::from_uuid(org));
        }
        self.touch(tenant, &row, now).await;
        Ok(principal)
    }

    /// Ghi lần dùng gần nhất, nhiều nhất một lần mỗi phút. Hỏng thì chỉ ghi log: không
    /// từ chối một request hợp lệ vì một dòng thống kê.
    async fn touch(&self, tenant: &TenantContext, row: &api_key::Model, now: DateTime<Utc>) {
        let stale = now - chrono::Duration::seconds(LAST_USED_RESOLUTION_SECONDS);
        if row.last_used_at.is_some_and(|last| last > stale) {
            return;
        }
        // Điều kiện lặp lại trong câu UPDATE: nhiều request đồng thời chỉ một bên ghi.
        let result = api_key::Entity::update_many()
            .col_expr(api_key::Column::LastUsedAt, Expr::value(Some(now)))
            .filter(api_key::Column::Id.eq(row.id))
            .filter(
                Condition::any()
                    .add(api_key::Column::LastUsedAt.is_null())
                    .add(api_key::Column::LastUsedAt.lte(stale)),
            )
            .scoped(tenant)
            .exec(tenant.database().write())
            .await;
        if let Err(error) = result {
            tracing::warn!(api_key_id = %row.id, error = %error, "cannot record API key use");
        }
    }

    /// Kiểm dữ liệu vào; báo mọi trường sai cùng lúc.
    fn check(
        &self,
        creator: &Principal,
        name: String,
        permissions: Vec<String>,
        expires_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<(String, Vec<String>), Error> {
        let name = name.trim().to_owned();
        let mut problems = Vec::new();
        if !(1..=NAME_MAX_LENGTH).contains(&name.chars().count()) {
            problems.push(
                FieldError::new(
                    "name",
                    "length",
                    "must be between 1 and {name_max_length} characters long",
                )
                .with_param("name_max_length", NAME_MAX_LENGTH),
            );
        }
        if !(1..=MAX_PERMISSIONS).contains(&permissions.len()) {
            problems.push(
                FieldError::new(
                    "permissions",
                    "length",
                    "must list between 1 and {max_permissions} permissions",
                )
                .with_param("max_permissions", MAX_PERMISSIONS),
            );
        }
        for (index, permission) in permissions.iter().enumerate() {
            let field = format!("permissions[{index}]");
            if !self.roles.catalog().contains(permission) {
                problems.push(FieldError::new(
                    field,
                    "unknown",
                    "is not a permission of this system",
                ));
            } else if !creator.allows(permission) {
                problems.push(FieldError::new(
                    field,
                    "not_granted",
                    "you do not have this permission, so a key of yours cannot have it",
                ));
            }
        }
        if expires_at.is_some_and(|expires_at| expires_at <= now) {
            problems.push(FieldError::new("expires_at", "past", "must be in the future"));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let permissions: BTreeSet<String> = permissions.into_iter().collect();
        Ok((name, permissions.into_iter().collect()))
    }
}

/// Permission của từng key trong `key_ids`, một câu truy vấn cho cả danh sách.
async fn permissions_of<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    key_ids: impl IntoIterator<Item = Uuid>,
) -> Result<HashMap<Uuid, Vec<String>>, Error> {
    let rows = api_key_permission::Entity::find()
        .filter(api_key_permission::Column::ApiKeyId.is_in(key_ids))
        .scoped(tenant)
        .order_by_asc(api_key_permission::Column::Permission)
        .all(connection)
        .await?;
    let mut by_key: HashMap<Uuid, Vec<String>> = HashMap::new();
    for row in rows {
        by_key.entry(row.api_key_id).or_default().push(row.permission);
    }
    Ok(by_key)
}

fn to_api_key(row: api_key::Model, permissions: Vec<String>) -> ApiKey {
    ApiKey {
        id: row.id,
        user_id: row.user_id,
        name: row.name,
        prefix: row.prefix,
        permissions,
        org_id: row.org_id.unwrap_or(row.tenant_id),
        expires_at: row.expires_at,
        last_used_at: row.last_used_at,
        created_at: row.created_at,
    }
}

/// Chủ thể (subject) của event về API key `id`.
fn key_subject(id: Uuid) -> Option<String> {
    Some(format!("auth.api_key:{id}"))
}

/// ID khách dạng 32 chữ hex thường, như trong key.
fn tenant_hex(tenant: TenantId) -> Simple {
    tenant.as_uuid().simple()
}

/// Khách ghi trong chuỗi `token` nếu nó là API key: `Ok(None)` khi không phải API key (vd access
/// token); key cũ `axk_…` thuộc khách mặc định; key `axk1_…` sai dạng → `auth.invalid_api_key`.
pub(crate) fn tenant_of(token: &str) -> Result<Option<TenantId>, Error> {
    if let Some(rest) = token.strip_prefix(API_KEY_PREFIX) {
        // Độ dài cố định: 32 chữ hex, `_`, rồi phần ngẫu nhiên (không rỗng).
        let hex = rest.get(..32).ok_or_else(invalid_api_key)?;
        let random = rest.get(32..).and_then(|rest| rest.strip_prefix('_'));
        if random.is_none_or(str::is_empty)
            || !hex.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(invalid_api_key());
        }
        let id = Uuid::try_parse(hex).map_err(|_| invalid_api_key())?;
        return Ok(Some(TenantId::from_uuid(id)));
    }
    Ok(token.starts_with(LEGACY_API_KEY_PREFIX).then_some(DEFAULT_TENANT))
}

pub(crate) fn invalid_api_key() -> Error {
    Error::unauthenticated(
        "auth.invalid_api_key",
        "The API key is unknown or was revoked. Check that the whole key was sent, or create \
         a new one.",
    )
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

/// Thu hồi mọi API key của người dùng `user_id` trong `connection` (transaction của bên gọi, vd
/// đặt lại mật khẩu). Mỗi key một event. Trả số key đã thu hồi.
pub(crate) async fn revoke_all_in<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    outbox: &Outbox,
    actor: Actor,
    user_id: Uuid,
) -> Result<u64, Error> {
    let rows = api_key::Entity::find()
        .filter(api_key::Column::UserId.eq(user_id))
        .scoped(tenant)
        .all(connection)
        .await?;
    for row in &rows {
        api_key::Entity::delete_by_id(row.id).scoped(tenant).exec(connection).await?;
        let before = json!({"user_id": user_id, "name": row.name, "prefix": row.prefix});
        let event = events::ApiKeyRevoked { before: Some(before), after: None };
        outbox.record(connection, tenant, actor, key_subject(row.id), &event).await?;
    }
    Ok(rows.len() as u64)
}

/// Lỗi của ô `org_id` khi tạo key gắn nút: nút phải có trong cây của khách và người tạo phải làm
/// việc ở đó (một grant bất kỳ phủ nút). Nút gốc luôn được.
async fn fence_problem(
    tenant: &TenantContext,
    creator: &Principal,
    org_id: Uuid,
) -> Result<Option<FieldError>, Error> {
    let org = OrgId::from_uuid(org_id);
    if org == root_of(tenant.id()) {
        return Ok(None);
    }
    let read = tenant.database().read();
    if OrgTree::ancestors(read, tenant.id(), org).await?.is_empty() {
        return Ok(Some(FieldError::new(
            "org_id",
            "unknown",
            "is not an organization unit of this tenant",
        )));
    }
    if !creator.reaches(read, org).await? {
        return Ok(Some(FieldError::new(
            "org_id",
            "out_of_scope",
            "must be a unit you work in (one of your roles covers it)",
        )));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::error::ErrorKind;
    use autoxide_kernel::pagination::Order;
    use autoxide_testkit::TestDatabase;

    use crate::api::{Accounts, NewAccount};
    use crate::role::RoleInput;
    use crate::testing::Services;
    use crate::user::UserChanges;

    /// Khách đọc theo độ dài cố định: phần ngẫu nhiên có `_` vẫn đúng; key cũ thuộc khách mặc
    /// định; access token không phải key; `axk1_` sai dạng là key sai.
    #[test]
    fn doc_khach_trong_key() {
        let id = Uuid::now_v7();
        let hex = id.simple();
        let key = format!("axk1_{hex}_ab_cd-ef");
        assert_eq!(tenant_of(&key).unwrap(), Some(TenantId::from_uuid(id)));
        assert_eq!(tenant_of("axk_abc").unwrap(), Some(DEFAULT_TENANT));
        assert_eq!(tenant_of("eyJhbGciOiJIUzI1NiJ9.e30.sig").unwrap(), None);
        let upper = format!("axk1_{}_x", hex.to_string().to_uppercase());
        let short = format!("axk1_{}_x", &hex.to_string()[..31]);
        for bad in [
            format!("axk1_{hex}_"),
            format!("axk1_{hex}x"),
            format!("axk1_{hex}"),
            upper,
            short,
            "axk1_".to_owned(),
            format!("axk1_{}_x", "g".repeat(32)),
        ] {
            assert_eq!(tenant_of(&bad).unwrap_err().code(), "auth.invalid_api_key", "{bad}");
        }
    }

    /// Người dùng `ada` có role cấp `auth.users.read` và `auth.roles.read`.
    async fn setup(test: &TestDatabase) -> (Services, Principal) {
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let ada = NewAccount {
            email: "ada@example.com".to_owned(),
            name: "Ada".to_owned(),
            password: Some(PASSWORD.to_owned()),
            admin: false,
        };
        let ada = Accounts::create(services.users.as_ref(), &services.tenant, ada).await.unwrap();
        let role = services
            .roles
            .create(
                &tenant,
                Actor::system(),
                RoleInput {
                    requires_mfa: false,
                    grants: Vec::new(),
                    name: "reader".to_owned(),
                    description: String::new(),
                    permissions: vec!["auth.users.read".to_owned(), "auth.roles.read".to_owned()],
                },
            )
            .await
            .unwrap();
        services
            .roles
            .set_user_roles(&tenant, Actor::system(), ada.id, vec![role.id])
            .await
            .unwrap();
        let principal = services.roles.principal(&tenant, ada.id).await.unwrap();
        (services, principal)
    }

    fn new_key(permissions: &[&str]) -> NewApiKey {
        NewApiKey {
            org_id: None,
            name: "agent".to_owned(),
            permissions: permissions.iter().map(|p| (*p).to_owned()).collect(),
            expires_at: None,
            current_password: PASSWORD.to_owned(),
            second_factor: None,
        }
    }

    const PASSWORD: &str = "correct horse battery staple";

    #[tokio::test]
    async fn tao_key_roi_xac_thuc_bang_no() {
        TestDatabase::run(async |test| {
            let (services, ada) = setup(test).await;
            let tenant = services.tenant.clone();

            let created = services
                .api_keys
                .create(&tenant, &ada, new_key(&["auth.users.read"]))
                .await
                .unwrap();

            // `axk1_<ID khách>_<phần ngẫu nhiên>`; phần nhận diện là đầu phần ngẫu nhiên.
            let head = format!("axk1_{}_", tenant.id().as_uuid().simple());
            assert!(created.secret.starts_with(&head), "{}", created.secret);
            assert_eq!(created.secret.len(), head.len() + 43);
            assert_eq!(created.key.prefix, format!("axk1_…{}", &created.secret[head.len()..][..8]));
            assert_eq!(tenant_of(&created.secret).unwrap(), Some(tenant.id()));
            assert_eq!(created.key.permissions, ["auth.users.read"]);
            let principal = services.api_keys.authenticate(&tenant, &created.secret).await.unwrap();
            assert_eq!(principal.user_id(), ada.user_id());
            assert_eq!(principal.api_key_id(), Some(created.key.id));
            // Key chỉ có đúng những gì ghi trên nó, dù chủ key có nhiều hơn.
            assert!(principal.allows("auth.users.read"));
            assert!(!principal.allows("auth.roles.read"));

            let page = services
                .api_keys
                .list(&tenant, ada.user_id(), &PageRequest::first(10, Order::Desc))
                .await
                .unwrap();
            assert_eq!(page.data.len(), 1);
            assert!(page.data[0].last_used_at.is_some());
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn key_khong_vuot_quyen_chu_key() {
        TestDatabase::run(async |test| {
            let (services, ada) = setup(test).await;
            let tenant = services.tenant.clone();
            let past = Utc::now() - chrono::Duration::hours(1);
            let input = NewApiKey {
                org_id: None,
                name: " ".to_owned(),
                permissions: vec![
                    "auth.users.read".to_owned(),
                    "auth.roles.manage".to_owned(),
                    "nope.read".to_owned(),
                ],
                expires_at: Some(past),
                current_password: PASSWORD.to_owned(),
                second_factor: None,
            };

            let error = services.api_keys.create(&tenant, &ada, input).await.unwrap_err();

            let fields: Vec<(&str, &str)> =
                error.fields().iter().map(|f| (f.field.as_str(), f.code.as_str())).collect();
            assert_eq!(
                fields,
                [
                    ("name", "length"),
                    ("permissions[1]", "not_granted"),
                    ("permissions[2]", "unknown"),
                    ("expires_at", "past"),
                ]
            );
            let empty = services.api_keys.create(&tenant, &ada, new_key(&[])).await.unwrap_err();
            assert_eq!(empty.fields()[0].field, "permissions");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn chu_key_mat_quyen_hay_bi_khoa_thi_key_mat_theo() {
        TestDatabase::run(async |test| {
            let (services, ada) = setup(test).await;
            let tenant = services.tenant.clone();
            let created = services
                .api_keys
                .create(&tenant, &ada, new_key(&["auth.users.read"]))
                .await
                .unwrap();

            services
                .roles
                .set_user_roles(&tenant, Actor::system(), ada.user_id(), vec![])
                .await
                .unwrap();
            let after_role =
                services.api_keys.authenticate(&tenant, &created.secret).await.unwrap();
            assert!(!after_role.allows("auth.users.read"));

            let disable = UserChanges { name: None, disabled: Some(true), ..Default::default() };
            services
                .users
                .update(&tenant, Actor::system(), ada.user_id(), disable, None)
                .await
                .unwrap();
            let error = services.api_keys.authenticate(&tenant, &created.secret).await.unwrap_err();
            assert_eq!(error.code(), "auth.user_disabled");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn thu_hoi_va_het_han() {
        TestDatabase::run(async |test| {
            let (services, ada) = setup(test).await;
            let tenant = services.tenant.clone();
            let keys = &services.api_keys;
            let revoked = keys.create(&tenant, &ada, new_key(&["auth.users.read"])).await.unwrap();
            let expiring = keys.create(&tenant, &ada, new_key(&["auth.users.read"])).await.unwrap();

            // Người khác không thu hồi được key của Ada.
            let other = keys
                .revoke(&tenant, &Principal::new(Uuid::now_v7(), []), revoked.key.id)
                .await
                .unwrap_err();
            assert_eq!(other.code(), "auth.api_key_not_found");
            keys.revoke(&tenant, &ada, revoked.key.id).await.unwrap();
            let error = keys.authenticate(&tenant, &revoked.secret).await.unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Unauthenticated);
            assert_eq!(error.code(), "auth.invalid_api_key");

            api_key::Entity::update_many()
                .col_expr(api_key::Column::ExpiresAt, Expr::value(Some(now())))
                .filter(api_key::Column::Id.eq(expiring.key.id))
                .exec(tenant.database().write())
                .await
                .unwrap();
            let error = keys.authenticate(&tenant, &expiring.secret).await.unwrap_err();
            assert_eq!(error.code(), "auth.api_key_expired");
            assert_eq!(
                keys.authenticate(&tenant, "axk_nope").await.unwrap_err().code(),
                "auth.invalid_api_key"
            );
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn api_key_khong_tao_duoc_api_key() {
        TestDatabase::run(async |test| {
            let (services, ada) = setup(test).await;
            let tenant = services.tenant.clone();
            let created = services
                .api_keys
                .create(&tenant, &ada, new_key(&["auth.users.read"]))
                .await
                .unwrap();
            let via_key = services.api_keys.authenticate(&tenant, &created.secret).await.unwrap();

            let error = services
                .api_keys
                .create(&tenant, &via_key, new_key(&["auth.users.read"]))
                .await
                .unwrap_err();

            assert_eq!(error.kind(), ErrorKind::Forbidden);
            assert_eq!(error.code(), "auth.api_key_cannot_create_keys");
        })
        .await
        .unwrap();
    }

    /// Nhiều request dùng cùng một key cùng lúc (mỗi request có thể ghi lần dùng gần
    /// nhất): mọi request đều được nhận — trên cả hai database.
    #[tokio::test]
    async fn dung_key_dong_thoi() {
        TestDatabase::run(async |test| {
            let (services, ada) = setup(test).await;
            let tenant = services.tenant.clone();
            let created = services
                .api_keys
                .create(&tenant, &ada, new_key(&["auth.users.read"]))
                .await
                .unwrap();

            let attempts: Vec<_> = (0..8)
                .map(|_| {
                    let keys = services.api_keys.clone();
                    let secret = created.secret.clone();
                    let tenant = tenant.clone();
                    tokio::spawn(async move { keys.authenticate(&tenant, &secret).await })
                })
                .collect();
            for attempt in attempts {
                assert!(attempt.await.unwrap().unwrap().allows("auth.users.read"));
            }
        })
        .await
        .unwrap();
    }
}
