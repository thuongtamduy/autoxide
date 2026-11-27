//! Tài nguyên SCIM `User` ↔ tài khoản nhân sự của khách.
//!
//! Mọi lệnh ghi (POST, PUT, PATCH) đi qua [`ScimService::save`] trong MỘT transaction: khoá dòng
//! tài khoản, kiểm trùng, ghi `auth_user` + `auth_scim_users`, và khi chuyển sang `active = false`
//! thì khoá tài khoản, thu hồi phiên, refresh token, API key và ghi event — không có lúc nào tài
//! khoản đã khoá mà phiên còn sống.

use autoxide_kernel::time::Timestamp;
use std::collections::HashMap;

use autoxide_kernel::access::Realm;
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{ScopeExt as _, TenantContext};
use sea_orm::sea_query::{Expr, ExprTrait as _, LikeExpr, Query};
use sea_orm::{
    ActiveModelTrait as _, ColumnTrait as _, DatabaseTransaction, EntityTrait as _,
    PaginatorTrait as _, QueryFilter as _, QueryOrder as _, QuerySelect as _, Set, SqlErr,
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::entity::scim_user;
use super::error::ScimError;
use super::filter::{self, Filter, Literal, Op};
use super::patch;
use super::{BASE_PATH, LIST_SCHEMA, ScimCaller, ScimService, USER_SCHEMA};
use crate::events;
use crate::user::entity as user;

/// Độ dài tối đa của `userName`, `externalId`.
const ID_MAX_LENGTH: usize = 255;
/// Độ dài tối đa của tên (như `auth_user.name`).
const NAME_MAX_LENGTH: usize = 200;

fn now() -> Timestamp {
    Timestamp::now()
}

/// Thân `User` đã đọc và kiểm.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UserInput {
    pub user_name: String,
    pub external_id: Option<String>,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    /// Tên hiển thị trong Autoxide (`auth_user.name`).
    pub name: String,
    pub email: String,
    pub active: bool,
}

/// Chuỗi đã cắt khoảng trắng hai đầu; rỗng là không có.
fn text(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::trim).filter(|text| !text.is_empty()).map(str::to_owned)
}

/// `true` / `false`, kể cả dạng chuỗi `"True"` / `"False"` mà Entra ID gửi.
fn boolean(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(flag) => Some(*flag),
        Value::String(text) if text.eq_ignore_ascii_case("true") => Some(true),
        Value::String(text) if text.eq_ignore_ascii_case("false") => Some(false),
        _ => None,
    }
}

