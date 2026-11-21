//! Cờ theo người dùng (D-207): giao diện nhớ "đã xem gợi ý này", "đã đọc tới mốc Có gì mới
//! v0.13" theo **tài khoản** chứ không theo trình duyệt — đổi máy không hiện lại gợi ý cũ.
//!
//! - Bảng `kernel_user_flags` của module nội bộ `kernel`: bảng cấp khách hàng, (`tenant_id`,
//!   `user_id`, `key`) duy nhất, giá trị là chuỗi ngắn. Mọi truy vấn lọc theo khách (bước 33).
//! - API dưới `/api/v1/me/flags` (`GET` theo tiền tố, `PUT` ghi đè, `DELETE`), chỉ cờ của chính
//!   người gọi; không endpoint nào đọc cờ của người khác. Chỉ cần đăng nhập, không cần permission.
//! - Mỗi người tối đa `AUTOXIDE_USER_FLAGS_MAX` cờ. Hai request tạo cờ MỚI cùng lúc có thể cùng
//!   lọt qua lần đếm — vượt mức tối đa vài cờ, không sai dữ liệu; chấp nhận vì đây là hạn mức chống
//!   lạm dụng, không phải số tiền.
//! - Cờ là dữ liệu cá nhân: có trong bản xuất dữ liệu (`PersonalData`), bị xoá khi tài khoản bị
//!   xoá (`privacy::USER_DELETED`).

use crate::time::Timestamp;
use std::sync::{Arc, OnceLock};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use sea_orm::sea_query::{LikeExpr, OnConflict};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, Set};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::access::Principal;
use crate::config::{ConfigReader, Invalid, Setting, parse_in_range};
use crate::error::{Error, FieldError};
use crate::event::{Delivered, EventHandler};
use crate::migration::prelude::*;
use crate::privacy::{PersonalData, deleted_user};
use crate::response::{Data, Problem};
use crate::tenancy::{ScopeExt, TenantContext, TenantScoped};

/// Tiền tố đường dẫn (dưới `/api/v1`) của các endpoint "của tôi" do kernel phục vụ.
pub(crate) const ME: &str = "me";

/// Độ dài tối đa của khoá và của giá trị.
const KEY_MAX: usize = 100;
const VALUE_MAX: usize = 200;

pub(crate) const MAX_FLAGS: Setting = Setting::new(
    "AUTOXIDE_USER_FLAGS_MAX",
    "Most interface flags one user may keep (dismissed hints, last seen \"what's new\" \
     version…), 1 to 100000. Past it, new flags are refused until the user deletes some.",
)
.with_default("500");

pub(crate) fn read_setting(reader: &mut ConfigReader<'_>) -> Result<u64, Invalid> {
    reader.get_with(&MAX_FLAGS, |raw| parse_in_range(raw, 1u64, 100_000))
}

mod entity {
    //! Bảng `kernel_user_flags`.
    use crate::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "kernel_user_flags")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub user_id: Uuid,
        pub key: String,
        pub value: String,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

use entity::{Column, Entity};

impl TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}

/// A flag the interface keeps for the signed-in user, e.g. a dismissed hint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = UserFlag)]
pub(crate) struct Flag {
    /// Lower-case letters, digits, `.`, `_` and `-`, up to 100 characters; group flags with a
    /// prefix, e.g. `hint.saved_views`.
    #[schema(example = "hint.saved_views")]
    pub key: String,
    /// Up to 200 characters, e.g. `dismissed` or the last version whose news were seen.
    #[schema(example = "dismissed")]
    pub value: String,
    pub updated_at: Timestamp,
}

impl From<entity::Model> for Flag {
    fn from(row: entity::Model) -> Self {
        Flag { key: row.key, value: row.value, updated_at: row.updated_at }
    }
}

/// Service cờ theo người dùng.
pub(crate) struct UserFlags {
    max: u64,
}

