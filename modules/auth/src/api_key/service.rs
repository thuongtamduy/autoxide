//! Service của API key: tạo, liệt kê, thu hồi, và xác thực request mang API key.
//!
//! Quyền thật của một key = permission ghi trên key ∩ permission HIỆN TẠI của chủ
//! key. Chủ key mất một quyền (đổi role) thì key mất theo ngay; chủ key bị khoá thì
//! key không dùng được.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::database::Database;
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
use crate::user::UserService;
use autoxide_kernel::event::{Actor, Outbox};
use serde_json::json;

/// Mọi API key bắt đầu bằng chuỗi này: nhìn là biết đây là API key (không phải access
/// token), và công cụ quét secret nhận ra được khi key lỡ bị commit.
pub(crate) const API_KEY_PREFIX: &str = "axk_";

/// Số byte ngẫu nhiên của một key: 256 bit.
const KEY_BYTES: usize = 32;
/// Số ký tự đầu của key được lưu và hiện ra để nhận diện (`axk_` + 8 ký tự).
const PREFIX_LENGTH: usize = 12;
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
    pub name: String,
    /// Vài ký tự đầu của key, để nhận diện.
    pub prefix: String,
    /// Theo thứ tự tên.
    pub permissions: Vec<String>,
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
    /// `None`: không hết hạn.
    pub expires_at: Option<DateTime<Utc>>,
    /// Mật khẩu hiện tại của người tạo: access token bị lộ không đủ để tạo key sống lâu
    /// hơn nó (D-079).
    pub current_password: String,
}

pub(crate) struct ApiKeyService {
    database: Database,
    users: Arc<UserService>,
    roles: Arc<RoleService>,
    sessions: Arc<crate::session::SessionService>,
    outbox: Outbox,
}

impl ApiKeyService {
    pub(crate) fn new(
        database: Database,
        users: Arc<UserService>,
        roles: Arc<RoleService>,
        sessions: Arc<crate::session::SessionService>,
        outbox: Outbox,
    ) -> Self {
        ApiKeyService { database, users, roles, sessions, outbox }
    }

