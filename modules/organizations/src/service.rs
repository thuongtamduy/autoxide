//! Service của cây tổ chức: MỌI logic nằm ở đây; REST và MCP chỉ gọi vào.
//!
//! Cấu trúc cây (cha, độ sâu, bảng tổ tiên, cờ ghi được) do `OrgTree` của kernel giữ; bảng của
//! module giữ phần nghĩa (tên, loại, mã, trạng thái). Mỗi lệnh đổi cây làm cả hai trong CÙNG
//! transaction, cùng event của nó.
//!
//! Thứ tự khoá: lệnh đổi cây gọi `OrgTree::lock` (khoá dòng gốc của khách) TRƯỚC mọi kiểm tra khác
//! — kể cả `require_write_at` (khoá chia sẻ cùng dòng đó) — nên hai lệnh đổi cây cùng khách nối
//! đuôi nhau, không deadlock.
//!
//! Quyền theo độ sâu: danh sách lọc bằng `Principal::scope_condition` trên cột `id` (nút là bản
//! ghi của chính nó); một nút ngoài phạm vi báo như không có (404, không lộ tồn tại); nút đích do
//! người gọi tự nêu (cha của nút mới, cha mới khi dời) ngoài phạm vi thì 403 `org_out_of_scope`.

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::search::{self, fold};
use autoxide_kernel::tenancy::{OrgId, OrgTree, ScopeExt, TenantContext, root_immutable, root_of};
use autoxide_kernel::time::Timestamp;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, PaginatorTrait,
    QueryFilter, QueryOrder, Set, SqlErr,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{OrgDirectory, Unit, UnitStatus};
use crate::config::OrganizationsConfig;
use crate::entity::{unit, unit_type};
use crate::events::{self, Change};
use crate::permissions::{TYPES_MANAGE, UNITS_MANAGE, UNITS_READ, UNITS_WRITE};

/// Loại của nút gốc; dành riêng, không tạo hay xoá được.
pub(crate) const ROOT_TYPE: &str = "tenant";

const NAME_MAX: usize = 200;
const CODE_MAX: usize = 64;
const TYPE_NAME_MAX: usize = 100;
const DESCRIPTION_MAX: usize = 1000;
const REASON_MAX: usize = 500;

pub(crate) type UnitRow = unit::Model;
pub(crate) type UnitTypeRow = unit_type::Model;

/// Bộ lọc danh sách nút.
#[derive(Debug, Clone, Default)]
pub(crate) struct UnitFilter {
    pub parent_id: Option<Uuid>,
    pub unit_type: Option<String>,
    pub status: Option<String>,
    pub q: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct NewUnit {
    pub parent_id: Uuid,
    pub unit_type: String,
    pub name: String,
    pub code: Option<String>,
}

/// Thay đổi cho một nút; `None` giữ nguyên. `code: Some(None)` xoá mã.
#[derive(Debug, Clone, Default)]
pub(crate) struct UnitChanges {
    pub name: Option<String>,
    pub code: Option<Option<String>>,
}

#[derive(Debug, Clone)]
pub(crate) struct NewUnitType {
    pub code: String,
    pub name: String,
    pub description: String,
}

pub(crate) struct OrgService {
    pub(crate) outbox: Outbox,
    config: OrganizationsConfig,
    /// Cài đặt theo nút mọi module đã khai (bước 34 mục 4).
    pub(crate) catalog: autoxide_kernel::tenancy::OrgSettingCatalog,
    /// Kiểm địa chỉ của hồ sơ công khai (module `address`); `None`: app không có module đó.
    pub(crate) units: Option<std::sync::Arc<dyn autoxide_address::api::AdministrativeUnits>>,
    /// Hạn mức số nút theo gói (bước 35).
    pub(crate) entitlements: autoxide_kernel::entitlement::EntitlementsHandle,
}

impl OrgService {
    pub(crate) fn new(
        outbox: Outbox,
        config: OrganizationsConfig,
        catalog: autoxide_kernel::tenancy::OrgSettingCatalog,
    ) -> Self {
        OrgService { outbox, config, catalog, units: None, entitlements: Default::default() }
    }

    pub(crate) fn with_units(
        mut self,
        units: Option<std::sync::Arc<dyn autoxide_address::api::AdministrativeUnits>>,
    ) -> Self {
        self.units = units;
        self
    }