/// Chỗ giữ service, dùng chung giữa module `kernel` (điền lúc `start`) và `Modules::routes` (đọc).
//
// `Arc<OnceLock<…>>`: hai bên giữ cùng một ô, ghi đúng một lần rồi chỉ đọc.
#[derive(Clone, Default)]
pub(crate) struct Handle(Arc<OnceLock<Arc<UserFlags>>>);

impl Handle {
    pub(crate) fn fill(&self, service: Arc<UserFlags>) {
        // Gọi lại (không xảy ra trong chương trình) thì giữ bản đầu.
        let _ = self.0.set(service);
    }

    pub(crate) fn get(&self) -> Option<Arc<UserFlags>> {
        self.0.get().cloned()
    }
}

fn now() -> Timestamp {
    Timestamp::now()
}

/// Khoá đúng dạng: 1–100 ký tự `a-z`, `0-9`, `.`, `_`, `-`.
fn is_key(key: &str) -> bool {
    (1..=KEY_MAX).contains(&key.len())
        && key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}

fn check_key(field: &str, key: &str, problems: &mut Vec<FieldError>) {
    if !is_key(key) {
        problems.push(
            FieldError::new(
                field,
                "invalid",
                "must be 1 to {key_max} characters of lower-case letters, digits, `.`, `_` and `-`",
            )
            .with_param("key_max", KEY_MAX),
        );
    }
}

fn limit_reached(max: u64) -> Error {
    Error::conflict(
        "kernel.flag_limit_reached",
        "You already keep {max} interface flags, the most allowed. Delete flags you no longer \
         need with `DELETE /api/v1/me/flags/{key}`, then try again.",
    )
    .with_param("max", max)
}

impl UserFlags {
    pub(crate) fn new(max: u64) -> Self {
        UserFlags { max }
    }

    /// Cờ của người gọi, theo thứ tự khoá; `prefix` (đúng dạng khoá, có thể rỗng) lọc theo tiền
    /// tố. Tối đa `max` cờ nên trả hết một lần.
    pub(crate) async fn list(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        prefix: Option<&str>,
    ) -> Result<Vec<Flag>, Error> {
        let mut select =
            Entity::find().filter(Column::UserId.eq(principal.user_id())).scoped(tenant);
        if let Some(prefix) = prefix.filter(|prefix| !prefix.is_empty()) {
            let mut problems = Vec::new();
            check_key("prefix", prefix, &mut problems);
            if !problems.is_empty() {
                return Err(Error::validation(problems));
            }
            // `_` là ký tự đại diện của `LIKE`: thoát nó (khoá không có `%` hay `\`).
            let pattern = format!("{}%", prefix.replace('_', "\\_"));
            select = select.filter(Column::Key.like(LikeExpr::new(pattern).escape('\\')));
        }
        let rows = select.order_by_asc(Column::Key).all(tenant.database().read()).await?;
        Ok(rows.into_iter().map(Flag::from).collect())
    }

    /// Ghi cờ `key` của người gọi (ghi đè nếu có): gọi lại bao nhiêu lần cũng ra một dòng.
    pub(crate) async fn put(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        key: &str,
        value: &str,
    ) -> Result<Flag, Error> {
        let mut problems = Vec::new();
        check_key("key", key, &mut problems);
        if value.chars().count() > VALUE_MAX {
            problems.push(
                FieldError::new("value", "length", "must be at most {value_max} characters long")
                    .with_param("value_max", VALUE_MAX),
            );
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let user_id = principal.user_id();
        let read = tenant.database().read();
        let exists = Entity::find()
            .filter(Column::UserId.eq(user_id))
            .filter(Column::Key.eq(key))
            .scoped(tenant)
            .count(read)
            .await?
            > 0;
        if !exists {
            let kept = Entity::find()
                .filter(Column::UserId.eq(user_id))
                .scoped(tenant)
                .count(read)
                .await?;
            if kept >= self.max {
                return Err(limit_reached(self.max));
            }
        }
        let now = now();
        let row = entity::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            user_id: Set(user_id),
            key: Set(key.to_owned()),
            value: Set(value.to_owned()),
            updated_at: Set(now),
        };
        // Một câu lệnh, chạy như nhau trên hai database: hai request cùng khoá cùng lúc ra đúng
        // một dòng (khoá duy nhất `tenant_id`, `user_id`, `key`), bên đến sau ghi đè giá trị.
        Entity::insert(row)
            .on_conflict(
                OnConflict::columns([Column::TenantId, Column::UserId, Column::Key])
                    .update_columns([Column::Value, Column::UpdatedAt])
                    .to_owned(),
            )
            .exec_without_returning(tenant.database().write())
            .await?;
        Ok(Flag { key: key.to_owned(), value: value.to_owned(), updated_at: now })
    }

