//! Service của `promotions`: MỌI logic nằm ở đây và ở các file con. Handler REST và tool MCP chỉ
//! lấy dữ liệu vào, kiểm permission, gọi service — nên hai đường cho cùng một kết quả (luật kiến
//! trúc 8).
//!
//! Hàm chạm database của khách nhận `&TenantContext` và lọc bằng `.scoped(tenant)` (bước 33); dữ
//! liệu luật dùng chung đọc ở database điều khiển (`DatabaseRegistry::control`).

use autoxide_kernel::access::{OrgTarget, Permission, Principal, Scope};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Outbox;
use autoxide_kernel::tenancy::{DatabaseRegistry, OrgId, TenantContext, root_of};
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::{ConnectionTrait, DbErr, SqlErr};
use uuid::Uuid;

use std::sync::Arc;

use autoxide_catalog::api::Catalog;
use autoxide_kernel::entitlement::EntitlementsHandle;
use autoxide_kernel::i18n::Translations;
use autoxide_organizations::api::OrgDirectory;
use chrono_tz::Tz;

use crate::compiled::CompiledCache;
use crate::config::PromotionsConfig;
use crate::legal::LegalCache;

/// Mọi việc của module. Không giữ database của khách: database đi theo `TenantContext` của từng
/// lời gọi; `databases` chỉ để đọc dữ liệu luật ở database điều khiển.
pub(crate) struct PromotionsService {
    pub(crate) databases: DatabaseRegistry,
    pub(crate) outbox: Outbox,
    pub(crate) entitlements: EntitlementsHandle,
    /// `dyn Catalog` của `catalog` (đăng ký trước): biến thể, giá tham chiếu, sàn giá.
    pub(crate) catalog: Arc<dyn Catalog>,
    /// `dyn OrgDirectory` của `organizations` khi đã cài: cài đặt kế thừa, múi giờ cửa hàng.
    pub(crate) orgs: Option<Arc<dyn OrgDirectory>>,
    pub(crate) translations: Translations,
    pub(crate) config: PromotionsConfig,
    pub(crate) legal_cache: LegalCache,
    pub(crate) compiled: CompiledCache,
    pub(crate) jobs: autoxide_kernel::job::JobQueue,
    /// Sổ đồng ý của `compliance` khi đã cài: bằng chứng đồng ý cho lời nhắc flash sale.
    pub(crate) consents: Option<Arc<dyn autoxide_compliance::api::ConsentLedger>>,
    /// `dyn Notifier` của `notifications` khi đã cài: gửi lời nhắc; không có thì không gửi.
    pub(crate) notifier: Option<Arc<dyn autoxide_notifications::api::Notifier>>,
}

impl PromotionsService {
    /// Giá trị hiệu lực của cài đặt `key` ở nút `org`; không có `organizations` thì `None`.
    pub(crate) async fn setting(
        &self,
        tenant: &TenantContext,
        org: OrgId,
        key: &str,
    ) -> Result<Option<serde_json::Value>, Error> {
        let Some(orgs) = &self.orgs else {
            return Ok(None);
        };
        Ok(orgs.setting(tenant, org, key).await?.and_then(|found| found.value))
    }

    /// Múi giờ của nút `org` (`organizations.timezone`, kế thừa); chưa đặt thì giờ Việt Nam.
    pub(crate) async fn timezone(&self, tenant: &TenantContext, org: OrgId) -> Result<Tz, Error> {
        Ok(self
            .setting(tenant, org, "organizations.timezone")
            .await?
            .and_then(|value| value.as_str().and_then(|name| name.parse::<Tz>().ok()))
            .unwrap_or(chrono_tz::Asia::Ho_Chi_Minh))
    }
}

/// Bây giờ, cắt tới micro giây: Postgres lưu tới micro giây, SQLite lưu nguyên. Cắt trước để hai
/// database trả về cùng một giá trị và so bằng nhau được.
pub(crate) fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

/// Cắt một thời điểm client gửi tới micro giây (như [`now`]).
pub(crate) fn micros(at: DateTime<Utc>) -> DateTime<Utc> {
    at.trunc_subsecs(6)
}