/// Đọc thân `User` (khoá đã chữ thường). Thuộc tính Autoxide không lưu (`title`, số điện thoại,
/// lược đồ mở rộng…) bị bỏ qua (RFC 7644 §3.3).
pub(crate) fn parse_user(body: &Value) -> Result<UserInput, ScimError> {
    let mut problems = Vec::new();
    let user_name = text(body.get("username")).unwrap_or_default();
    if user_name.is_empty() {
        problems.push("`userName` is required".to_owned());
    } else if user_name.chars().count() > ID_MAX_LENGTH {
        problems.push(format!("`userName` must be at most {ID_MAX_LENGTH} characters"));
    }
    let external_id = text(body.get("externalid"));
    if external_id.as_ref().is_some_and(|id| id.chars().count() > ID_MAX_LENGTH) {
        problems.push(format!("`externalId` must be at most {ID_MAX_LENGTH} characters"));
    }
    let name = body.get("name");
    let given_name = text(name.and_then(|name| name.get("givenname")));
    let family_name = text(name.and_then(|name| name.get("familyname")));
    let formatted = text(name.and_then(|name| name.get("formatted")));
    let joined = [given_name.as_deref(), family_name.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    let display = text(body.get("displayname"))
        .or(formatted)
        .or((!joined.is_empty()).then_some(joined))
        .unwrap_or_else(|| user_name.clone());
    for (field, value) in [
        ("displayName", Some(&display)),
        ("name.givenName", given_name.as_ref()),
        ("name.familyName", family_name.as_ref()),
    ] {
        if value.is_some_and(|value| value.chars().count() > NAME_MAX_LENGTH) {
            problems.push(format!("`{field}` must be at most {NAME_MAX_LENGTH} characters"));
        }
    }
    // Email chính: phần tử `primary`, không thì phần tử đầu; không có email thì `userName` khi
    // nó là email (Entra ID, Okta mặc định dùng email làm `userName`).
    let emails = body.get("emails").and_then(Value::as_array);
    let email = emails
        .and_then(|emails| {
            emails
                .iter()
                .find(|email| email.get("primary").and_then(boolean) == Some(true))
                .or_else(|| emails.first())
        })
        .and_then(|email| text(email.get("value")))
        .unwrap_or_else(|| user_name.clone());
    let email = autoxide_kernel::email::normalize(&email);
    if !user_name.is_empty() && crate::user::check_email_field("emails", &email).is_some() {
        problems
            .push("`emails` must hold a valid work email (or `userName` must be one)".to_owned());
    }
    let active = match body.get("active") {
        None | Some(Value::Null) => true,
        Some(value) => match boolean(value) {
            Some(flag) => flag,
            None => {
                problems.push("`active` must be true or false".to_owned());
                true
            }
        },
    };
    if !problems.is_empty() {
        return Err(ScimError::invalid_value(problems.join("; ")));
    }
    Ok(UserInput { user_name, external_id, given_name, family_name, name: display, email, active })
}

/// Bản JSON của một `User`.
pub(crate) fn render(account: &user::Model, link: &scim_user::Model, groups: &[Value]) -> Value {
    let mut name = json!({"formatted": account.name});
    if let Some(given) = &link.given_name {
        name["givenName"] = json!(given);
    }
    if let Some(family) = &link.family_name {
        name["familyName"] = json!(family);
    }
    let mut body = json!({
        "schemas": [USER_SCHEMA],
        "id": account.id,
        "userName": link.user_name,
        "displayName": account.name,
        "name": name,
        "emails": [{"value": account.email, "type": "work", "primary": true}],
        "active": account.disabled_at.is_none(),
        "groups": groups,
        "meta": {
            "resourceType": "User",
            "created": link.created_at,
            "lastModified": std::cmp::max(account.updated_at, link.updated_at),
            "location": format!("{BASE_PATH}/Users/{}", account.id),
        },
    });
    if let Some(external) = &link.external_id {
        body["externalId"] = json!(external);
    }
    body
}

/// Lỗi "không có User này".
fn user_not_found(id: &str) -> ScimError {
    ScimError::not_found("User", id)
}

impl ScimService {
    /// Một `User` theo ID.
    pub(crate) async fn get_user(&self, caller: &ScimCaller, id: &str) -> Result<Value, ScimError> {
        let tenant = &caller.tenant;
        let user_id = Uuid::try_parse(id).map_err(|_| user_not_found(id))?;
        let read = tenant.database().read();
        let link = scim_user::Entity::find_by_id((tenant.id().as_uuid(), user_id))
            .one(read)
            .await?
            .ok_or_else(|| user_not_found(id))?;
        let account = user::Entity::find_by_id(user_id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| user_not_found(id))?;
        let groups = self.groups_of(tenant, &[user_id]).await?;
        Ok(render(&account, &link, groups.get(&user_id).map(Vec::as_slice).unwrap_or_default()))
    }

    /// Một trang `User` (RFC 7644 §3.4.2): `filter`, `startIndex` (từ 1), `count`.
    pub(crate) async fn list_users(
        &self,
        caller: &ScimCaller,
        filter: Option<&str>,
        start_index: u64,
        count: u64,
    ) -> Result<Value, ScimError> {
        let tenant = &caller.tenant;
        let mut select = scim_user::Entity::find().scoped(tenant);
        if let Some(filter) = filter.map(str::trim).filter(|filter| !filter.is_empty()) {
            select = select.filter(user_condition(&filter::parse(filter)?, tenant)?);
        }
        let read = tenant.database().read();
        let total = select.clone().count(read).await?;
        // `OFFSET` có chủ đích: SCIM chỉ có phân trang theo chỉ số (D-20261121-scim). Danh sách
        // nhân viên một khách nhỏ; thứ tự theo ID (UUIDv7) ổn định giữa các trang.
        let links = if count == 0 {
            Vec::new()
        } else {
            select
                .order_by_asc(scim_user::Column::Id)
                .offset(start_index.saturating_sub(1))
                .limit(count)
                .all(read)
                .await?
        };
        let ids: Vec<Uuid> = links.iter().map(|link| link.id).collect();
        let accounts: HashMap<Uuid, user::Model> = user::Entity::find()
            .filter(user::Column::Id.is_in(ids.iter().copied()))
            .scoped(tenant)
            .all(read)
            .await?
            .into_iter()
            .map(|account| (account.id, account))
            .collect();
        let groups = self.groups_of(tenant, &ids).await?;
        let resources: Vec<Value> = links
            .iter()
            .filter_map(|link| {
                let account = accounts.get(&link.id)?;
                let groups = groups.get(&link.id).map(Vec::as_slice).unwrap_or_default();
                Some(render(account, link, groups))
            })
            .collect();
        Ok(json!({
            "schemas": [LIST_SCHEMA],
            "totalResults": total,
            "startIndex": start_index,
            "itemsPerPage": resources.len(),
            "Resources": resources,
        }))
    }

    /// `POST /Users`: tạo tài khoản nhân sự, hay nhận tài khoản nhân sự có sẵn cùng email chưa
    /// liên kết SCIM (tài khoản tạo trước khi khách bật SCIM).
    pub(crate) async fn create_user(
        &self,
        caller: &ScimCaller,
        body: &Value,
    ) -> Result<Value, ScimError> {
        let input = parse_user(&patch::lowercase_keys(body.clone()))?;
        let tenant = &caller.tenant;
        let transaction = tenant.database().begin_write().await?;
        let result = self.save(&transaction, caller, None, &input).await;
        let (account, link) = match result {
            Ok(saved) => saved,
            Err(error) => {
                transaction.rollback().await?;
                return Err(error);
            }
        };
        transaction.commit().await?;
        tracing::info!(user_id = %account.id, "SCIM user provisioned");
        Ok(render(&account, &link, &[]))
    }

    /// `PUT /Users/{id}`: thay toàn bộ thuộc tính.
    pub(crate) async fn replace_user(
        &self,
        caller: &ScimCaller,
        id: &str,
        body: &Value,
    ) -> Result<Value, ScimError> {
        let body = patch::lowercase_keys(body.clone());
        self.update_user(caller, id, |_| parse_user(&body)).await
    }

    /// `PATCH /Users/{id}` (RFC 7644 §3.5.2).
    pub(crate) async fn patch_user(
        &self,
        caller: &ScimCaller,
        id: &str,
        body: &Value,
    ) -> Result<Value, ScimError> {
        self.update_user(caller, id, |current| {
            let mut resource = patch::lowercase_keys(current);
            patch::apply(&mut resource, body, &["id"])?;
            parse_user(&resource)
        })
        .await
    }

    /// Khoá dòng tài khoản, dựng thân mới từ bản hiện tại bằng `change`, rồi ghi — một
    /// transaction (đọc → so → ghi không chen được, D-181).
    async fn update_user(
        &self,
        caller: &ScimCaller,
        id: &str,
        change: impl FnOnce(Value) -> Result<UserInput, ScimError>,
    ) -> Result<Value, ScimError> {
        let tenant = &caller.tenant;
        let user_id = Uuid::try_parse(id).map_err(|_| user_not_found(id))?;
        let transaction = tenant.database().begin_write().await?;
        let result = async {
            let account =
                rows::lock_one(&transaction, user::Entity::find_by_id(user_id).scoped(tenant))
                    .await?
                    .ok_or_else(|| user_not_found(id))?;
            let link = scim_user::Entity::find_by_id((tenant.id().as_uuid(), user_id))
                .one(&transaction)
                .await?
                .ok_or_else(|| user_not_found(id))?;
            let input = change(render(&account, &link, &[]))?;
            self.save(&transaction, caller, Some((account, link)), &input).await
        }
        .await;
        let (account, link) = match result {
            Ok(saved) => saved,
            Err(error) => {
                transaction.rollback().await?;
                return Err(error);
            }
        };
        transaction.commit().await?;
        let groups = self.groups_of(tenant, &[user_id]).await?;
        Ok(render(&account, &link, groups.get(&user_id).map(Vec::as_slice).unwrap_or_default()))
    }

    /// `DELETE /Users/{id}`: quy trình xoá tài khoản sẵn có (event `auth.user_deleted` để module
    /// khác xoá phần của họ; không xoá được admin cuối cùng). Người tạo token phải bao trùm quyền
    /// của tài khoản (như khi khoá — xoá còn nặng hơn khoá).
    pub(crate) async fn delete_user(&self, caller: &ScimCaller, id: &str) -> Result<(), ScimError> {
        let tenant = &caller.tenant;
        let user_id = Uuid::try_parse(id).map_err(|_| user_not_found(id))?;
        let read = tenant.database().read();
        scim_user::Entity::find_by_id((tenant.id().as_uuid(), user_id))
            .one(read)
            .await?
            .ok_or_else(|| user_not_found(id))?;
        self.require_creator_covers(read, caller, user_id).await?;
        self.users.delete(tenant, caller.actor(), user_id).await.map_err(|error| {
            if error.code() == "auth.user_not_found" { user_not_found(id) } else { error.into() }
        })?;
        tracing::info!(user_id = %user_id, "SCIM user deleted");
        Ok(())
    }

    /// `Ok` nếu người tạo token SCIM của `caller` bao trùm mọi permission của tài khoản `target`
    /// (D-079): token do người quyền thấp tạo không nhận về, khoá hay đổi email tài khoản quyền cao
    /// hơn. Người tạo đã bị xoá hay khoá coi như không có permission nào.
    async fn require_creator_covers<C: sea_orm::ConnectionTrait>(
        &self,
        connection: &C,
        caller: &ScimCaller,
        target: Uuid,
    ) -> Result<(), ScimError> {
        let creator = self.authority(caller).await?;
        let tenant = &caller.tenant;
        self.roles.require_covers(tenant, &creator, target).await?;
        // `require_covers` so theo TÊN permission, bỏ qua phạm vi: người tạo có quyền ở một nhánh vẫn qua với tài khoản
        // ở nhánh khác (PT-19). Thêm luật "giữ được" của membership: người tạo phải cấp được từng (nút, role) tài khoản
        // đang giữ — cùng luật với nhóm SCIM và luật ánh xạ (D-20261125-scim-grant-cover).
        use crate::role::entity::user_role;
        let root = autoxide_kernel::tenancy::root_of(tenant.id()).as_uuid();
        let held: Vec<(Uuid, Uuid)> = user_role::Entity::find()
            .filter(user_role::Column::UserId.eq(target))
            .scoped(tenant)
            .all(connection)
            .await?
            .into_iter()
            .map(|row| (row.org_id.unwrap_or(root), row.role_id))
            .collect();
        crate::role::require_grantable(connection, tenant, &creator, &held, &[]).await?;
        Ok(())
    }

    /// Quyền mà token SCIM của `caller` được dùng: quyền của người tạo token **lúc này** (không
    /// phải lúc tạo token) — người tạo bị hạ quyền thì token hạ theo; bị khoá, bị xoá hay không còn
    /// là nhân sự thì token không cấp, giữ hay gỡ được gì (D-20261125-scim-grant-cover).
    pub(super) async fn authority(
        &self,
        caller: &ScimCaller,
    ) -> Result<autoxide_kernel::access::Principal, ScimError> {
        let tenant = &caller.tenant;
        Ok(match self.roles.user_principal(tenant, caller.created_by).await? {
            // Người tạo còn hoạt động, là nhân sự và CÒN `auth.scim.manage`: mất quyền quản lý SCIM
            // thì token của họ không cấp, giữ hay gỡ được gì (`creator_status = missing_permission`).
            Some((user, principal))
                if user.is_active()
                    && user.realm == Realm::Staff
                    && principal.has(&crate::permissions::SCIM_MANAGE) =>
            {
                principal
            }
            _ => autoxide_kernel::access::Principal::new(caller.created_by, [])
                .in_tenant(tenant.id()),
        })
    }

    /// Ghi `input` cho tài khoản `existing` (đã khoá dòng; `None`: tạo / nhận tài khoản) trong
    /// `transaction`.
    async fn save(
        &self,
        transaction: &DatabaseTransaction,
        caller: &ScimCaller,
        existing: Option<(user::Model, scim_user::Model)>,
        input: &UserInput,
    ) -> Result<(user::Model, scim_user::Model), ScimError> {
        let tenant = &caller.tenant;
        let actor = caller.actor();
        let own_id = existing.as_ref().map(|(account, _)| account.id);
        let user_name_key = input.user_name.to_lowercase();
        // Trùng `userName` / `externalId` với User khác: `uniqueness` (RFC 7644 §3.3). Index unique
        // vẫn chặn khi hai request đua nhau (bắt ở dưới).
        let mut taken = scim_user::Entity::find()
            .filter(scim_user::Column::UserNameKey.eq(user_name_key.clone()))
            .scoped(tenant);
        if let Some(own) = own_id {
            taken = taken.filter(scim_user::Column::Id.ne(own));
        }
        if taken.one(transaction).await?.is_some() {
            return Err(ScimError::uniqueness(format!(
                "userName `{}` is already used by another User.",
                input.user_name
            )));
        }
        if let Some(external) = &input.external_id {
            let mut taken = scim_user::Entity::find()
                .filter(scim_user::Column::ExternalId.eq(external.clone()))
                .scoped(tenant);
            if let Some(own) = own_id {
                taken = taken.filter(scim_user::Column::Id.ne(own));
            }
            if taken.one(transaction).await?.is_some() {
                return Err(ScimError::uniqueness(format!(
                    "externalId `{external}` is already used by another User."
                )));
            }
        }
        // Tài khoản nhân sự khác đang dùng email này.
        let mut same_email = user::Entity::find()
            .filter(user::Column::Realm.eq("staff"))
            .filter(user::Column::Email.eq(input.email.clone()))
            .scoped(tenant);
        if let Some(own) = own_id {
            same_email = same_email.filter(user::Column::Id.ne(own));
        }
        let now = now();
        let (before, account, link_before) = match existing {
            Some((account, link)) => {
                if same_email.one(transaction).await?.is_some() {
                    return Err(ScimError::uniqueness(format!(
                        "The email `{}` belongs to another account.",
                        input.email
                    )));
                }
                (Some(snapshot(&account)), account, Some(link))
            }
            None => match rows::lock_one(transaction, same_email).await? {
                // Tài khoản có sẵn: nhận về, trừ khi nó đã thuộc User SCIM khác.
                Some(account) => {
                    let linked = scim_user::Entity::find_by_id((tenant.id().as_uuid(), account.id))
                        .one(transaction)
                        .await?;
                    if linked.is_some() {
                        return Err(ScimError::uniqueness(format!(
                            "The email `{}` belongs to another User.",
                            input.email
                        )));
                    }
                    // Nhận về tài khoản có sẵn: người tạo token phải bao trùm quyền của nó.
                    self.require_creator_covers(transaction, caller, account.id).await?;
                    (Some(snapshot(&account)), account, None)
                }
                None => {
                    let account = user::ActiveModel {
                        id: Set(Uuid::now_v7()),
                        tenant_id: Set(tenant.id().as_uuid()),
                        email: Set(input.email.clone()),
                        search_text: Set(crate::user::search_text(&input.email, &input.name)),
                        name: Set(input.name.clone()),
                        password_hash: Set(None),
                        disabled_at: Set(None),
                        // Hệ thống danh tính của khách là nguồn của email nhân viên (D-20261121-scim).
                        email_verified_at: Set(Some(now)),
                        created_at: Set(now),
                        updated_at: Set(now),
                        last_sign_in_at: Set(None),
                        locale: Set(None),
                        time_zone: Set(None),
                        realm: Set(user::realm_str(Realm::Staff).to_owned()),
                        phone_key: Set(None),
                    }
                    .insert(transaction)
                    .await
                    .map_err(unique_to_scim)?;
                    (None, account, None)
                }
            },
        };
        let id = account.id;
        let was_active = account.disabled_at.is_none();
        // Khoá tài khoản có thể lấy đi admin cuối cùng: đếm trước (sau khi đã khoá dòng người
        // dùng — cùng thứ tự khoá với `UserService::update`, D-156).
        let admins_before = if was_active && !input.active {
            crate::role::count_active_admins(transaction, tenant).await?
        } else {
            0
        };
        let email_changed = account.email != input.email;
        // Mọi thay đổi trên tài khoản đã liên kết — không chỉ email và `active` — cần người tạo token
        // bao trùm quyền của tài khoản: `userName` / `externalId` là khoá hệ thống danh tính dùng để
        // khớp người; đổi chúng trên tài khoản quyền cao thì lần đẩy sau (bằng token của người quyền
        // cao) có thể ghi hồ sơ của người khác, kể cả email, vào tài khoản đó (PT-19,
        // D-20261125-scim-sync-report). PUT / PATCH không đổi gì thì không kiểm.
        let changed = link_before.as_ref().is_some_and(|link| {
            email_changed
                || was_active != input.active
                || account.name != input.name
                || link.user_name != input.user_name
                || link.external_id != input.external_id
                || link.given_name != input.given_name
                || link.family_name != input.family_name
        });
        if changed {
            self.require_creator_covers(transaction, caller, id).await?;
        }
        let mut update = user::Entity::update_many()
            .col_expr(user::Column::UpdatedAt, Expr::value(now))
            .col_expr(user::Column::Name, Expr::value(input.name.clone()))
            .col_expr(user::Column::Email, Expr::value(input.email.clone()))
            .col_expr(
                user::Column::SearchText,
                Expr::value(crate::user::search_text(&input.email, &input.name)),
            );
        if email_changed {
            // Tài khoản do SCIM tạo (liên kết và tài khoản sinh cùng lúc): hệ thống danh tính là
            // nguồn của email (D-20261121-scim). Tài khoản có sẵn được nhận về: email mới chưa ai
            // xác nhận — để chủ tài khoản xác minh lại (D-20261121-scim-claim-covers).
            let created_by_scim =
                link_before.as_ref().is_some_and(|link| link.created_at == account.created_at);
            let verified = created_by_scim.then_some(now);
            update = update.col_expr(user::Column::EmailVerifiedAt, Expr::value(verified));
        }
        update = match (was_active, input.active) {
            (true, false) => update.col_expr(user::Column::DisabledAt, Expr::value(Some(now))),
            (false, true) => {
                update.col_expr(user::Column::DisabledAt, Expr::value(None::<Timestamp>))
            }
            _ => update,
        };
        update
            .filter(user::Column::Id.eq(id))
            .scoped(tenant)
            .exec(transaction)
            .await
            .map_err(unique_to_scim)?;
        if email_changed && link_before.is_some() {
            // Email đổi qua SCIM: mọi cách vào tài khoản gắn với email cũ đóng lại trong CÙNG
            // transaction — mật khẩu, phiên, refresh token, API key, passkey, TOTP, link đăng nhập /
            // đặt lại đang chờ, liên kết đăng nhập ngoài. Không thì người đã giữ tài khoản dưới email
            // cũ (vd kẻ chiếm trước `userName` mà hệ thống danh tính sẽ khớp, rồi đặt mật khẩu qua
            // email của mình) vẫn đăng nhập được sau khi hệ thống danh tính trao tài khoản cho người
            // thật; chủ mới đăng nhập bằng nhà cung cấp danh tính hay đặt lại mật khẩu qua email mới.
            self.users.lock_out_in(tenant, transaction, actor, id).await?;
            tracing::info!(user_id = %id, "SCIM email change: sign-in credentials revoked");
        }
        if was_active && !input.active {
            // Nhân viên nghỉ: mọi đường vào đóng ngay, cùng transaction với việc khoá.
            crate::session::SessionService::revoke_user(transaction, tenant, id).await?;
            crate::api_key::revoke_all_in(transaction, tenant, &self.outbox, actor, id).await?;
            // Token SCIM và vai người cấu hình nhà cung cấp đăng nhập của nhân viên nghỉ: thu hồi như
            // API key — bật lại tài khoản không làm chúng sống lại.
            crate::user::revoke_durable_grants_in(
                transaction,
                tenant,
                &self.outbox,
                actor,
                id,
                "disabled",
            )
            .await?;
            if admins_before > 0
                && crate::role::count_active_admins(transaction, tenant).await? == 0
            {
                return Err(ScimError::from(crate::role::last_admin()));
            }
        }
        let link = scim_user::Model {
            tenant_id: tenant.id().as_uuid(),
            id,
            user_name: input.user_name.clone(),
            user_name_key,
            external_id: input.external_id.clone(),
            external_key: input.external_id.as_ref().map(|external| external.to_lowercase()),
            given_name: input.given_name.clone(),
            family_name: input.family_name.clone(),
            created_at: link_before.as_ref().map_or(now, |link| link.created_at),
            updated_at: now,
        };
        let mut active = scim_user::ActiveModel::from(link.clone());
        let saved = if link_before.is_some() {
            // `from(Model)` đánh dấu mọi cột là "chưa đổi": đặt lại để `update` ghi chúng.
            active = active.reset_all();
            active.update(transaction).await
        } else {
            active.insert(transaction).await
        };
        saved.map_err(unique_to_scim)?;
        let account = user::Entity::find_by_id(id)
            .scoped(tenant)
            .one(transaction)
            .await?
            .ok_or_else(|| user_not_found(&id.to_string()))?;
        let after = Some(snapshot(&account));
        let subject = events::user_subject(id);
        match before {
            None => {
                let event = events::UserCreated {
                    before: None,
                    after: after.map(|mut after| {
                        after["scim"] = json!(true);
                        after
                    }),
                };
                self.outbox.record(transaction, tenant, actor, subject, &event).await?;
            }
            Some(before) if before != snapshot(&account) || link_before.is_none() => {
                let event = events::UserUpdated { before: Some(before), after };
                self.outbox.record(transaction, tenant, actor, subject, &event).await?;
            }
            Some(_) => {}
        }
        Ok((account, link))
    }
}

/// Ảnh chụp cho audit (như `UserService`): không có hash mật khẩu.
fn snapshot(account: &user::Model) -> Value {
    json!({"email": account.email, "name": account.name, "disabled_at": account.disabled_at})
}

/// Vi phạm index unique (hai request đua nhau) thành `uniqueness`.
fn unique_to_scim(error: sea_orm::DbErr) -> ScimError {
    match error.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => ScimError::uniqueness(
            "userName, externalId or email is already used by another account.",
        ),
        _ => ScimError::from(error),
    }
}