    /// Xoá cờ `key` của người gọi; không có thì không đổi gì.
    pub(crate) async fn delete(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        key: &str,
    ) -> Result<(), Error> {
        let mut problems = Vec::new();
        check_key("key", key, &mut problems);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        Entity::delete_many()
            .filter(Column::UserId.eq(principal.user_id()))
            .filter(Column::Key.eq(key))
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        Ok(())
    }

    /// Xoá mọi cờ của người dùng `user_id` (tài khoản đã bị xoá). Trả số cờ đã xoá.
    pub(crate) async fn forget(&self, tenant: &TenantContext, user_id: Uuid) -> Result<u64, Error> {
        let deleted = Entity::delete_many()
            .filter(Column::UserId.eq(user_id))
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        Ok(deleted.rows_affected)
    }
}

#[async_trait::async_trait]
impl PersonalData for UserFlags {
    async fn export(&self, tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
        let rows = Entity::find()
            .filter(Column::UserId.eq(user_id))
            .scoped(tenant)
            .order_by_asc(Column::Key)
            .all(tenant.database().read())
            .await?;
        let flags: Vec<Flag> = rows.into_iter().map(Flag::from).collect();
        // View đã lưu (P7) là thiết lập giao diện của người dùng như cờ: xuất cùng chỗ.
        let views = crate::ops::views::export_user(tenant, user_id).await?;
        let pins = crate::ops::pins::export_user(tenant, user_id).await?;
        Ok(json!({ "user_flags": flags, "saved_views": views, "pins": pins }))
    }
}

/// Tài khoản bị xoá: xoá cờ của người đó. Nhận lại event thì không còn gì để xoá.
#[async_trait::async_trait]
impl EventHandler for UserFlags {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        if let Some(user_id) = deleted_user(event) {
            let count = self.forget(&event.tenant, user_id).await?;
            crate::ops::views::forget_user(event.tenant.database().write(), &event.tenant, user_id)
                .await?;
            crate::ops::pins::forget_user(&event.tenant, user_id).await?;
            tracing::info!(user_id = %user_id, count, "interface flags of a deleted user removed");
        }
        Ok(())
    }
}

// --- REST -----------------------------------------------------------------------------------

/// Which flags to list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct FlagsQuery {
    /// Only flags whose key starts with this, e.g. `hint.`.
    #[param(example = "hint.")]
    prefix: Option<String>,
}

/// The value of a flag.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = UserFlagValue)]
pub(crate) struct FlagValue {
    /// Up to 200 characters, e.g. `dismissed`.
    #[schema(example = "dismissed")]
    value: String,
}

/// List my interface flags.
///
/// Requires signing in, no permission: only the caller's own flags, sorted by key. Use them to
/// remember per account (not per browser) which hints were dismissed or which news were seen.
#[utoipa::path(
    get,
    path = "/flags",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_flags",
    security(("bearer" = [])),
    params(FlagsQuery),
    responses(
        (status = 200, description = "The caller's flags, sorted by key (at most `AUTOXIDE_USER_FLAGS_MAX`).", body = Data<Vec<Flag>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "`prefix` is not a valid key prefix; see `errors`.", body = Problem),
    )
)]
async fn list_flags(
    State(service): State<Arc<UserFlags>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<FlagsQuery>,
) -> Result<Data<Vec<Flag>>, Error> {
    Ok(Data { data: service.list(&tenant, &principal, query.prefix.as_deref()).await? })
}