    /// Tạo key cho `creator`, với permission nằm trong số permission `creator` đang có.
    pub(crate) async fn create(
        &self,
        creator: &Principal,
        input: NewApiKey,
    ) -> Result<CreatedApiKey, Error> {
        if creator.api_key_id().is_some() {
            return Err(Error::forbidden(
                "auth.api_key_cannot_create_keys",
                "An API key cannot create API keys: a leaked key must not be able to make \
                 more. Sign in with a password to create keys.",
            ));
        }
        let now = now();
        // Kiểm đầu vào (gom mọi lỗi, kể cả thiếu mật khẩu) TRƯỚC khi kiểm mật khẩu.
        let blank = FieldError::blank("current_password", &input.current_password);
        let checked = self.check(creator, input.name, input.permissions, input.expires_at, now);
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
        self.sessions.check_password(creator.user_id(), &input.current_password).await?;
        let user_id = creator.user_id();
        let existing = api_key::Entity::find()
            .filter(api_key::Column::UserId.eq(user_id))
            .count(self.database.read())
            .await?;
        if existing >= MAX_KEYS_PER_USER {
            return Err(Error::conflict(
                "auth.api_key_limit",
                "You already have {limit} API keys. Revoke keys you no longer use first.",
            )
            .with_param("limit", MAX_KEYS_PER_USER));
        }

        let secret = format!("{API_KEY_PREFIX}{}", token::random_token(KEY_BYTES)?);
        let prefix: String = secret.chars().take(PREFIX_LENGTH).collect();
        let id = Uuid::now_v7();
        let transaction = self.database.begin_write().await?;
        api_key::ActiveModel {
            id: Set(id),
            user_id: Set(user_id),
            name: Set(name.clone()),
            prefix: Set(prefix.clone()),
            key_hash: Set(token::sha256(&secret)),
            expires_at: Set(input.expires_at),
            last_used_at: Set(None),
            created_at: Set(now),
        }
        .insert(&transaction)
        .await?;
        for permission in &permissions {
            api_key_permission::ActiveModel {
                id: Set(Uuid::now_v7()),
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
            "expires_at": input.expires_at,
        });
        let event = events::ApiKeyCreated { before: None, after: Some(after) };
        self.outbox.record(&transaction, Actor::from(creator), key_subject(id), &event).await?;
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, api_key_id = %id, "API key created");
        let key = ApiKey {
            id,
            name,
            prefix,
            permissions,
            expires_at: input.expires_at,
            last_used_at: None,
            created_at: now,
        };
        Ok(CreatedApiKey { key, secret })
    }

    /// Một trang key của người dùng `user_id`, mới tạo trước (mặc định).
    pub(crate) async fn list(
        &self,
        user_id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<ApiKey>, Error> {
        let connection = self.database.read();
        let select = api_key::Entity::find().filter(api_key::Column::UserId.eq(user_id));
        let page = fetch_page(select, api_key::Column::Id, |row| row.id, page, connection).await?;
        let mut permissions =
            permissions_of(connection, page.data.iter().map(|row| row.id)).await?;
        Ok(page.map(|row| {
            let granted = permissions.remove(&row.id).unwrap_or_default();
            to_api_key(row, granted)
        }))
    }

    /// Thu hồi (xoá) key `id` của bên gọi. Key thôi dùng được ngay.
    pub(crate) async fn revoke(&self, caller: &Principal, id: Uuid) -> Result<(), Error> {
        let user_id = caller.user_id();
        let transaction = self.database.begin_write().await?;
        let row = api_key::Entity::find_by_id(id)
            .filter(api_key::Column::UserId.eq(user_id))
            .one(&transaction)
            .await?
            .ok_or_else(|| {
                Error::not_found(
                    "auth.api_key_not_found",
                    "You have no API key with id `{id}`. List your keys with \
                     `GET /api/v1/auth/api-keys`.",
                )
                .with_param("id", id)
            })?;
        api_key::Entity::delete_by_id(id).exec(&transaction).await?;
        let before = json!({"user_id": user_id, "name": row.name, "prefix": row.prefix});
        let event = events::ApiKeyRevoked { before: Some(before), after: None };
        self.outbox.record(&transaction, Actor::from(caller), key_subject(id), &event).await?;
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, api_key_id = %id, "API key revoked");
        Ok(())
    }

    /// Thu hồi mọi API key của người dùng `user_id` (vd khi chủ tài khoản lấy lại tài khoản
    /// bằng cách đặt lại mật khẩu). Trả số key đã thu hồi.
    pub(crate) async fn revoke_all(&self, actor: Actor, user_id: Uuid) -> Result<u64, Error> {
        let transaction = self.database.begin_write().await?;
        let rows = api_key::Entity::find()
            .filter(api_key::Column::UserId.eq(user_id))
            .all(&transaction)
            .await?;
        for row in &rows {
            api_key::Entity::delete_by_id(row.id).exec(&transaction).await?;
            let before = json!({"user_id": user_id, "name": row.name, "prefix": row.prefix});
            let event = events::ApiKeyRevoked { before: Some(before), after: None };
            self.outbox.record(&transaction, actor, key_subject(row.id), &event).await?;
        }
        transaction.commit().await?;
        Ok(rows.len() as u64)
    }

    /// Bên gọi của một request mang API key `secret`.
    pub(crate) async fn authenticate(&self, secret: &str) -> Result<Principal, Error> {
        let now = now();
        let row = api_key::Entity::find()
            .filter(api_key::Column::KeyHash.eq(token::sha256(secret)))
            .one(self.database.read())
            .await?
            .ok_or_else(invalid_api_key)?;
        if row.expires_at.is_some_and(|expires_at| expires_at <= now) {
            return Err(Error::unauthenticated(
                "auth.api_key_expired",
                "This API key has expired. Create a new one with `POST /api/v1/auth/api-keys`.",
            ));
        }
        let user = self.users.get(row.user_id).await?.ok_or_else(invalid_api_key)?;
        if !user.is_active() {
            return Err(crate::user::user_disabled());
        }
        let owner = self.roles.principal(user.id).await?;
        let granted = api_key_permission::Entity::find()
            .filter(api_key_permission::Column::ApiKeyId.eq(row.id))
            .all(self.database.read())
            .await?
            .into_iter()
            .map(|row| row.permission)
            .filter(|permission| owner.allows(permission));
        let principal = Principal::new(user.id, granted).via_api_key(row.id);
        self.touch(&row, now).await;
        Ok(principal)
    }

    /// Ghi lần dùng gần nhất, nhiều nhất một lần mỗi phút. Hỏng thì chỉ ghi log: không
    /// từ chối một request hợp lệ vì một dòng thống kê.
    async fn touch(&self, row: &api_key::Model, now: DateTime<Utc>) {
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
            .exec(self.database.write())
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
            problems.push(FieldError::new(
                "name",
                "length",
                format!("must be between 1 and {NAME_MAX_LENGTH} characters long"),
            ));
        }
        if !(1..=MAX_PERMISSIONS).contains(&permissions.len()) {
            problems.push(FieldError::new(
                "permissions",
                "length",
                format!("must list between 1 and {MAX_PERMISSIONS} permissions"),
            ));
        }
        for (index, permission) in permissions.iter().enumerate() {
            let field = format!("permissions[{index}]");
            if !self.roles.catalog().contains(permission) {
                problems.push(FieldError::new(
                    field,
                    "unknown",
                    "is not a permission of this system; see `GET /api/v1/auth/permissions`",
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
    key_ids: impl IntoIterator<Item = Uuid>,
) -> Result<HashMap<Uuid, Vec<String>>, Error> {
    let rows = api_key_permission::Entity::find()
        .filter(api_key_permission::Column::ApiKeyId.is_in(key_ids))
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
        name: row.name,
        prefix: row.prefix,
        permissions,
        expires_at: row.expires_at,
        last_used_at: row.last_used_at,
        created_at: row.created_at,
    }
}

/// Chủ thể (subject) của event về API key `id`.
fn key_subject(id: Uuid) -> Option<String> {
    Some(format!("auth.api_key:{id}"))
}

fn invalid_api_key() -> Error {
    Error::unauthenticated(
        "auth.invalid_api_key",
        "The API key is unknown or was revoked. Check that the whole key was sent, or create \
         a new one.",
    )
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
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

    /// Người dùng `ada` có role cấp `auth.users.read` và `auth.roles.read`.
    async fn setup(test: &TestDatabase) -> (Services, Principal) {
        let services = crate::testing::services(test).await;
        let ada = NewAccount {
            email: "ada@example.com".to_owned(),
            name: "Ada".to_owned(),
            password: Some(PASSWORD.to_owned()),
            admin: false,
        };
        let ada = Accounts::create(services.users.as_ref(), ada).await.unwrap();
        let role = services
            .roles
            .create(
                Actor::system(),
                RoleInput {
                    name: "reader".to_owned(),
                    description: String::new(),
                    permissions: vec!["auth.users.read".to_owned(), "auth.roles.read".to_owned()],
                },
            )
            .await
            .unwrap();
        services.roles.set_user_roles(Actor::system(), ada.id, vec![role.id]).await.unwrap();
        let principal = services.roles.principal(ada.id).await.unwrap();
        (services, principal)
    }

    fn new_key(permissions: &[&str]) -> NewApiKey {
        NewApiKey {
            name: "agent".to_owned(),
            permissions: permissions.iter().map(|p| (*p).to_owned()).collect(),
            expires_at: None,
            current_password: PASSWORD.to_owned(),
        }
    }

    const PASSWORD: &str = "correct horse battery staple";

    #[tokio::test]
    async fn tao_key_roi_xac_thuc_bang_no() {
        TestDatabase::run(async |test| {
            let (services, ada) = setup(test).await;

            let created =
                services.api_keys.create(&ada, new_key(&["auth.users.read"])).await.unwrap();

            assert!(created.secret.starts_with("axk_"));
            assert_eq!(created.secret.len(), 4 + 43);
            assert!(created.secret.starts_with(&created.key.prefix));
            assert_eq!(created.key.permissions, ["auth.users.read"]);
            let principal = services.api_keys.authenticate(&created.secret).await.unwrap();
            assert_eq!(principal.user_id(), ada.user_id());
            assert_eq!(principal.api_key_id(), Some(created.key.id));
            // Key chỉ có đúng những gì ghi trên nó, dù chủ key có nhiều hơn.
            assert!(principal.allows("auth.users.read"));
            assert!(!principal.allows("auth.roles.read"));

            let page = services
                .api_keys
                .list(ada.user_id(), &PageRequest::first(10, Order::Desc))
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
            let past = Utc::now() - chrono::Duration::hours(1);
            let input = NewApiKey {
                name: " ".to_owned(),
                permissions: vec![
                    "auth.users.read".to_owned(),
                    "auth.roles.manage".to_owned(),
                    "nope.read".to_owned(),
                ],
                expires_at: Some(past),
                current_password: PASSWORD.to_owned(),
            };

            let error = services.api_keys.create(&ada, input).await.unwrap_err();

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
            let empty = services.api_keys.create(&ada, new_key(&[])).await.unwrap_err();
            assert_eq!(empty.fields()[0].field, "permissions");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn chu_key_mat_quyen_hay_bi_khoa_thi_key_mat_theo() {
        TestDatabase::run(async |test| {
            let (services, ada) = setup(test).await;
            let created =
                services.api_keys.create(&ada, new_key(&["auth.users.read"])).await.unwrap();

            services.roles.set_user_roles(Actor::system(), ada.user_id(), vec![]).await.unwrap();
            let after_role = services.api_keys.authenticate(&created.secret).await.unwrap();
            assert!(!after_role.allows("auth.users.read"));

            let disable = UserChanges { name: None, disabled: Some(true) };
            services.users.update(Actor::system(), ada.user_id(), disable, None).await.unwrap();
            let error = services.api_keys.authenticate(&created.secret).await.unwrap_err();
            assert_eq!(error.code(), "auth.user_disabled");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn thu_hoi_va_het_han() {
        TestDatabase::run(async |test| {
            let (services, ada) = setup(test).await;
            let keys = &services.api_keys;
            let revoked = keys.create(&ada, new_key(&["auth.users.read"])).await.unwrap();
            let expiring = keys.create(&ada, new_key(&["auth.users.read"])).await.unwrap();

            // Người khác không thu hồi được key của Ada.
            let other =
                keys.revoke(&Principal::new(Uuid::now_v7(), []), revoked.key.id).await.unwrap_err();
            assert_eq!(other.code(), "auth.api_key_not_found");
            keys.revoke(&ada, revoked.key.id).await.unwrap();
            let error = keys.authenticate(&revoked.secret).await.unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Unauthenticated);
            assert_eq!(error.code(), "auth.invalid_api_key");

            api_key::Entity::update_many()
                .col_expr(api_key::Column::ExpiresAt, Expr::value(Some(now())))
                .filter(api_key::Column::Id.eq(expiring.key.id))
                .exec(keys.database.write())
                .await
                .unwrap();
            let error = keys.authenticate(&expiring.secret).await.unwrap_err();
            assert_eq!(error.code(), "auth.api_key_expired");
            assert_eq!(
                keys.authenticate("axk_nope").await.unwrap_err().code(),
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
            let created =
                services.api_keys.create(&ada, new_key(&["auth.users.read"])).await.unwrap();
            let via_key = services.api_keys.authenticate(&created.secret).await.unwrap();

            let error = services
                .api_keys
                .create(&via_key, new_key(&["auth.users.read"]))
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
            let created =
                services.api_keys.create(&ada, new_key(&["auth.users.read"])).await.unwrap();

            let attempts: Vec<_> = (0..8)
                .map(|_| {
                    let keys = services.api_keys.clone();
                    let secret = created.secret.clone();
                    tokio::spawn(async move { keys.authenticate(&secret).await })
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
