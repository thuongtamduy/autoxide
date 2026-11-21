//! Đặt, gỡ, đọc rào quyền; áp rào lúc dựng `Principal`.

use autoxide_kernel::time::Timestamp;
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use autoxide_kernel::access::{PermissionCatalog, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Delivered, EventHandler, Outbox, Subscription};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{DatabaseRegistry, TenancyMode, TenantContext, TenantId};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde_json::json;
use uuid::Uuid;

use super::entity;
use crate::events;
use crate::permissions::{GUARDRAILS_MANAGE, ROLES_READ};

/// Giữ rào đã tính của một khách bao lâu (máy chủ khác thấy rào mới trong khoảng này).
pub(crate) const CACHE_SECONDS: u64 = 30;

/// Một rào: khách bị rào, khách đặt, permission tối đa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GuardrailView {
    pub tenant_id: Uuid,
    pub set_by_tenant_id: Uuid,
    pub max_permissions: Vec<String>,
    pub updated_by: Uuid,
    pub updated_at: Timestamp,
}

impl From<entity::Model> for GuardrailView {
    fn from(row: entity::Model) -> Self {
        GuardrailView {
            tenant_id: row.tenant_id,
            set_by_tenant_id: row.set_by_tenant_id,
            max_permissions: names(&row.max_permissions),
            updated_by: row.updated_by,
            updated_at: row.updated_at,
        }
    }
}

fn names(stored: &str) -> Vec<String> {
    stored.split_whitespace().map(str::to_owned).collect()
}

type Allowed = Option<Arc<BTreeSet<String>>>;

pub(crate) struct GuardrailService {
    databases: DatabaseRegistry,
    catalog: PermissionCatalog,
    outbox: Outbox,
    // `Mutex` của std: giữ rất ngắn, không qua `.await`.
    cache: Mutex<HashMap<TenantId, (Instant, Allowed)>>,
}

impl GuardrailService {
    pub(crate) fn new(
        databases: DatabaseRegistry,
        catalog: PermissionCatalog,
        outbox: Outbox,
    ) -> Self {
        GuardrailService { databases, catalog, outbox, cache: Mutex::default() }
    }

    /// Database nền tảng, nơi lưu rào.
    fn platform(&self) -> TenantContext {
        self.databases.default_tenant()
    }

    /// Thu `principal` về trong rào của khách nó đang làm việc.
    pub(crate) async fn apply(&self, principal: Principal) -> Result<Principal, Error> {
        let allowed = self.allowed(principal.tenant_id()).await?;
        Ok(principal.within(allowed.as_deref(), &self.catalog))
    }

    /// Permission tối đa của khách `tenant` (giao mọi rào trên đường từ gốc); `None`: không rào.
    pub(crate) async fn allowed(&self, tenant: TenantId) -> Result<Allowed, Error> {
        // Một khách thì không có khách con: không truy vấn gì.
        if self.databases.mode() == TenancyMode::Single {
            return Ok(None);
        }
        let ttl = Duration::from_secs(CACHE_SECONDS);
        if let Some((at, allowed)) =
            self.cache.lock().unwrap_or_else(PoisonError::into_inner).get(&tenant)
            && at.elapsed() < ttl
        {
            return Ok(allowed.clone());
        }
        let rails = self.rails_over(tenant).await?;
        let allowed = intersect(&rails);
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(tenant, (Instant::now(), allowed.clone()));
        Ok(allowed)
    }

    /// Mọi rào áp cho khách `tenant`: đặt cho chính nó hay cho một khách cấp trên của nó.
    async fn rails_over(&self, tenant: TenantId) -> Result<Vec<entity::Model>, Error> {
        self.rails_over_in(self.platform().database().read(), tenant).await
    }