    /// Nút gốc của khách; tạo (cùng loại `tenant`) nếu còn thiếu. Chạy lại được, kể cả hai lời gọi
    /// cùng lúc: bên thua đọc lại dòng bên thắng đã ghi.
    pub(crate) async fn ensure_root(&self, tenant: &TenantContext) -> Result<UnitRow, Error> {
        let root = root_of(tenant.id()).as_uuid();
        let find = || unit::Entity::find_by_id(root).scoped(tenant);
        if let Some(row) = find().one(tenant.database().read()).await? {
            return Ok(row);
        }
        match self.create_root(tenant).await {
            Ok(row) => Ok(row),
            Err(error) => find().one(tenant.database().write()).await?.ok_or(error),
        }
    }

    async fn create_root(&self, tenant: &TenantContext) -> Result<UnitRow, Error> {
        let now = now();
        let transaction = tenant.database().begin_write().await?;
        OrgTree::insert_root(&transaction, tenant.id()).await?;
        let has_type = unit_type::Entity::find_by_id((tenant.id().as_uuid(), ROOT_TYPE.to_owned()))
            .scoped(tenant)
            .one(&transaction)
            .await?
            .is_some();
        if !has_type {
            unit_type::ActiveModel {
                tenant_id: Set(tenant.id().as_uuid()),
                code: Set(ROOT_TYPE.to_owned()),
                name: Set("Tenant".to_owned()),
                description: Set("The root of the organization tree: the tenant itself.".to_owned()),
                created_at: Set(now),
                updated_at: Set(now),
            }
            .insert(&transaction)
            .await?;
        }
        let name: String = tenant.name().chars().take(NAME_MAX).collect();
        let row = unit::ActiveModel {
            id: Set(root_of(tenant.id()).as_uuid()),
            tenant_id: Set(tenant.id().as_uuid()),
            parent_id: Set(None),
            unit_type: Set(ROOT_TYPE.to_owned()),
            name_folded: Set(fold(&name)),
            name: Set(name),
            code: Set(None),
            status: Set(UnitStatus::Active.as_str().to_owned()),
            status_reason: Set(None),
            created_by: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&transaction)
        .await?;
        let event = events::UnitCreated(Change { before: None, after: Some(snapshot(&row)) });
        self.outbox
            .record(&transaction, tenant, Actor::system(), events::unit_subject(row.id), &event)
            .await?;
        transaction.commit().await?;
        tracing::info!(tenant_id = %tenant.id(), "organization root created");
        Ok(row)
    }

