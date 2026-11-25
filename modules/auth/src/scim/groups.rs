//! Tài nguyên SCIM `Group`: nhóm của hệ thống danh tính và thành viên của nó. Nhóm tự nó không cấp
//! quyền gì; luật ánh xạ của khách (`auth_scim_group_mappings`, [`super::mappings`]) biến "ở trong
//! nhóm X" thành "giữ role R ở nút N".
//!
//! Ghi nhóm và thành viên trong một transaction; sau khi commit, membership của từng người có thành
//! viên đổi được đồng bộ ([`ScimService::sync_users`]) — mỗi người một transaction, vì hạn mức số
//! nhân sự của gói giữ chỗ ngoài transaction. Đồng bộ lỗi (vượt hạn mức, gỡ admin cuối cùng) thì
//! trả lỗi để hệ thống danh tính gửi lại; gửi lại không đổi nhóm thêm lần nữa, chỉ đồng bộ lại.

use autoxide_kernel::time::Timestamp;
use std::collections::{BTreeSet, HashMap};

use autoxide_kernel::tenancy::{ScopeExt as _, TenantContext};
use sea_orm::sea_query::{Expr, ExprTrait as _, Query};
use sea_orm::{
    ActiveModelTrait as _, ColumnTrait as _, ConnectionTrait, DatabaseTransaction,
    EntityTrait as _, PaginatorTrait as _, QueryFilter as _, QueryOrder as _, QuerySelect as _,
    Set, SqlErr,
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::entity::{group, mapping, member, scim_user};
use super::error::ScimError;
use super::filter::{self, Filter, Literal, Op};
use super::users::text_condition;
use super::{BASE_PATH, GROUP_SCHEMA, LIST_SCHEMA, ScimCaller, ScimService, patch};
use crate::events;
use crate::user::entity as user;

const NAME_MAX_LENGTH: usize = 255;
/// Nhiều thành viên "chờ" nhất được đồng bộ lại sau một lần ghi nhóm: nhóm có hàng nghìn người chờ
/// (vd gói hết chỗ) không làm mỗi request chậm theo số người đó.
const PENDING_MAX: usize = 100;
/// Nhiều thành viên nhất trong một lần ghi nhóm.
const MAX_MEMBERS: usize = 10_000;

fn now() -> Timestamp {
    Timestamp::now()
}

fn group_not_found(id: &str) -> ScimError {
    ScimError::not_found("Group", id)
}

/// Mọi ID người dùng (chuỗi là UUID) xuất hiện trong thân request: thành viên request nêu tên.
fn named_users(body: &Value) -> BTreeSet<Uuid> {
    let mut found = BTreeSet::new();
    let mut stack = vec![body];
    while let Some(value) = stack.pop() {
        match value {
            Value::String(text) => {
                if let Ok(id) = Uuid::try_parse(text.trim()) {
                    found.insert(id);
                }
            }
            Value::Array(items) => stack.extend(items),
            Value::Object(fields) => stack.extend(fields.values()),
            _ => {}
        }
    }
    found
}

/// Thân `Group` đã đọc và kiểm.
#[derive(Debug, Clone, PartialEq)]
struct GroupInput {
    display_name: String,
    external_id: Option<String>,
    members: BTreeSet<Uuid>,
}

fn parse_group(body: &Value) -> Result<GroupInput, ScimError> {
    let text = |key: &str| {
        body.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let mut problems = Vec::new();
    let display_name = text("displayname").unwrap_or_default();
    if display_name.is_empty() || display_name.chars().count() > NAME_MAX_LENGTH {
        problems.push(format!("`displayName` must be 1 to {NAME_MAX_LENGTH} characters"));
    }
    let external_id = text("externalid");
    if external_id.as_ref().is_some_and(|id| id.chars().count() > NAME_MAX_LENGTH) {
        problems.push(format!("`externalId` must be at most {NAME_MAX_LENGTH} characters"));
    }
    let mut members = BTreeSet::new();
    match body.get("members") {
        None | Some(Value::Null) => {}
        Some(Value::Array(items)) => {
            for item in items {
                match item
                    .get("value")
                    .and_then(Value::as_str)
                    .and_then(|id| Uuid::try_parse(id.trim()).ok())
                {
                    Some(id) => {
                        members.insert(id);
                    }
                    None => problems
                        .push("every `members` entry needs the `value` of a User id".to_owned()),
                }
            }
        }
        Some(_) => problems.push("`members` must be a list".to_owned()),
    }
    if members.len() > MAX_MEMBERS {
        problems.push(format!("a group may have at most {MAX_MEMBERS} members"));
    }
    if !problems.is_empty() {
        problems.dedup();
        return Err(ScimError::invalid_value(problems.join("; ")));
    }
    Ok(GroupInput { display_name, external_id, members })
}

/// Thành viên của các nhóm `ids`: nhóm → [(ID người, tên)].
async fn members_of<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<(Uuid, String)>>, ScimError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = member::Entity::find()
        .filter(member::Column::GroupId.is_in(ids.iter().copied()))
        .scoped(tenant)
        .order_by_asc(member::Column::Id)
        .all(connection)
        .await?;
    let names: HashMap<Uuid, String> = user::Entity::find()
        .filter(user::Column::Id.is_in(rows.iter().map(|row| row.user_id)))
        .scoped(tenant)
        .all(connection)
        .await?
        .into_iter()
        .map(|account| (account.id, account.name))
        .collect();
    let mut found: HashMap<Uuid, Vec<(Uuid, String)>> = HashMap::new();
    for row in rows {
        let name = names.get(&row.user_id).cloned().unwrap_or_default();
        found.entry(row.group_id).or_default().push((row.user_id, name));
    }
    Ok(found)
}

fn render(row: &group::Model, members: Option<&[(Uuid, String)]>) -> Value {
    let mut body = json!({
        "schemas": [GROUP_SCHEMA],
        "id": row.id,
        "displayName": row.display_name,
        "meta": {
            "resourceType": "Group",
            "created": row.created_at,
            "lastModified": row.updated_at,
            "location": format!("{BASE_PATH}/Groups/{}", row.id),
        },
    });
    if let Some(members) = members {
        body["members"] = members
            .iter()
            .map(|(id, name)| {
                json!({"value": id, "display": name, "type": "User", "$ref": format!("{BASE_PATH}/Users/{id}")})
            })
            .collect();
    }
    if let Some(external) = &row.external_id {
        body["externalId"] = json!(external);
    }
    body
}

impl ScimService {
    /// Nhóm của các người `ids` (thuộc tính `groups` của `User`): người → `[{value, display, $ref}]`.
    pub(crate) async fn groups_of(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, Vec<Value>>, ScimError> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let read = tenant.database().read();
        let rows = member::Entity::find()
            .filter(member::Column::UserId.is_in(ids.iter().copied()))
            .scoped(tenant)
            .all(read)
            .await?;
        let groups: HashMap<Uuid, String> = group::Entity::find()
            .filter(group::Column::Id.is_in(rows.iter().map(|row| row.group_id)))
            .scoped(tenant)
            .all(read)
            .await?
            .into_iter()
            .map(|row| (row.id, row.display_name))
            .collect();
        let mut found: HashMap<Uuid, Vec<Value>> = HashMap::new();
        for row in rows {
            let Some(name) = groups.get(&row.group_id) else { continue };
            found.entry(row.user_id).or_default().push(json!({
                "value": row.group_id,
                "display": name,
                "$ref": format!("{BASE_PATH}/Groups/{}", row.group_id),
            }));
        }
        Ok(found)
    }

    pub(crate) async fn get_group(
        &self,
        caller: &ScimCaller,
        id: &str,
        with_members: bool,
    ) -> Result<Value, ScimError> {
        let tenant = &caller.tenant;
        let group_id = Uuid::try_parse(id).map_err(|_| group_not_found(id))?;
        let read = tenant.database().read();
        let row = group::Entity::find_by_id((tenant.id().as_uuid(), group_id))
            .one(read)
            .await?
            .ok_or_else(|| group_not_found(id))?;
        let members =
            if with_members { Some(members_of(read, tenant, &[group_id]).await?) } else { None };
        let list = members
            .as_ref()
            .map(|members| members.get(&group_id).map(Vec::as_slice).unwrap_or_default());
        Ok(render(&row, list))
    }

    /// Một trang `Group`.
    pub(crate) async fn list_groups(
        &self,
        caller: &ScimCaller,
        filter: Option<&str>,
        start_index: u64,
        count: u64,
        with_members: bool,
    ) -> Result<Value, ScimError> {
        let tenant = &caller.tenant;
        let mut select = group::Entity::find().scoped(tenant);
        if let Some(filter) = filter.map(str::trim).filter(|filter| !filter.is_empty()) {
            select = select.filter(group_condition(&filter::parse(filter)?, tenant)?);
        }
        let read = tenant.database().read();
        let total = select.clone().count(read).await?;
        // `OFFSET` có chủ đích, như `Users` (D-20261121-scim).
        let rows = if count == 0 {
            Vec::new()
        } else {
            select
                .order_by_asc(group::Column::Id)
                .offset(start_index.saturating_sub(1))
                .limit(count)
                .all(read)
                .await?
        };
        let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        let members = if with_members { Some(members_of(read, tenant, &ids).await?) } else { None };
        let resources: Vec<Value> = rows
            .iter()
            .map(|row| {
                let list = members
                    .as_ref()
                    .map(|members| members.get(&row.id).map(Vec::as_slice).unwrap_or_default());
                render(row, list)
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

    pub(crate) async fn create_group(
        &self,
        caller: &ScimCaller,
        body: &Value,
    ) -> Result<Value, ScimError> {
        let input = parse_group(&patch::lowercase_keys(body.clone()))?;
        let named = input.members.clone();
        let tenant = &caller.tenant;
        let transaction = tenant.database().begin_write().await?;
        let saved = self.save_group(&transaction, caller, None, &input).await;
        let (row, changed) = match saved {
            Ok(saved) => saved,
            Err(error) => {
                transaction.rollback().await?;
                return Err(error);
            }
        };
        transaction.commit().await?;
        self.sync_group_members(caller, row.id, changed, &named).await?;
        let members = members_of(tenant.database().write(), tenant, &[row.id]).await?;
        Ok(render(&row, Some(members.get(&row.id).map(Vec::as_slice).unwrap_or_default())))
    }

    pub(crate) async fn replace_group(
        &self,
        caller: &ScimCaller,
        id: &str,
        body: &Value,
    ) -> Result<Value, ScimError> {
        let body = patch::lowercase_keys(body.clone());
        self.update_group(caller, id, named_users(&body), |_| parse_group(&body)).await
    }

    pub(crate) async fn patch_group(
        &self,
        caller: &ScimCaller,
        id: &str,
        body: &Value,
    ) -> Result<Value, ScimError> {
        self.update_group(caller, id, named_users(body), |current| {
            let mut resource = patch::lowercase_keys(current);
            patch::apply(&mut resource, body, &["id"])?;
            parse_group(&resource)
        })
        .await
    }

    async fn update_group(
        &self,
        caller: &ScimCaller,
        id: &str,
        named: BTreeSet<Uuid>,
        change: impl FnOnce(Value) -> Result<GroupInput, ScimError>,
    ) -> Result<Value, ScimError> {
        let tenant = &caller.tenant;
        let group_id = Uuid::try_parse(id).map_err(|_| group_not_found(id))?;
        let transaction = tenant.database().begin_write().await?;
        let result = async {
            // Khoá dòng nhóm: hai PATCH thành viên cùng lúc nối đuôi nhau (D-181).
            let row = autoxide_kernel::rows::lock_one(
                &transaction,
                group::Entity::find_by_id((tenant.id().as_uuid(), group_id)),
            )
            .await?
            .ok_or_else(|| group_not_found(id))?;
            let members = members_of(&transaction, tenant, &[group_id]).await?;
            let current =
                render(&row, Some(members.get(&group_id).map(Vec::as_slice).unwrap_or_default()));
            let input = change(current)?;
            self.save_group(&transaction, caller, Some(row), &input).await
        }
        .await;
        let (row, changed) = match result {
            Ok(saved) => saved,
            Err(error) => {
                transaction.rollback().await?;
                return Err(error);
            }
        };
        transaction.commit().await?;
        self.sync_group_members(caller, row.id, changed, &named).await?;
        let members = members_of(tenant.database().write(), tenant, &[row.id]).await?;
        Ok(render(&row, Some(members.get(&row.id).map(Vec::as_slice).unwrap_or_default())))
    }

    /// `DELETE /Groups/{id}`: xoá nhóm, thành viên và luật ánh xạ của nó; membership do nhóm cấp
    /// bị gỡ.
    pub(crate) async fn delete_group(
        &self,
        caller: &ScimCaller,
        id: &str,
    ) -> Result<(), ScimError> {
        let tenant = &caller.tenant;
        let group_id = Uuid::try_parse(id).map_err(|_| group_not_found(id))?;
        let transaction = tenant.database().begin_write().await?;
        let Some(row) = autoxide_kernel::rows::lock_one(
            &transaction,
            group::Entity::find_by_id((tenant.id().as_uuid(), group_id)),
        )
        .await?
        else {
            transaction.rollback().await?;
            return Err(group_not_found(id));
        };
        // Xoá nhóm gỡ mọi luật của nó và role nó đã cấp: người tạo token phải gỡ được từng role.
        if let Err(error) =
            self.require_group_grantable(&transaction, caller, group_id, false).await
        {
            transaction.rollback().await?;
            return Err(error);
        }
        let former: BTreeSet<Uuid> = member::Entity::find()
            .filter(member::Column::GroupId.eq(group_id))
            .scoped(tenant)
            .all(&transaction)
            .await?
            .into_iter()
            .map(|row| row.user_id)
            .collect();
        member::Entity::delete_many()
            .filter(member::Column::GroupId.eq(group_id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        mapping::Entity::delete_many()
            .filter(mapping::Column::GroupId.eq(group_id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        group::Entity::delete_many()
            .filter(group::Column::Id.eq(group_id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let event = events::ScimGroupDeleted {
            before: Some(
                json!({"id": group_id, "display_name": row.display_name, "members": former.len()}),
            ),
            after: None,
        };
        self.outbox
            .record(&transaction, tenant, caller.actor(), group_subject(group_id), &event)
            .await?;
        transaction.commit().await?;
        self.sync_users(caller, &former).await
    }

    /// Ghi nhóm trong `transaction`; trả dòng nhóm và những người có thành viên đổi.
    async fn save_group(
        &self,
        transaction: &DatabaseTransaction,
        caller: &ScimCaller,
        existing: Option<group::Model>,
        input: &GroupInput,
    ) -> Result<(group::Model, BTreeSet<Uuid>), ScimError> {
        let tenant = &caller.tenant;
        let own_id = existing.as_ref().map(|row| row.id);
        let key = input.display_name.to_lowercase();
        let mut taken = group::Entity::find()
            .filter(group::Column::DisplayNameKey.eq(key.clone()))
            .scoped(tenant);
        if let Some(own) = own_id {
            taken = taken.filter(group::Column::Id.ne(own));
        }
        if taken.one(transaction).await?.is_some() {
            return Err(ScimError::uniqueness(format!(
                "displayName `{}` is already used by another Group.",
                input.display_name
            )));
        }
        if let Some(external) = &input.external_id {
            let mut taken = group::Entity::find()
                .filter(group::Column::ExternalId.eq(external.clone()))
                .scoped(tenant);
            if let Some(own) = own_id {
                taken = taken.filter(group::Column::Id.ne(own));
            }
            if taken.one(transaction).await?.is_some() {
                return Err(ScimError::uniqueness(format!(
                    "externalId `{external}` is already used by another Group."
                )));
            }
        }
        // Thành viên phải là User SCIM của khách này.
        if !input.members.is_empty() {
            let known: BTreeSet<Uuid> = scim_user::Entity::find()
                .filter(scim_user::Column::Id.is_in(input.members.iter().copied()))
                .scoped(tenant)
                .all(transaction)
                .await?
                .into_iter()
                .map(|row| row.id)
                .collect();
            if let Some(unknown) = input.members.difference(&known).next() {
                return Err(ScimError::invalid_value(format!(
                    "`members`: no User of this tenant has the id `{unknown}`."
                )));
            }
        }
        let now = now();
        let row = group::Model {
            tenant_id: tenant.id().as_uuid(),
            id: own_id.unwrap_or_else(Uuid::now_v7),
            display_name: input.display_name.clone(),
            display_name_key: key,
            external_id: input.external_id.clone(),
            external_key: input.external_id.as_ref().map(|external| external.to_lowercase()),
            created_at: existing.as_ref().map_or(now, |row| row.created_at),
            updated_at: now,
        };
        let active = group::ActiveModel::from(row.clone());
        let saved = match &existing {
            Some(_) => active.reset_all().update(transaction).await,
            None => active.insert(transaction).await,
        };
        saved.map_err(unique_to_scim)?;
        let before: BTreeSet<Uuid> = member::Entity::find()
            .filter(member::Column::GroupId.eq(row.id))
            .scoped(tenant)
            .all(transaction)
            .await?
            .into_iter()
            .map(|row| row.user_id)
            .collect();
        let added: Vec<Uuid> = input.members.difference(&before).copied().collect();
        let removed: Vec<Uuid> = before.difference(&input.members).copied().collect();
        // Thêm / bớt thành viên là cấp / gỡ role theo luật của nhóm: kiểm TRƯỚC khi ghi, để token
        // quyền thấp không để lại thành viên "chờ" mà lần đồng bộ sau (của người khác) biến thành
        // quyền. Đổi tên nhóm không đổi gì (luật gắn ID nhóm).
        if !added.is_empty() {
            self.require_group_grantable(transaction, caller, row.id, true).await?;
        }
        if !removed.is_empty() {
            self.require_group_grantable(transaction, caller, row.id, false).await?;
        }
        if !removed.is_empty() {
            member::Entity::delete_many()
                .filter(member::Column::GroupId.eq(row.id))
                .filter(member::Column::UserId.is_in(removed.iter().copied()))
                .scoped(tenant)
                .exec(transaction)
                .await?;
        }
        for user_id in &added {
            member::ActiveModel {
                tenant_id: Set(tenant.id().as_uuid()),
                id: Set(Uuid::now_v7()),
                group_id: Set(row.id),
                user_id: Set(*user_id),
                created_at: Set(now),
            }
            .insert(transaction)
            .await
            .map_err(unique_to_scim)?;
        }
        let subject = group_subject(row.id);
        match &existing {
            None => {
                let event = events::ScimGroupCreated {
                    before: None,
                    after: Some(
                        json!({"id": row.id, "display_name": row.display_name, "members": added}),
                    ),
                };
                self.outbox.record(transaction, tenant, caller.actor(), subject, &event).await?;
            }
            Some(previous)
                if previous.display_name != row.display_name
                    || previous.external_id != row.external_id
                    || !added.is_empty()
                    || !removed.is_empty() =>
            {
                let event = events::ScimGroupUpdated {
                    before: Some(json!({"display_name": previous.display_name})),
                    after: Some(
                        json!({"display_name": row.display_name, "added": added, "removed": removed}),
                    ),
                };
                self.outbox.record(transaction, tenant, caller.actor(), subject, &event).await?;
            }
            Some(_) => {}
        }
        let changed = added.into_iter().chain(removed).collect();
        Ok((row, changed))
    }

    /// Người tạo token của `caller` cấp (`give`) hay gỡ được mọi (nút, role) mà luật ánh xạ của
    /// nhóm `group_id` cấp không (D-20261125-scim-grant-cover); không thì 403.
    async fn require_group_grantable(
        &self,
        transaction: &DatabaseTransaction,
        caller: &ScimCaller,
        group_id: Uuid,
        give: bool,
    ) -> Result<(), ScimError> {
        let tenant = &caller.tenant;
        let pairs: Vec<(Uuid, Uuid)> = mapping::Entity::find()
            .filter(mapping::Column::GroupId.eq(group_id))
            .scoped(tenant)
            .all(transaction)
            .await?
            .into_iter()
            .map(|row| (row.org_id, row.role_id))
            .collect();
        if pairs.is_empty() {
            return Ok(());
        }
        let authority = self.authority(caller).await?;
        let (given, taken) = if give { (pairs, Vec::new()) } else { (Vec::new(), pairs) };
        crate::role::require_grantable(transaction, tenant, &authority, &given, &taken).await?;
        Ok(())
    }

    /// Sau một lần ghi nhóm `group_id`: đồng bộ người có thành viên đổi (`changed`), cùng tối đa
    /// [`PENDING_MAX`] thành viên còn "chờ" (thiếu role mà luật của nhóm cấp — lần đồng bộ trước lỗi),
    /// ưu tiên người request nêu tên (`named`). Hệ thống danh tính gửi lại đúng request cũ thì không
    /// còn ai đổi, nhưng người chờ được nêu tên vẫn được đồng bộ lại. Trả lỗi chỉ khi người đổi hay
    /// người được nêu tên lỗi; người chờ cũ khác lỗi (vd vẫn hết chỗ) chỉ ghi log — không làm hỏng
    /// request không liên quan tới họ (D-20261125-scim-sync-report).
    async fn sync_group_members(
        &self,
        caller: &ScimCaller,
        group_id: Uuid,
        changed: BTreeSet<Uuid>,
        named: &BTreeSet<Uuid>,
    ) -> Result<(), ScimError> {
        let pending = pending_members(&caller.tenant, group_id).await?;
        let (first, rest): (Vec<Uuid>, Vec<Uuid>) =
            pending.into_iter().partition(|user_id| named.contains(user_id));
        let pending: BTreeSet<Uuid> = first.into_iter().chain(rest).take(PENDING_MAX).collect();
        let mut must = changed.clone();
        must.extend(pending.intersection(named).copied());
        let mut users = changed;
        users.extend(pending);
        if users.is_empty() {
            return Ok(());
        }
        let authority = self.authority(caller).await?;
        let report = self.sync_in(&caller.tenant, caller.actor(), &authority, &users).await;
        match report.failed.into_iter().find(|(user_id, _)| must.contains(user_id)) {
            Some((_, error)) => Err(ScimError::from(error)),
            None => Ok(()),
        }
    }

    /// Đồng bộ membership do SCIM cấp của những người `users` theo nhóm hiện tại của họ.
    pub(crate) async fn sync_users(
        &self,
        caller: &ScimCaller,
        users: &BTreeSet<Uuid>,
    ) -> Result<(), ScimError> {
        if users.is_empty() {
            return Ok(());
        }
        let authority = self.authority(caller).await?;
        let report = self.sync_in(&caller.tenant, caller.actor(), &authority, users).await;
        // Đã đồng bộ mọi người được; còn lỗi thì báo lỗi đầu để hệ thống danh tính gửi lại. Gửi lại
        // đồng bộ lại người còn chờ (`pending_members`) dù thành viên nhóm không đổi thêm.
        match report.failed.into_iter().next() {
            Some((_, error)) => Err(ScimError::from(error)),
            None => Ok(()),
        }
    }

    /// Như [`sync_users`](Self::sync_users), cho bên gọi ngoài SCIM (đổi luật ánh xạ).
    /// `authority`: người chịu trách nhiệm — chỉ role người đó cấp / gỡ được mới được thêm / gỡ.
    /// Đồng bộ MỌI người dù có người lỗi (vd vượt hạn mức số nhân sự): một người lỗi không để những
    /// người sau thành "chờ"; trả ai đã đồng bộ xong và ai lỗi (D-20261125-scim-sync-report).
    pub(crate) async fn sync_in(
        &self,
        tenant: &TenantContext,
        actor: autoxide_kernel::event::Actor,
        authority: &autoxide_kernel::access::Principal,
        users: &BTreeSet<Uuid>,
    ) -> SyncReport {
        let mut report = SyncReport::default();
        for user_id in users {
            let synced = async {
                let wanted = wanted_grants(tenant, *user_id).await?;
                self.roles.sync_scim_grants(tenant, actor, authority, *user_id, wanted).await
            };
            match synced.await {
                Ok(_) => report.synced.push(*user_id),
                Err(error) => {
                    // `error` đầy đủ chỉ vào log (kể cả lỗi nội bộ); ra ngoài chỉ có mã và câu chung.
                    tracing::warn!(user_id = %user_id, code = error.code(), %error, "SCIM membership sync failed for a user");
                    report.failed.push((*user_id, error));
                }
            }
        }
        report
    }
}

/// Kết quả đồng bộ membership của một nhóm người: ai xong, ai lỗi (kèm lỗi).
#[derive(Debug, Default)]
pub(crate) struct SyncReport {
    pub synced: Vec<Uuid>,
    pub failed: Vec<(Uuid, autoxide_kernel::error::Error)>,
}

/// Thành viên của nhóm `group_id` còn thiếu một (nút, role) mà luật của nhóm cấp — lần đồng bộ trước
/// của họ lỗi (vượt hạn mức…) hay bị bỏ qua vì người chịu trách nhiệm không cấp được. Mỗi luật một câu;
/// đọc qua kết nối ghi (ngay sau commit, read replica có thể trễ).
async fn pending_members(
    tenant: &TenantContext,
    group_id: Uuid,
) -> Result<BTreeSet<Uuid>, ScimError> {
    use crate::role::entity::user_role;
    let connection = tenant.database().write();
    let root = autoxide_kernel::tenancy::root_of(tenant.id()).as_uuid();
    let rules = mapping::Entity::find()
        .filter(mapping::Column::GroupId.eq(group_id))
        .scoped(tenant)
        .all(connection)
        .await?;
    let mut pending = BTreeSet::new();
    for rule in rules {
        let holders = Query::select()
            .column(user_role::Column::UserId)
            .from(user_role::Entity)
            .and_where(Expr::col(user_role::Column::TenantId).eq(tenant.id().as_uuid()))
            // Dòng cũ (trước bước 34) để `org_id` NULL nghĩa là nút gốc.
            .and_where(if rule.org_id == root {
                Expr::col(user_role::Column::OrgId)
                    .eq(rule.org_id)
                    .or(Expr::col(user_role::Column::OrgId).is_null())
            } else {
                Expr::col(user_role::Column::OrgId).eq(rule.org_id)
            })
            .and_where(Expr::col(user_role::Column::RoleId).eq(rule.role_id))
            .to_owned();
        let missing = member::Entity::find()
            .filter(member::Column::GroupId.eq(group_id))
            .filter(Expr::col((member::Entity, member::Column::UserId)).not_in_subquery(holders))
            .scoped(tenant)
            .all(connection)
            .await?;
        pending.extend(missing.into_iter().map(|row| row.user_id));
    }
    Ok(pending)
}

/// (Nút, role) người `user_id` phải giữ theo các nhóm của họ và luật ánh xạ.
async fn wanted_grants(
    tenant: &TenantContext,
    user_id: Uuid,
) -> Result<BTreeSet<(Uuid, Uuid)>, autoxide_kernel::error::Error> {
    // Kết nối GHI: đọc ngay sau commit nhóm / luật, read replica có thể trễ — thấy người đã bị gỡ
    // khỏi nhóm vẫn còn thì không gỡ role (D-20261125-scim-sync-report).
    let read = tenant.database().write();
    let groups = Query::select()
        .column(member::Column::GroupId)
        .from(member::Entity)
        .and_where(Expr::col(member::Column::TenantId).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(member::Column::UserId).eq(user_id))
        .to_owned();
    Ok(mapping::Entity::find()
        .filter(Expr::col((mapping::Entity, mapping::Column::GroupId)).in_subquery(groups))
        .scoped(tenant)
        .all(read)
        .await?
        .into_iter()
        .map(|row| (row.org_id, row.role_id))
        .collect())
}

pub(crate) fn group_subject(id: Uuid) -> Option<String> {
    Some(format!("auth.scim_group:{id}"))
}

fn unique_to_scim(error: sea_orm::DbErr) -> ScimError {
    match error.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => {
            ScimError::uniqueness("displayName or externalId is already used by another Group.")
        }
        _ => ScimError::from(error),
    }
}

/// Điều kiện SQL cho bộ lọc `Group`: `displayName`, `externalId`, `id`, `members` (`members.value`,
/// chỉ `eq`).
fn group_condition(filter: &Filter, tenant: &TenantContext) -> Result<Expr, ScimError> {
    let text = |attribute: &str, value: &Literal| match value {
        Literal::Text(text) => Ok(text.clone()),
        _ => {
            Err(ScimError::invalid_filter(format!("`{attribute}` compares with a quoted string.")))
        }
    };
    Ok(match filter {
        Filter::And(left, right) => {
            group_condition(left, tenant)?.and(group_condition(right, tenant)?)
        }
        Filter::Or(left, right) => {
            group_condition(left, tenant)?.or(group_condition(right, tenant)?)
        }
        Filter::Present { attribute } => match attribute.as_str() {
            "externalid" => Expr::col(group::Column::ExternalId).is_not_null(),
            "displayname" | "id" => Expr::val(1).eq(1),
            other => return Err(unknown_attribute(other)),
        },
        Filter::Compare { attribute, op, value } => match attribute.as_str() {
            "displayname" => text_condition(
                group::Column::DisplayNameKey,
                *op,
                &text(attribute, value)?.to_lowercase(),
            ),
            "externalid" => {
                let wanted = text(attribute, value)?;
                match op {
                    Op::Eq => Expr::col(group::Column::ExternalId).eq(wanted),
                    Op::Ne => Expr::col(group::Column::ExternalId).ne(wanted),
                    _ => text_condition(group::Column::ExternalKey, *op, &wanted.to_lowercase()),
                }
            }
            "id" => match (Uuid::try_parse(&text(attribute, value)?), op) {
                (Ok(id), Op::Eq) => Expr::col(group::Column::Id).eq(id),
                (Ok(id), Op::Ne) => Expr::col(group::Column::Id).ne(id),
                (Err(_), Op::Ne) => Expr::val(1).eq(1),
                (_, Op::Eq) => Expr::val(1).eq(0),
                _ => return Err(ScimError::invalid_filter("`id` supports eq and ne only.")),
            },
            "members" | "members.value" => {
                if *op != Op::Eq {
                    return Err(ScimError::invalid_filter("`members` supports eq only."));
                }
                let Ok(user_id) = Uuid::try_parse(&text(attribute, value)?) else {
                    return Ok(Expr::val(1).eq(0));
                };
                let groups = Query::select()
                    .column(member::Column::GroupId)
                    .from(member::Entity)
                    .and_where(Expr::col(member::Column::TenantId).eq(tenant.id().as_uuid()))
                    .and_where(Expr::col(member::Column::UserId).eq(user_id))
                    .to_owned();
                Expr::col((group::Entity, group::Column::Id)).in_subquery(groups)
            }
            other => return Err(unknown_attribute(other)),
        },
    })
}

fn unknown_attribute(attribute: &str) -> ScimError {
    ScimError::invalid_filter(format!(
        "Cannot filter Groups by `{attribute}`; use displayName, externalId, id or members."
    ))
}