/// Nút của chương trình mới khi người tạo không nêu: gốc nếu họ có `permission` cho cả khách; nút
/// duy nhất nếu mọi grant ở một nút; nhiều nút thì phải chọn (`promotions.org_required`).
pub(crate) fn default_org(
    tenant: &TenantContext,
    principal: &Principal,
    permission: &Permission,
) -> Result<OrgId, Error> {
    let grants = principal.grants(permission);
    if grants.is_empty() || grants.iter().any(|grant| grant.scope == Scope::Tenant) {
        return Ok(root_of(tenant.id()));
    }
    let first = grants[0].org;
    if grants.iter().all(|grant| grant.org == first) {
        return Ok(first);
    }
    Err(Error::new(
        ErrorKind::Validation,
        "promotions.org_required",
        "You work in several organization units: give `org_id`.",
    )
    .with_field(FieldError::new(
        "org_id",
        "required",
        "you work in several organization units: choose one",
    )))
}

/// Đích kiểm quyền của bản ghi thuộc nút `org`.
pub(crate) fn at(org: Uuid) -> OrgTarget {
    OrgTarget::new(OrgId::from_uuid(org), None)
}

/// Đọc bản ghi thuộc nút `org`: cần `permission` phủ nút; không phủ thì báo như không có.
pub(crate) async fn readable<C: ConnectionTrait>(
    principal: &Principal,
    connection: &C,
    permission: &Permission,
    org: Uuid,
    not_found: impl FnOnce() -> Error,
) -> Result<(), Error> {
    match principal.require_at(connection, permission, at(org)).await {
        Err(error) if error.code() == "org_out_of_scope" => Err(not_found()),
        other => other,
    }
}

/// Ghi bản ghi thuộc nút `org` trong transaction: không đọc được thì 404 (không lộ bản ghi tồn
/// tại), đọc được mà không ghi được thì 403; nút đang khoá → `org_frozen`.
pub(crate) async fn writable<C: ConnectionTrait>(
    principal: &Principal,
    transaction: &C,
    write: &Permission,
    org: Uuid,
    not_found: impl FnOnce() -> Error,
) -> Result<(), Error> {
    let target = at(org);
    if principal.require_at(transaction, &crate::permissions::READ, target).await.is_err()
        && principal.require_at(transaction, write, target).await.is_err()
    {
        return Err(not_found());
    }
    principal.require_write_at(transaction, write, target).await
}

/// Lỗi vi phạm unique.
pub(crate) fn is_unique_violation(error: &DbErr) -> bool {
    matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_)))
}

/// Tăng số phiên bản bộ chương trình của khách, cùng transaction với thay đổi: máy chủ nào cũng
/// biết phải nạp lại `CompiledPromotions`.
/// Khoá dòng phiên bản bộ luật đã biên dịch của khách tới hết `transaction` ghi (tạo dòng nếu chưa
/// có). Dùng làm khoá chung của khách khi phải xét nhiều chương trình cùng lúc (vd tổng số ngày
/// giảm giá trong năm khi bật — D-181): hai lần bật nối đuôi nhau, bên sau thấy bên trước đã bật.
/// Khoá SAU dòng chương trình (cùng thứ tự với [`bump_revision`] ở cuối mọi lệnh ghi) để không
/// deadlock.
pub(crate) async fn lock_revision(
    transaction: &sea_orm::DatabaseTransaction,
    tenant: &TenantContext,
) -> Result<(), Error> {
    use crate::entity::revision;
    use sea_orm::sea_query::OnConflict;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};
    revision::Entity::insert(revision::ActiveModel {
        tenant_id: Set(tenant.id().as_uuid()),
        revision: Set(0),
    })
    .on_conflict(OnConflict::column(revision::Column::TenantId).do_nothing().to_owned())
    .exec_without_returning(transaction)
    .await?;
    autoxide_kernel::rows::lock_one(
        transaction,
        revision::Entity::find().filter(revision::Column::TenantId.eq(tenant.id().as_uuid())),
    )
    .await?;
    Ok(())
}

pub(crate) async fn bump_revision<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
) -> Result<(), Error> {
    use crate::entity::revision;
    use sea_orm::sea_query::OnConflict;
    use sea_orm::{ColumnTrait, EntityTrait, Set};
    revision::Entity::insert(revision::ActiveModel {
        tenant_id: Set(tenant.id().as_uuid()),
        revision: Set(0),
    })
    .on_conflict(OnConflict::column(revision::Column::TenantId).do_nothing().to_owned())
    .exec_without_returning(transaction)
    .await?;
    autoxide_kernel::rows::adjust::<revision::Entity, _>(
        transaction,
        revision::Column::Revision,
        revision::Column::TenantId.eq(tenant.id().as_uuid()),
        1,
        autoxide_kernel::rows::Bounds::ANY,
    )
    .await?;
    Ok(())
}