/// Điều kiện SQL cho bộ lọc `User`. Thuộc tính lọc được: `userName`, `externalId`, `emails`
/// (`emails.value`), `active`, `id`. Chuỗi so không phân biệt hoa thường, trừ `externalId eq`
/// (caseExact theo RFC 7643).
fn user_condition(filter: &Filter, tenant: &TenantContext) -> Result<Expr, ScimError> {
    Ok(match filter {
        Filter::And(left, right) => {
            user_condition(left, tenant)?.and(user_condition(right, tenant)?)
        }
        Filter::Or(left, right) => user_condition(left, tenant)?.or(user_condition(right, tenant)?),
        Filter::Present { attribute } => match attribute.as_str() {
            "externalid" => Expr::col(scim_user::Column::ExternalId).is_not_null(),
            "username" | "id" | "emails" | "emails.value" | "active" => Expr::val(1).eq(1),
            other => return Err(unknown_attribute(other)),
        },
        Filter::Compare { attribute, op, value } => match attribute.as_str() {
            "username" => {
                text_condition(scim_user::Column::UserNameKey, *op, &lower_text(attribute, value)?)
            }
            "externalid" => {
                let wanted = expect_text(attribute, value)?;
                match op {
                    Op::Eq => Expr::col(scim_user::Column::ExternalId).eq(wanted),
                    Op::Ne => Expr::col(scim_user::Column::ExternalId).ne(wanted),
                    _ => {
                        text_condition(scim_user::Column::ExternalKey, *op, &wanted.to_lowercase())
                    }
                }
            }
            "emails" | "emails.value" => {
                let wanted = lower_text(attribute, value)?;
                accounts_where(tenant, text_condition(user::Column::Email, *op, &wanted))
            }
            "active" => {
                let Literal::Bool(wanted) = value else {
                    return Err(ScimError::invalid_filter("`active` compares with true or false."));
                };
                let active = match op {
                    Op::Eq => *wanted,
                    Op::Ne => !*wanted,
                    _ => {
                        return Err(ScimError::invalid_filter("`active` supports eq and ne only."));
                    }
                };
                let condition = if active {
                    Expr::col(user::Column::DisabledAt).is_null()
                } else {
                    Expr::col(user::Column::DisabledAt).is_not_null()
                };
                accounts_where(tenant, condition)
            }
            "id" => {
                let wanted = expect_text(attribute, value)?;
                match (Uuid::try_parse(&wanted), op) {
                    (Ok(id), Op::Eq) => Expr::col(scim_user::Column::Id).eq(id),
                    (Ok(id), Op::Ne) => Expr::col(scim_user::Column::Id).ne(id),
                    (Err(_), Op::Ne) => Expr::val(1).eq(1),
                    (_, Op::Eq) => Expr::val(1).eq(0),
                    _ => return Err(ScimError::invalid_filter("`id` supports eq and ne only.")),
                }
            }
            other => return Err(unknown_attribute(other)),
        },
    })
}