    /// Như [`rails_over`](Self::rails_over), đọc qua `connection` (vd transaction đang mở).
    async fn rails_over_in<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: TenantId,
    ) -> Result<Vec<entity::Model>, Error> {
        let mut chain = vec![tenant.as_uuid()];
        chain.extend(self.databases.tenant_ancestors(tenant).await?.iter().map(TenantId::as_uuid));
        Ok(entity::Entity::find()
            .filter(entity::Column::TenantId.is_in(chain))
            .order_by_asc(entity::Column::Id)
            .all(connection)
            .await?)
    }

    /// Tách khách: khách mới nhận bản sao mọi rào đang áp cho khách nguồn (rào đặt cho chính nó và
    /// cho tổ tiên của nó), giữ nguyên khách đặt rào. Không có bước này, khách mới tách ra làm
    /// khách gốc sẽ thoát mọi rào. Chạy lại được: cặp đã có thì giữ nguyên.
    pub(crate) async fn carry_on_split(&self, source: Uuid, target: Uuid) -> Result<(), Error> {
        let platform = self.platform();
        self.carry_on_split_in(platform.database().write(), source, target).await
    }

    /// Như [`carry_on_split`](Self::carry_on_split), ghi qua `connection` — transaction của bước
    /// chuyển khi tách (`SwitchHook`), để rào có hiệu lực cùng lúc khách mới hoạt động.
    pub(crate) async fn carry_on_split_in<C: ConnectionTrait>(
        &self,
        connection: &C,
        source: Uuid,
        target: Uuid,
    ) -> Result<(), Error> {
        let rails = self.rails_over_in(connection, TenantId::from_uuid(source)).await?;
        for rail in rails {
            let copy = entity::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(target),
                set_by_tenant_id: Set(rail.set_by_tenant_id),
                max_permissions: Set(rail.max_permissions),
                updated_by: Set(rail.updated_by),
                updated_at: Set(Timestamp::now()),
            };
            entity::Entity::insert(copy)
                .on_conflict(
                    OnConflict::columns([entity::Column::TenantId, entity::Column::SetByTenantId])
                        .do_nothing()
                        .to_owned(),
                )
                .exec_without_returning(connection)
                .await?;
        }
        self.forget();
        tracing::info!(source = %source, target = %target, "tenant guardrails carried to a split tenant");
        Ok(())
    }

    fn forget(&self) {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner).clear();
    }

    /// Bên gọi làm việc ở chính khách của mình (không qua uỷ quyền / hỗ trợ) và có
    /// `auth.guardrails.manage`; `child` phải là khách cấp dưới của khách đó.
    async fn require_parent_of(&self, caller: &Principal, child: Uuid) -> Result<(), Error> {
        caller.require(&GUARDRAILS_MANAGE)?;
        if caller.operator_from().is_some()
            || !matches!(caller.access(), autoxide_kernel::access::AccessPath::Direct)
        {
            return Err(not_a_child(child));
        }
        let ancestors = self.databases.tenant_ancestors(TenantId::from_uuid(child)).await?;
        if ancestors.contains(&caller.tenant_id()) {
            return Ok(());
        }
        // Khách đã đặt rào luôn sửa / gỡ được rào của mình, kể cả khi khách bị rào đã tách ra khỏi
        // nhánh của nó (rào được chép theo khi tách, xem `carry_on_split`).
        let own = entity::Entity::find()
            .filter(entity::Column::TenantId.eq(child))
            .filter(entity::Column::SetByTenantId.eq(caller.tenant_id().as_uuid()))
            .one(self.platform().database().read())
            .await?;
        if own.is_some() { Ok(()) } else { Err(not_a_child(child)) }
    }

    /// Đặt (hay thay) rào của khách bên gọi cho khách `child`.
    pub(crate) async fn set(
        &self,
        tenant: &TenantContext,
        caller: &Principal,
        child: Uuid,
        permissions: Vec<String>,
    ) -> Result<GuardrailView, Error> {
        self.require_parent_of(caller, child).await?;
        let mut problems = Vec::new();
        for (index, name) in permissions.iter().enumerate() {
            if !self.catalog.contains(name) {
                problems.push(FieldError::new(
                    format!("max_permissions[{index}]"),
                    "unknown",
                    "is not a permission of this system",
                ));
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let permissions: BTreeSet<String> = permissions.into_iter().collect();
        let stored = permissions.iter().cloned().collect::<Vec<_>>().join(" ");
        let now = Timestamp::now();
        let set_by = caller.tenant_id().as_uuid();
        let row = entity::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(child),
            set_by_tenant_id: Set(set_by),
            max_permissions: Set(stored.clone()),
            updated_by: Set(caller.user_id()),
            updated_at: Set(now),
        };
        // Đặt lại cùng cặp (khách con, khách đặt) thì thay, không thêm dòng.
        entity::Entity::insert(row)
            .on_conflict(
                OnConflict::columns([entity::Column::TenantId, entity::Column::SetByTenantId])
                    .update_columns([
                        entity::Column::MaxPermissions,
                        entity::Column::UpdatedBy,
                        entity::Column::UpdatedAt,
                    ])
                    .to_owned(),
            )
            .exec_without_returning(self.platform().database().write())
            .await?;
        self.forget();
        let after = json!({"tenant_id": child, "max_permissions": permissions});
        let event = events::GuardrailSet { before: None, after: Some(after) };
        self.record(tenant, caller, child, &event).await?;
        tracing::info!(tenant_id = %child, set_by = %set_by, "tenant guardrail set");
        Ok(GuardrailView {
            tenant_id: child,
            set_by_tenant_id: set_by,
            max_permissions: permissions.into_iter().collect(),
            updated_by: caller.user_id(),
            updated_at: now,
        })
    }

    /// Gỡ rào khách bên gọi đã đặt cho `child`. Không có thì `auth.guardrail_not_found`.
    pub(crate) async fn remove(
        &self,
        tenant: &TenantContext,
        caller: &Principal,
        child: Uuid,
    ) -> Result<(), Error> {
        self.require_parent_of(caller, child).await?;
        let deleted = entity::Entity::delete_many()
            .filter(entity::Column::TenantId.eq(child))
            .filter(entity::Column::SetByTenantId.eq(caller.tenant_id().as_uuid()))
            .exec(self.platform().database().write())
            .await?;
        if deleted.rows_affected == 0 {
            return Err(Error::not_found(
                "auth.guardrail_not_found",
                "This tenant has set no guardrail for the tenant `{tenant}`.",
            )
            .with_param("tenant", child));
        }
        self.forget();
        let event =
            events::GuardrailRemoved { before: Some(json!({"tenant_id": child})), after: None };
        self.record(tenant, caller, child, &event).await
    }

    /// Một trang rào khách bên gọi đã đặt cho khách cấp dưới.
    pub(crate) async fn list_set(
        &self,
        caller: &Principal,
        page: &PageRequest,
    ) -> Result<Page<GuardrailView>, Error> {
        caller.require(&GUARDRAILS_MANAGE)?;
        let select = entity::Entity::find()
            .filter(entity::Column::SetByTenantId.eq(caller.tenant_id().as_uuid()));
        let read = self.platform();
        let page =
            fetch_page(select, entity::Column::Id, |row| row.id, page, read.database().read())
                .await?;
        Ok(page.map(GuardrailView::from))
    }

    /// Rào đang áp cho khách của bên gọi (của chính nó và của khách cấp trên) và permission tối đa
    /// sau khi giao; `None` khi không bị rào.
    pub(crate) async fn applied(
        &self,
        caller: &Principal,
    ) -> Result<(Vec<GuardrailView>, Option<Vec<String>>), Error> {
        caller.require(&ROLES_READ)?;
        let rails = self.rails_over(caller.tenant_id()).await?;
        let allowed = intersect(&rails).map(|set| set.iter().cloned().collect());
        Ok((rails.into_iter().map(GuardrailView::from).collect(), allowed))
    }

    /// Event ghi ở khách đặt rào (audit của khách đó thấy).
    async fn record<E: autoxide_kernel::event::Event + Sync>(
        &self,
        tenant: &TenantContext,
        caller: &Principal,
        child: Uuid,
        event: &E,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let subject = Some(format!("auth.guardrail:{child}"));
        self.outbox.record(&transaction, tenant, Actor::from(caller), subject, event).await?;
        transaction.commit().await?;
        Ok(())
    }
}