/// Set one of my interface flags.
///
/// Requires signing in, no permission. Creates the flag or replaces its value: sending it again
/// is safe. A user keeps at most `AUTOXIDE_USER_FLAGS_MAX` flags (500 by default).
#[utoipa::path(
    put,
    path = "/flags/{key}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "set_my_flag",
    security(("bearer" = [])),
    params(("key" = String, Path, description = "Lower-case letters, digits, `.`, `_` and `-`, up to 100 characters, e.g. `hint.saved_views`.")),
    request_body = FlagValue,
    responses(
        (status = 200, description = "The flag as stored.", body = Data<Flag>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 409, description = "The caller already keeps the most flags allowed; delete some first. Code `kernel.flag_limit_reached`.", body = Problem),
        (status = 422, description = "The key or the value is invalid; see `errors`.", body = Problem),
    )
)]
async fn set_flag(
    State(service): State<Arc<UserFlags>>,
    tenant: TenantContext,
    principal: Principal,
    Path(key): Path<String>,
    crate::http::ValidJson(input): crate::http::ValidJson<FlagValue>,
) -> Result<Data<Flag>, Error> {
    Ok(Data { data: service.put(&tenant, &principal, &key, &input.value).await? })
}

/// Delete one of my interface flags.
///
/// Requires signing in, no permission. Deleting a flag that does not exist changes nothing.
#[utoipa::path(
    delete,
    path = "/flags/{key}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "delete_my_flag",
    security(("bearer" = [])),
    params(("key" = String, Path, description = "Key of the flag.")),
    responses(
        (status = 204, description = "The flag is gone (or never existed)."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "The key is invalid; see `errors`.", body = Problem),
    )
)]
async fn delete_flag(
    State(service): State<Arc<UserFlags>>,
    tenant: TenantContext,
    principal: Principal,
    Path(key): Path<String>,
) -> Result<StatusCode, Error> {
    service.delete(&tenant, &principal, &key).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Route dưới `/api/v1/me`.
pub(crate) fn routes(service: Arc<UserFlags>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_flags))
        .routes(routes!(set_flag, delete_flag))
        .with_state(service)
}

// --- Migration ------------------------------------------------------------------------------

/// Bảng `kernel_user_flags`.
pub(crate) struct CreateUserFlags;

impl MigrationName for CreateUserFlags {
    fn name(&self) -> &str {
        "m0002_create_user_flags"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateUserFlags {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("kernel_user_flags")
                    .col(schema::uuid("id").primary_key())
                    .col(schema::uuid("user_id"))
                    .col(schema::string_len("key", 100))
                    .col(schema::string_len("value", 200))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("kernel_user_flags_user_key")
                    .table("kernel_user_flags")
                    .col("user_id")
                    .col("key")
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("kernel_user_flags").to_owned()).await
    }
}