fn unknown_attribute(attribute: &str) -> ScimError {
    ScimError::invalid_filter(format!(
        "Cannot filter Users by `{attribute}`; use userName, externalId, emails, active or id."
    ))
}

fn expect_text(attribute: &str, value: &Literal) -> Result<String, ScimError> {
    match value {
        Literal::Text(text) => Ok(text.clone()),
        _ => {
            Err(ScimError::invalid_filter(format!("`{attribute}` compares with a quoted string.")))
        }
    }
}

fn lower_text(attribute: &str, value: &Literal) -> Result<String, ScimError> {
    expect_text(attribute, value).map(|text| text.to_lowercase())
}

/// So cột (đã lưu chữ thường) với `wanted` (đã chữ thường).
pub(crate) fn text_condition<C: sea_orm::ColumnTrait>(column: C, op: Op, wanted: &str) -> Expr {
    match filter::like_pattern(op, wanted) {
        Some(pattern) => Expr::col(column).like(LikeExpr::new(pattern).escape('\\')),
        None if op == Op::Ne => Expr::col(column).ne(wanted),
        None => Expr::col(column).eq(wanted),
    }
}

/// `auth_scim_users.id IN (SELECT id FROM auth_user WHERE <condition> AND tenant_id = …)`.
fn accounts_where(tenant: &TenantContext, condition: Expr) -> Expr {
    let accounts = Query::select()
        .column(user::Column::Id)
        .from(user::Entity)
        .and_where(Expr::col(user::Column::TenantId).eq(tenant.id().as_uuid()))
        .and_where(condition)
        .to_owned();
    Expr::col((scim_user::Entity, scim_user::Column::Id)).in_subquery(accounts)
}