/// Giao các rào; `None` khi không có rào nào.
fn intersect(rails: &[entity::Model]) -> Allowed {
    let mut allowed: Option<BTreeSet<String>> = None;
    for rail in rails {
        let set: BTreeSet<String> = names(&rail.max_permissions).into_iter().collect();
        allowed = Some(match allowed {
            None => set,
            Some(current) => current.intersection(&set).cloned().collect(),
        });
    }
    allowed.map(Arc::new)
}

fn not_a_child(child: Uuid) -> Error {
    Error::forbidden(
        "auth.guardrail_not_a_child",
        "Only a tenant above `{tenant}` in the customer tree can set its guardrail, from its own \
         account (not while working in another tenant).",
    )
    .with_param("tenant", child)
}

/// Khách bị xoá hẳn: bỏ rào đặt cho nó và rào nó đặt.
const TENANT_DELETED: &str = "kernel.tenant_deleted";

/// Bắt đầu tách / gộp khách (kernel ghi trước khi chép dữ liệu).
const RELOCATION_STARTED: &str = "kernel.relocation_started";

/// `service`: có sau `Module::start` (bộ phát event dựng sau đó).
pub(crate) fn subscriptions(service: Option<Arc<GuardrailService>>) -> Vec<Subscription> {
    let mut subscriptions = vec![Subscription::new(TENANT_DELETED, Arc::new(ForgetDeletedTenant))];
    if let Some(service) = service {
        subscriptions.push(Subscription::new(RELOCATION_STARTED, Arc::new(CarryOnSplit(service))));
    }
    subscriptions
}