/// `kernel_user_flags` theo khách (bước 33): khoá duy nhất `(tenant_id, user_id, key)`.
pub(crate) fn adopt_tenancy() -> Box<dyn MigrationTrait> {
    crate::tenancy::migration::adopt(
        "m0004_tenancy_user_flags",
        vec![
            crate::tenancy::migration::TenantTable::new("kernel_user_flags")
                .unique("kernel_user_flags_user_key", ["tenant_id", "user_id", "key"]),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ConfigSource};
    use crate::database::Database;
    use crate::migration::{Migrator, ModuleMigrations};
    use autoxide_testkit::TestDatabase;

    async fn service(test: &TestDatabase, max: u64) -> (Arc<UserFlags>, TenantContext) {
        let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
        let mut reader = ConfigReader::new(&source);
        let config = Config::read(&mut reader);
        let (config, _) = reader.finish(config).unwrap();
        let database = Database::connect(&config.database).await.unwrap();
        let migrations = vec![
            Box::new(crate::tenancy::schema::CreateTenancy) as Box<dyn MigrationTrait>,
            Box::new(CreateUserFlags),
            adopt_tenancy(),
            Box::new(crate::ops::views::CreateSavedViews),
            Box::new(crate::ops::pins::CreatePins),
        ];
        Migrator::new(vec![ModuleMigrations::new("kernel", migrations)])
            .unwrap()
            .up(&database)
            .await
            .unwrap();
        (Arc::new(UserFlags::new(max)), TenantContext::default_on(database))
    }

    fn user() -> Principal {
        Principal::new(Uuid::now_v7(), Vec::<String>::new())
    }

    fn keys(flags: &[Flag]) -> Vec<&str> {
        flags.iter().map(|flag| flag.key.as_str()).collect()
    }

    #[tokio::test]
    async fn flag_ghi_lai_cung_key_la_ghi_de() {
        TestDatabase::run(async |test| {
            let (flags, tenant) = service(test, 500).await;
            let ada = user();
            flags.put(&tenant, &ada, "whats_new", "v0.12").await.unwrap();
            flags.put(&tenant, &ada, "whats_new", "v0.13").await.unwrap();
            let found = flags.list(&tenant, &ada, None).await.unwrap();
            assert_eq!(found.len(), 1);
            assert_eq!(found[0].value, "v0.13");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn khong_doc_duoc_flag_cua_nguoi_khac() {
        TestDatabase::run(async |test| {
            let (flags, tenant) = service(test, 500).await;
            let (ada, bob) = (user(), user());
            flags.put(&tenant, &ada, "hint.views", "dismissed").await.unwrap();
            flags.put(&tenant, &bob, "hint.pins", "dismissed").await.unwrap();
            assert_eq!(keys(&flags.list(&tenant, &ada, None).await.unwrap()), ["hint.views"]);
            assert_eq!(
                keys(&flags.list(&tenant, &bob, Some("hint.")).await.unwrap()),
                ["hint.pins"]
            );
            // Xoá cùng khoá của người khác không đụng tới cờ của mình.
            flags.delete(&tenant, &bob, "hint.views").await.unwrap();
            assert_eq!(keys(&flags.list(&tenant, &ada, None).await.unwrap()), ["hint.views"]);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn loc_theo_tien_to_khong_coi_gach_duoi_la_ky_tu_dai_dien() {
        TestDatabase::run(async |test| {
            let (flags, tenant) = service(test, 500).await;
            let ada = user();
            for key in ["hint_a", "hintxa", "hint.b", "news"] {
                flags.put(&tenant, &ada, key, "1").await.unwrap();
            }
            assert_eq!(keys(&flags.list(&tenant, &ada, Some("hint_")).await.unwrap()), ["hint_a"]);
            assert_eq!(keys(&flags.list(&tenant, &ada, Some("hint")).await.unwrap()).len(), 3);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn key_sai_dinh_dang_bi_tu_choi() {
        TestDatabase::run(async |test| {
            let (flags, tenant) = service(test, 500).await;
            let ada = user();
            let long = "a".repeat(101);
            for key in ["", "Hint", "hint views", "hint/x", "gợi_ý", long.as_str()] {
                let error = flags.put(&tenant, &ada, key, "x").await.unwrap_err();
                assert_eq!(error.fields()[0].field, "key", "{key}");
            }
            let error = flags.put(&tenant, &ada, "ok", &"x".repeat(201)).await.unwrap_err();
            assert_eq!(error.fields()[0].field, "value");
            assert_eq!(
                flags.list(&tenant, &ada, Some("Bad")).await.unwrap_err().fields()[0].field,
                "prefix"
            );
            assert!(flags.list(&tenant, &ada, None).await.unwrap().is_empty());
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn vuot_han_muc_flag_bao_loi_ro() {
        TestDatabase::run(async |test| {
            let (flags, tenant) = service(test, 2).await;
            let ada = user();
            flags.put(&tenant, &ada, "a", "1").await.unwrap();
            flags.put(&tenant, &ada, "b", "1").await.unwrap();
            let error = flags.put(&tenant, &ada, "c", "1").await.unwrap_err();
            assert_eq!(error.code(), "kernel.flag_limit_reached");
            assert!(error.to_string().contains("DELETE /api/v1/me/flags"), "{error}");
            // Ghi đè cờ đang có vẫn được khi đã đủ mức; xoá bớt thì tạo được cờ mới.
            flags.put(&tenant, &ada, "a", "2").await.unwrap();
            flags.delete(&tenant, &ada, "b").await.unwrap();
            flags.put(&tenant, &ada, "c", "1").await.unwrap();
            // Hạn mức theo từng người.
            flags.put(&tenant, &user(), "a", "1").await.unwrap();
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn xoa_tai_khoan_xoa_flag() {
        TestDatabase::run(async |test| {
            let (flags, tenant) = service(test, 500).await;
            let (ada, bob) = (user(), user());
            flags.put(&tenant, &ada, "hint.views", "dismissed").await.unwrap();
            flags.put(&tenant, &bob, "hint.views", "dismissed").await.unwrap();
            let exported = flags.export(&tenant.clone(), ada.user_id()).await.unwrap();
            assert_eq!(exported["user_flags"][0]["key"], "hint.views");

            let event = Delivered::new(
                tenant.clone(),
                "auth",
                crate::privacy::USER_DELETED,
                json!({"before": {"id": ada.user_id()}}),
            );
            flags.handle(&event).await.unwrap();
            flags.handle(&event).await.unwrap();
            assert!(flags.list(&tenant, &ada, None).await.unwrap().is_empty());
            assert_eq!(
                flags.list(&tenant, &bob, None).await.unwrap().len(),
                1,
                "others keep theirs"
            );
            assert_eq!(
                flags.export(&tenant.clone(), ada.user_id()).await.unwrap()["user_flags"],
                json!([])
            );
        })
        .await
        .unwrap();
    }

    /// Cùng người (cùng `user_id`) và cùng khoá ở hai khách là hai dòng; khách này không thấy, không
    /// sửa, không xoá được cờ của khách kia.
    #[tokio::test]
    async fn flag_cua_khach_nay_khong_lo_sang_khach_khac() {
        TestDatabase::run(async |test| {
            let (flags, ours) = service(test, 500).await;
            let theirs = TenantContext::new(
                crate::tenancy::TenantId::from_uuid(Uuid::now_v7()),
                "other",
                "Other",
                crate::tenancy::TenantStatus::Active,
                crate::tenancy::MAIN_PLACEMENT,
                ours.database().clone(),
            );
            let ada = user();
            flags.put(&ours, &ada, "hint.views", "ours").await.unwrap();
            assert!(flags.list(&theirs, &ada, None).await.unwrap().is_empty());
            flags.put(&theirs, &ada, "hint.views", "theirs").await.unwrap();
            assert_eq!(flags.list(&ours, &ada, None).await.unwrap()[0].value, "ours");
            flags.delete(&theirs, &ada, "hint.views").await.unwrap();
            assert_eq!(flags.list(&ours, &ada, None).await.unwrap().len(), 1);
            assert_eq!(
                flags.export(&theirs, ada.user_id()).await.unwrap()["user_flags"],
                json!([])
            );
            assert_eq!(flags.forget(&theirs, ada.user_id()).await.unwrap(), 0);
        })
        .await
        .unwrap();
    }

    /// Nhiều lệnh `PUT` cùng khoá cùng lúc: đúng một dòng, giá trị là của một trong các lệnh.
    #[tokio::test]
    async fn put_dong_thoi_cung_key_chi_mot_dong() {
        TestDatabase::run(async |test| {
            let (flags, tenant) = service(test, 500).await;
            let ada = user();
            let mut tasks = Vec::new();
            for round in 0..8 {
                let (flags, ada, tenant) = (Arc::clone(&flags), ada.clone(), tenant.clone());
                tasks.push(tokio::spawn(async move {
                    flags.put(&tenant, &ada, "whats_new", &format!("v{round}")).await
                }));
            }
            for task in tasks {
                task.await.unwrap().unwrap();
            }
            let found = flags.list(&tenant, &ada, None).await.unwrap();
            assert_eq!(found.len(), 1);
            assert!(found[0].value.starts_with('v'));
        })
        .await
        .unwrap();
    }
}