    /// Một trang nút trong phạm vi `organizations.units.read` của người gọi.
    pub(crate) async fn list(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: &UnitFilter,
        page: &PageRequest,
    ) -> Result<Page<UnitRow>, Error> {
        self.ensure_root(tenant).await?;
        let scope = principal.scope_condition(&UNITS_READ, unit::Column::Id, None)?;
        let mut select = unit::Entity::find().filter(scope).scoped(tenant);
        if let Some(parent) = filter.parent_id {
            select = select.filter(unit::Column::ParentId.eq(parent));
        }
        if let Some(unit_type) = &filter.unit_type {
            select = select.filter(unit::Column::UnitType.eq(unit_type.as_str()));
        }
        if let Some(status) = &filter.status {
            if UnitStatus::parse(status).is_none() {
                return Err(Error::validation(vec![invalid_status("status")]));
            }
            select = select.filter(unit::Column::Status.eq(status.as_str()));
        }
        if let Some(q) = filter.q.as_deref().map(fold).filter(|q| !q.is_empty()) {
            select = select.filter(search::contains(unit::Column::NameFolded, &q));
        }
        fetch_page(select, unit::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Nút `id`; ngoài phạm vi đọc thì báo như không có.
    pub(crate) async fn get(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<UnitRow, Error> {
        self.ensure_root(tenant).await?;
        let read = tenant.database().read();
        let row = find(read, tenant, id).await?;
        hidden(principal.require_at(read, &UNITS_READ, at(id)).await, id)?;
        Ok(row)
    }

    /// Các nút trên `id`, gần nhất trước, gồm gốc.
    pub(crate) async fn ancestors(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Vec<UnitRow>, Error> {
        self.get(tenant, principal, id).await?;
        self.lineage(tenant, OrgId::from_uuid(id)).await
    }

    async fn lineage(&self, tenant: &TenantContext, id: OrgId) -> Result<Vec<UnitRow>, Error> {
        let read = tenant.database().read();
        let ids: Vec<Uuid> = OrgTree::ancestors(read, tenant.id(), id)
            .await?
            .into_iter()
            .skip(1)
            .map(|org| org.as_uuid())
            .collect();
        let rows = unit::Entity::find()
            .filter(unit::Column::Id.is_in(ids.clone()))
            .scoped(tenant)
            .all(read)
            .await?;
        Ok(ids.iter().filter_map(|id| rows.iter().find(|row| row.id == *id).cloned()).collect())
    }

    /// Tạo nút dưới `parent_id`: `active` khi người tạo có `organizations.units.manage` tại cha,
    /// không thì `pending` (chờ duyệt; chưa ghi dữ liệu được).
    pub(crate) async fn create(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewUnit,
    ) -> Result<UnitRow, Error> {
        // Hạn mức số nút (bước 35): giữ chỗ TRƯỚC transaction của module, lỗi thì trả lại. Giữ cả
        // chỗ chung lẫn chỗ theo loại nút (`organizations.units.<loại>`).
        let id = OrgId::from_uuid(Uuid::now_v7());
        let reference = unit_reference(id.as_uuid());
        let unit_type = input.unit_type.clone();
        self.hold(tenant, &unit_type, &reference).await?;
        match self.insert_unit(tenant, principal, input, id).await {
            Ok(row) => {
                self.keep(tenant, &unit_type, &reference).await?;
                Ok(row)
            }
            Err(error) => {
                self.give_back(tenant, &unit_type, &reference).await?;
                Err(error)
            }
        }
    }

    /// Giữ chỗ hạn mức cho một nút loại `unit_type`: chỗ chung rồi chỗ theo loại; loại hết chỗ thì
    /// trả lại chỗ chung.
    async fn hold(
        &self,
        tenant: &TenantContext,
        unit_type: &str,
        reference: &str,
    ) -> Result<(), Error> {
        let entitlements = &self.entitlements;
        entitlements.reserve(tenant.id(), &UNITS_LIMIT, 1, reference).await?;
        let part =
            entitlements.reserve_part(tenant.id(), &UNITS_LIMIT, unit_type, 1, reference).await;
        if let Err(error) = part {
            entitlements.release(tenant.id(), &UNITS_LIMIT, reference).await?;
            return Err(error);
        }
        Ok(())
    }

    /// Chỗ đã giữ thành vĩnh viễn (nút đã ghi).
    async fn keep(
        &self,
        tenant: &TenantContext,
        unit_type: &str,
        reference: &str,
    ) -> Result<(), Error> {
        self.entitlements.confirm(tenant.id(), &UNITS_LIMIT, reference).await?;
        self.entitlements.confirm_part(tenant.id(), &UNITS_LIMIT, unit_type, reference).await
    }

    /// Trả lại chỗ (ghi hỏng, hay nút bị lưu trữ).
    async fn give_back(
        &self,
        tenant: &TenantContext,
        unit_type: &str,
        reference: &str,
    ) -> Result<(), Error> {
        self.entitlements.release(tenant.id(), &UNITS_LIMIT, reference).await?;
        self.entitlements.release_part(tenant.id(), &UNITS_LIMIT, unit_type, reference).await
    }

    async fn insert_unit(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewUnit,
        id: OrgId,
    ) -> Result<UnitRow, Error> {
        let (name, code) = check_unit(Some(&input.name), Some(input.code.as_deref()))?;
        let name = name.unwrap_or_default();
        let code = code.flatten();
        self.ensure_root(tenant).await?;
        let parent = OrgId::from_uuid(input.parent_id);
        let transaction = tenant.database().begin_write().await?;
        OrgTree::lock(&transaction, tenant.id()).await?;
        principal.require_write_at(&transaction, &UNITS_WRITE, at(input.parent_id)).await?;
        if input.unit_type == ROOT_TYPE {
            return Err(Error::validation(vec![FieldError::new(
                "unit_type",
                "reserved",
                "`tenant` is the type of the root unit only; choose another type",
            )]));
        }
        find_type(&transaction, tenant, &input.unit_type).await?;
        let approved =
            principal.require_at(&transaction, &UNITS_MANAGE, at(input.parent_id)).await.is_ok();
        OrgTree::insert(&transaction, tenant.id(), id, parent, self.config.max_depth).await?;
        let status = if approved { UnitStatus::Active } else { UnitStatus::Pending };
        if !approved {
            OrgTree::set_writable(&transaction, tenant.id(), id, false).await?;
        }
        let now = now();
        let row = unit::ActiveModel {
            id: Set(id.as_uuid()),
            tenant_id: Set(tenant.id().as_uuid()),
            parent_id: Set(Some(input.parent_id)),
            unit_type: Set(input.unit_type),
            name_folded: Set(fold(&name)),
            name: Set(name),
            code: Set(code.clone()),
            status: Set(status.as_str().to_owned()),
            status_reason: Set(None),
            created_by: Set(Some(principal.user_id())),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&transaction)
        .await
        .map_err(|error| code_conflict(error, code.as_deref()))?;
        let event = events::UnitCreated(Change { before: None, after: Some(snapshot(&row)) });
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::unit_subject(row.id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(unit_id = %row.id, status = status.as_str(), "organization unit created");
        Ok(row)
    }

    /// Đổi tên hay mã của nút `id`. `expected`: `updated_at` người gọi đã đọc (`If-Match`).
    pub(crate) async fn update(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        changes: UnitChanges,
        expected: Option<Timestamp>,
    ) -> Result<UnitRow, Error> {
        let (name, code) =
            check_unit(changes.name.as_deref(), changes.code.as_ref().map(|code| code.as_deref()))?;
        let transaction = tenant.database().begin_write().await?;
        let before = find(&transaction, tenant, id).await?;
        hidden(principal.require_at(&transaction, &UNITS_WRITE, at(id)).await, id)?;
        principal.require_write_at(&transaction, &UNITS_WRITE, at(id)).await?;
        let mut row = before.clone().into_active_model();
        if let Some(name) = &name {
            row.name_folded = Set(fold(name));
            row.name = Set(name.clone());
        }
        if let Some(code) = &code {
            row.code = Set(code.clone());
        }
        row.updated_at = Set(now());
        let mut update = unit::Entity::update(row).validate()?.scoped(tenant);
        if let Some(expected) = expected {
            update = update.filter(unit::Column::UpdatedAt.eq(expected));
        }
        let after = update.exec(&transaction).await.map_err(|error| match error {
            sea_orm::DbErr::RecordNotUpdated => autoxide_kernel::version::changed(),
            error => code_conflict(error, code.clone().flatten().as_deref()),
        })?;
        let change = Change { before: Some(snapshot(&before)), after: Some(snapshot(&after)) };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::unit_subject(id),
                &events::UnitUpdated(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Dời nút `id` (cùng cả nhánh) sang dưới `parent_id`. Cần `organizations.units.write` tại nút
    /// và tại cha mới; cả hai phải ghi được.
    pub(crate) async fn relocate(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        parent_id: Uuid,
    ) -> Result<UnitRow, Error> {
        let transaction = tenant.database().begin_write().await?;
        OrgTree::lock(&transaction, tenant.id()).await?;
        let before = find(&transaction, tenant, id).await?;
        hidden(principal.require_at(&transaction, &UNITS_WRITE, at(id)).await, id)?;
        principal.require_write_at(&transaction, &UNITS_WRITE, at(id)).await?;
        principal.require_write_at(&transaction, &UNITS_WRITE, at(parent_id)).await?;
        OrgTree::move_subtree(
            &transaction,
            tenant.id(),
            OrgId::from_uuid(id),
            OrgId::from_uuid(parent_id),
            self.config.max_depth,
        )
        .await?;
        let mut row = before.clone().into_active_model();
        row.parent_id = Set(Some(parent_id));
        row.updated_at = Set(now());
        let after = unit::Entity::update(row).validate()?.scoped(tenant).exec(&transaction).await?;
        let change = Change {
            before: Some(json!({"parent_id": before.parent_id})),
            after: Some(json!({"parent_id": after.parent_id})),
        };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::unit_subject(id),
                &events::UnitMoved(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Đổi trạng thái nút `id`. Chuyển hợp lệ: `pending → active`, `active ↔ suspended`,
    /// `active / suspended → archived`, `archived → active`. Gốc không đổi được.
    pub(crate) async fn change_status(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        status: &str,
        reason: Option<String>,
    ) -> Result<UnitRow, Error> {
        // Hạn mức số nút đếm nút không lưu trữ: bỏ lưu trữ giữ chỗ trước, lưu trữ trả lại sau.
        let reference = unit_reference(id);
        let archiving = status == UnitStatus::Archived.as_str();
        // Nút đang lưu trữ mà đổi sang trạng thái khác: lại được tính vào hạn mức.
        let revived = match archiving {
            true => None,
            false => find(tenant.database().read(), tenant, id)
                .await
                .ok()
                .filter(|row| row.status == UnitStatus::Archived.as_str())
                .map(|row| row.unit_type),
        };
        if let Some(unit_type) = &revived {
            self.hold(tenant, unit_type, &reference).await?;
        }
        let changed = self.set_status(tenant, principal, id, status, reason).await;
        match (&changed, &revived) {
            (Ok(_), Some(unit_type)) => self.keep(tenant, unit_type, &reference).await?,
            (Err(_), Some(unit_type)) => self.give_back(tenant, unit_type, &reference).await?,
            (Ok(row), None) if row.status == UnitStatus::Archived.as_str() => {
                self.give_back(tenant, &row.unit_type, &reference).await?;
            }
            _ => {}
        }
        changed
    }

    async fn set_status(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        status: &str,
        reason: Option<String>,
    ) -> Result<UnitRow, Error> {
        let mut problems = Vec::new();
        let target = UnitStatus::parse(status);
        if target.is_none() {
            problems.push(invalid_status("status"));
        }
        let reason =
            reason.map(|reason| reason.trim().to_owned()).filter(|reason| !reason.is_empty());
        if reason.as_ref().is_some_and(|reason| reason.chars().count() > REASON_MAX) {
            problems.push(too_long("reason", REASON_MAX));
        }
        let Some(target) = target.filter(|_| problems.is_empty()) else {
            return Err(Error::validation(problems));
        };
        let transaction = tenant.database().begin_write().await?;
        OrgTree::lock(&transaction, tenant.id()).await?;
        let before = find(&transaction, tenant, id).await?;
        hidden(principal.require_at(&transaction, &UNITS_READ, at(id)).await, id)?;
        principal.require_at(&transaction, &UNITS_MANAGE, at(id)).await?;
        if before.parent_id.is_none() {
            return Err(root_immutable());
        }
        let current = UnitStatus::parse(&before.status).unwrap_or(UnitStatus::Pending);
        let allowed = matches!(
            (current, target),
            (UnitStatus::Pending, UnitStatus::Active)
                | (UnitStatus::Active, UnitStatus::Suspended)
                | (UnitStatus::Suspended, UnitStatus::Active)
                | (UnitStatus::Active | UnitStatus::Suspended, UnitStatus::Archived)
                | (UnitStatus::Archived, UnitStatus::Active)
        );
        if !allowed {
            return Err(Error::conflict(
                "organizations.invalid_transition",
                "A unit cannot go from `{from}` to `{to}`. Allowed: pending → active, active ↔ \
                 suspended, active or suspended → archived, archived → active.",
            )
            .with_param("from", current.as_str())
            .with_param("to", target.as_str()));
        }
        OrgTree::set_writable(
            &transaction,
            tenant.id(),
            OrgId::from_uuid(id),
            target == UnitStatus::Active,
        )
        .await?;
        let mut row = before.clone().into_active_model();
        row.status = Set(target.as_str().to_owned());
        row.status_reason = Set(reason);
        row.updated_at = Set(now());
        let after = unit::Entity::update(row).validate()?.scoped(tenant).exec(&transaction).await?;
        let change = Change {
            before: Some(json!({"status": before.status})),
            after: Some(json!({"status": after.status, "reason": after.status_reason})),
        };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::unit_subject(id),
                &events::UnitStatusChanged(change),
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(unit_id = %id, status = target.as_str(), "organization unit status changed");
        Ok(after)
    }

    /// Mọi loại nút của khách, theo mã. Cần `organizations.units.read` ở bất kỳ độ sâu nào.
    pub(crate) async fn list_types(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<Vec<UnitTypeRow>, Error> {
        principal.require(&UNITS_READ)?;
        self.ensure_root(tenant).await?;
        Ok(unit_type::Entity::find()
            .scoped(tenant)
            .order_by_asc(unit_type::Column::Code)
            .all(tenant.database().read())
            .await?)
    }

    pub(crate) async fn create_type(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewUnitType,
    ) -> Result<UnitTypeRow, Error> {
        principal.require(&TYPES_MANAGE)?;
        let code = input.code.trim().to_owned();
        let (name, description) =
            check_type(Some(&code), Some(&input.name), Some(&input.description))?;
        self.ensure_root(tenant).await?;
        let now = now();
        let transaction = tenant.database().begin_write().await?;
        let row = unit_type::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            code: Set(code.clone()),
            name: Set(name.unwrap_or_default()),
            description: Set(description.unwrap_or_default()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&transaction)
        .await
        .map_err(|error| match error.sql_err() {
            Some(SqlErr::UniqueConstraintViolation(_)) => type_taken(&code),
            _ => Error::from(error),
        })?;
        let change = Change { before: None, after: Some(type_snapshot(&row)) };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::type_subject(&code),
                &events::UnitTypeChanged(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(row)
    }

    pub(crate) async fn update_type(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        code: &str,
        name: Option<String>,
        description: Option<String>,
    ) -> Result<UnitTypeRow, Error> {
        principal.require(&TYPES_MANAGE)?;
        let (name, description) = check_type(None, name.as_deref(), description.as_deref())?;
        self.ensure_root(tenant).await?;
        let transaction = tenant.database().begin_write().await?;
        let before = find_type(&transaction, tenant, code).await?;
        let mut row = before.clone().into_active_model();
        if let Some(name) = name {
            row.name = Set(name);
        }
        if let Some(description) = description {
            row.description = Set(description);
        }
        row.updated_at = Set(now());
        let after =
            unit_type::Entity::update(row).validate()?.scoped(tenant).exec(&transaction).await?;
        let change =
            Change { before: Some(type_snapshot(&before)), after: Some(type_snapshot(&after)) };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::type_subject(code),
                &events::UnitTypeChanged(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Xoá loại nút `code`; loại đang dùng → 409 `organizations.unit_type_in_use`.
    pub(crate) async fn delete_type(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        code: &str,
    ) -> Result<(), Error> {
        principal.require(&TYPES_MANAGE)?;
        if code == ROOT_TYPE {
            return Err(Error::conflict(
                "organizations.unit_type_reserved",
                "The unit type `tenant` belongs to the root unit and cannot be deleted.",
            ));
        }
        let transaction = tenant.database().begin_write().await?;
        let before = find_type(&transaction, tenant, code).await?;
        let used = unit::Entity::find()
            .filter(unit::Column::UnitType.eq(code))
            .scoped(tenant)
            .count(&transaction)
            .await?;
        if used > 0 {
            return Err(Error::conflict(
                "organizations.unit_type_in_use",
                "The unit type `{code}` is used by {count} units; change their type first.",
            )
            .with_param("code", code)
            .with_param("count", used));
        }
        unit_type::Entity::delete_many()
            .filter(unit_type::Column::Code.eq(code))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let change = Change { before: Some(type_snapshot(&before)), after: None };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::type_subject(code),
                &events::UnitTypeChanged(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(())
    }
}

/// Khách mới bắt đầu với nút gốc (gọi khi tạo khách; chạy lại được).
#[async_trait::async_trait]
impl autoxide_kernel::tenancy::TenantSeed for OrgService {
    async fn seed(&self, tenant: &TenantContext) -> Result<(), Error> {
        self.ensure_root(tenant).await.map(|_| ())
    }
}

#[async_trait::async_trait]
impl OrgDirectory for OrgService {
    async fn unit(&self, tenant: &TenantContext, id: OrgId) -> Result<Option<Unit>, Error> {
        let row = unit::Entity::find_by_id(id.as_uuid())
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        // Khách mặc định của bản cài một khách không qua bước tạo khách (seed) nên nút gốc có thể
        // chưa có: module khác hỏi nút gốc (vd `ledger` mở sổ) thì tạo, như `root()`.
        if row.is_none() && id == root_of(tenant.id()) {
            return Ok(Some(to_unit(&self.ensure_root(tenant).await?)));
        }
        Ok(row.map(|row| to_unit(&row)))
    }

    async fn units(&self, tenant: &TenantContext, ids: &[OrgId]) -> Result<Vec<Unit>, Error> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = unit::Entity::find()
            .filter(unit::Column::Id.is_in(ids.iter().map(|id| id.as_uuid())))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        let mut found: Vec<Unit> = rows.iter().map(to_unit).collect();
        // Như `unit`: nút gốc của khách mặc định tạo khi được hỏi tới lần đầu.
        let root = root_of(tenant.id());
        if ids.contains(&root) && !found.iter().any(|unit| unit.id == root) {
            found.push(to_unit(&self.ensure_root(tenant).await?));
        }
        Ok(found)
    }

    async fn root(&self, tenant: &TenantContext) -> Result<Unit, Error> {
        Ok(to_unit(&self.ensure_root(tenant).await?))
    }

    async fn ancestors(&self, tenant: &TenantContext, id: OrgId) -> Result<Vec<Unit>, Error> {
        Ok(self.lineage(tenant, id).await?.iter().map(to_unit).collect())
    }

    async fn is_within(
        &self,
        tenant: &TenantContext,
        id: OrgId,
        root: OrgId,
    ) -> Result<bool, Error> {
        OrgTree::is_within(tenant.database().read(), tenant.id(), id, &[root]).await
    }

    async fn setting(
        &self,
        tenant: &TenantContext,
        id: OrgId,
        key: &str,
    ) -> Result<Option<crate::api::EffectiveSetting>, Error> {
        if self.catalog.get(key).is_none() {
            return Err(crate::settings::setting_unknown(key));
        }
        Ok(self.resolve_settings(tenant, id, Some(key)).await?.into_iter().next())
    }

    async fn opening_hours(
        &self,
        tenant: &TenantContext,
        id: OrgId,
    ) -> Result<Option<Vec<crate::api::OpeningRange>>, Error> {
        self.opening_ranges(tenant, id.as_uuid()).await
    }
}

pub(crate) fn to_unit(row: &UnitRow) -> Unit {
    Unit {
        id: OrgId::from_uuid(row.id),
        parent_id: row.parent_id.map(OrgId::from_uuid),
        unit_type: row.unit_type.clone(),
        name: row.name.clone(),
        code: row.code.clone(),
        status: UnitStatus::parse(&row.status).unwrap_or(UnitStatus::Pending),
    }
}

fn at(id: Uuid) -> OrgTarget {
    OrgTarget::new(OrgId::from_uuid(id), None)
}

async fn find<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<UnitRow, Error> {
    unit::Entity::find_by_id(id).scoped(tenant).one(connection).await?.ok_or_else(|| not_found(id))
}

async fn find_type<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    code: &str,
) -> Result<UnitTypeRow, Error> {
    unit_type::Entity::find_by_id((tenant.id().as_uuid(), code.to_owned()))
        .scoped(tenant)
        .one(connection)
        .await?
        .ok_or_else(|| {
            Error::not_found(
                "organizations.unit_type_not_found",
                "There is no unit type `{code}`; see `GET /api/v1/organizations/unit-types`.",
            )
            .with_param("code", code)
        })
}

/// Nút ngoài phạm vi báo như không có — không lộ nút tồn tại.
fn hidden(result: Result<(), Error>, id: Uuid) -> Result<(), Error> {
    result.map_err(|error| if error.code() == "org_out_of_scope" { not_found(id) } else { error })
}

pub(crate) fn not_found(id: Uuid) -> Error {
    Error::not_found("organizations.unit_not_found", "There is no organization unit `{id}`.")
        .with_param("id", id)
}

fn type_taken(code: &str) -> Error {
    Error::conflict(
        "organizations.unit_type_taken",
        "The unit type `{code}` already exists. Choose another code.",
    )
    .with_param("code", code)
}

fn code_conflict(error: sea_orm::DbErr, code: Option<&str>) -> Error {
    match error.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => Error::conflict(
            "organizations.unit_code_taken",
            "The code `{code}` is already used by another unit. Choose another code.",
        )
        .with_param("code", code.unwrap_or_default()),
        _ => Error::from(error),
    }
}

fn invalid_status(field: &str) -> FieldError {
    FieldError::new(field, "one_of", "must be one of `pending`, `active`, `suspended`, `archived`")
}

fn too_long(field: &str, max: usize) -> FieldError {
    FieldError::new(field, "length", "must be at most {max} characters long").with_param("max", max)
}

fn length(field: &str, max: usize) -> FieldError {
    FieldError::new(field, "length", "must be 1 to {max} characters, not only spaces")
        .with_param("max", max)
}

/// Tên và mã đã kiểm: `None` là không đổi; mã `Some(None)` là xoá.
type CheckedUnit = (Option<String>, Option<Option<String>>);

/// Kiểm tên (1–200 ký tự) và mã (1–64 ký tự, hay `None` để xoá); trường `None` bỏ qua. Mọi lỗi
/// báo một lần. Trả giá trị đã bỏ khoảng trắng hai đầu.
fn check_unit(name: Option<&str>, code: Option<Option<&str>>) -> Result<CheckedUnit, Error> {
    let mut problems = Vec::new();
    let name = name.map(str::trim);
    if name.is_some_and(|name| !(1..=NAME_MAX).contains(&name.chars().count())) {
        problems.push(length("name", NAME_MAX));
    }
    let code = code.map(|code| code.map(str::trim));
    if let Some(Some(code)) = code
        && !(1..=CODE_MAX).contains(&code.chars().count())
    {
        problems.push(length("code", CODE_MAX));
    }
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    Ok((name.map(str::to_owned), code.map(|code| code.map(str::to_owned))))
}

/// Mã loại nút: `[a-z][a-z0-9_]{0,31}`.
fn valid_type_code(code: &str) -> bool {
    let mut chars = code.chars();
    chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && code.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn check_type(
    code: Option<&str>,
    name: Option<&str>,
    description: Option<&str>,
) -> Result<(Option<String>, Option<String>), Error> {
    let mut problems = Vec::new();
    if let Some(code) = code {
        if !valid_type_code(code) {
            problems.push(FieldError::new(
                "code",
                "pattern",
                "must start with a lower-case letter, then up to 31 lower-case letters, digits or `_`",
            ));
        } else if code == ROOT_TYPE {
            problems.push(FieldError::new(
                "code",
                "reserved",
                "`tenant` is the type of the root unit",
            ));
        }
    }
    let name = name.map(str::trim);
    if name.is_some_and(|name| !(1..=TYPE_NAME_MAX).contains(&name.chars().count())) {
        problems.push(length("name", TYPE_NAME_MAX));
    }
    let description = description.map(str::trim);
    if description.is_some_and(|text| text.chars().count() > DESCRIPTION_MAX) {
        problems.push(too_long("description", DESCRIPTION_MAX));
    }
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    Ok((name.map(str::to_owned), description.map(str::to_owned)))
}

fn snapshot(row: &UnitRow) -> Value {
    json!({
        "parent_id": row.parent_id,
        "unit_type": row.unit_type,
        "name": row.name,
        "code": row.code,
        "status": row.status,
    })
}

fn type_snapshot(row: &UnitTypeRow) -> Value {
    json!({"code": row.code, "name": row.name, "description": row.description})
}

/// Bây giờ, cắt tới micro giây: hai database trả cùng một giá trị.
fn now() -> Timestamp {
    Timestamp::now()
}

/// Hạn mức số nút tổ chức (bước 35): mọi nút trừ gốc, trừ nút đã lưu trữ.
pub(crate) const UNITS_LIMIT: autoxide_kernel::entitlement::Limit =
    autoxide_kernel::entitlement::Limit::count(
        "organizations.units",
        "Organization units a tenant can have, besides its root (archived units do not count).",
    )
    // Gói đặt thêm được mức theo loại nút: `organizations.units.<loại>` (A-3.12).
    .per("unit type");

/// Mã giữ chỗ hạn mức của nút `id`.
fn unit_reference(id: Uuid) -> String {
    format!("organizations.unit:{id}")
}

/// Bộ đếm thật cho job đối soát của `plans`: số nút không lưu trữ, trừ gốc.
/// Phần hẹp kernel cần về cây tổ chức (`OrgNodes`): vd gắn tên miền với một cửa hàng. Nút lưu trữ
/// không còn dùng được.
#[async_trait::async_trait]
impl autoxide_kernel::tenancy::OrgNodes for OrgService {
    async fn has_node(&self, tenant: &TenantContext, org: OrgId) -> Result<bool, Error> {
        let found = unit::Entity::find_by_id(org.as_uuid())
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        Ok(found.is_some_and(|unit| unit.status != UnitStatus::Archived.as_str()))
    }
}

pub(crate) struct UnitCounter;

#[async_trait::async_trait]
impl autoxide_kernel::entitlement::UsageCounter for UnitCounter {
    async fn count(&self, tenant: &TenantContext) -> Result<i64, Error> {
        use sea_orm::PaginatorTrait;
        let found = unit::Entity::find()
            .filter(unit::Column::ParentId.is_not_null())
            .filter(unit::Column::Status.ne(UnitStatus::Archived.as_str()))
            .scoped(tenant)
            .count(tenant.database().read())
            .await?;
        Ok(i64::try_from(found).unwrap_or(i64::MAX))
    }

    async fn count_parts(
        &self,
        tenant: &TenantContext,
    ) -> Result<std::collections::BTreeMap<String, i64>, Error> {
        use sea_orm::QuerySelect;
        // Một câu `GROUP BY` theo loại, không tải từng nút.
        let counts: Vec<(String, i64)> = unit::Entity::find()
            .select_only()
            .column(unit::Column::UnitType)
            .column_as(unit::Column::Id.count(), "units")
            .filter(unit::Column::ParentId.is_not_null())
            .filter(unit::Column::Status.ne(UnitStatus::Archived.as_str()))
            .scoped(tenant)
            .group_by(unit::Column::UnitType)
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        let parts: std::collections::BTreeMap<String, i64> = counts.into_iter().collect();
        Ok(parts)
    }
}