/// Hook đồng bộ của bước chuyển khi tách (D-20261119-relocation-switch-hook): chép rào sang khách
/// mới trong transaction mở khách mới — không có lúc nào khách mới chạy mà chưa có rào.
pub(crate) struct GuardrailSwitch(pub(crate) Arc<GuardrailService>);

#[async_trait::async_trait]
impl autoxide_kernel::tenancy::SwitchHook for GuardrailSwitch {
    async fn switch(
        &self,
        control: &sea_orm::DatabaseTransaction,
        relocation: &autoxide_kernel::tenancy::RelocationSwitch,
    ) -> Result<(), Error> {
        if relocation.kind != autoxide_kernel::tenancy::RelocationKind::Split {
            return Ok(());
        }
        let (source, target) = (relocation.source.as_uuid(), relocation.target.as_uuid());
        self.0.carry_on_split_in(control, source, target).await
    }
}

/// Lưới an toàn sau hook trên (chạy lại được, cặp đã có thì giữ): tách khách thì chép rào sang
/// khách mới. Gộp: khách bị gộp mất cùng rào của nó; dữ liệu của nó theo rào của khách nhận.
struct CarryOnSplit(Arc<GuardrailService>);

#[async_trait::async_trait]
impl EventHandler for CarryOnSplit {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        if event.payload["kind"].as_str() != Some("split") {
            return Ok(());
        }
        let id =
            |field: &str| event.payload[field].as_str().and_then(|raw| raw.parse::<Uuid>().ok());
        let (Some(source), Some(target)) = (id("source_tenant_id"), id("target_tenant_id")) else {
            return Err(Error::internal("a split event without source or target tenant"));
        };
        self.0.carry_on_split(source, target).await
    }
}

struct ForgetDeletedTenant;

#[async_trait::async_trait]
impl EventHandler for ForgetDeletedTenant {
    // Event ghi ở khách nền tảng, nơi lưu rào. Chạy lại được: xoá thêm lần nữa không đổi gì.
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(gone) =
            event.payload["tenant_id"].as_str().and_then(|raw| raw.parse::<Uuid>().ok())
        else {
            return Err(Error::internal("a `kernel.tenant_deleted` event without `tenant_id`"));
        };
        entity::Entity::delete_many()
            .filter(
                Condition::any()
                    .add(entity::Column::TenantId.eq(gone))
                    .add(entity::Column::SetByTenantId.eq(gone)),
            )
            .exec(event.tenant.database().write())
            .await?;
        Ok(())
    }
}
