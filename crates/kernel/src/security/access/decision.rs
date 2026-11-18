//! Quyết định quyền trên bản ghi: MỘT mô tả luật cho cả hai cách dùng (bước 54 mục 1, D-20261118-step54-one-decision).
//!
//! Danh sách lọc bằng điều kiện SQL ([`Principal::scope_condition`]), một bản ghi kiểm bằng
//! [`Principal::check`] / [`Principal::require_at`]. Hai đường này trước đây viết luật hai lần
//! nên có thể lệch nhau: danh sách cho thấy một bản ghi mà mở ra lại bị 403, hay ngược lại. Giờ cả
//! hai đọc cùng một [`Reach`] — phần mà `permission` cho phép, rút một lần từ grant của người gọi —
//! và test `access_check` (sinh ngẫu nhiên cây, grant, bản ghi) chứng minh hai đường cho cùng kết quả
//! trên cả SQLite và Postgres.

use sea_orm::sea_query::Condition;
use sea_orm::{ColumnTrait, ConnectionTrait};
use uuid::Uuid;

use super::{OrgTarget, Permission, Principal, Scope, org_out_of_scope, permission_denied};
use crate::error::Error;
use crate::tenancy::{OrgId, OrgTree, TenantId};

/// Kết quả của một lần quyết định quyền trên một bản ghi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Được làm.
    Allow,
    /// Không có `permission` ở nút nào (403 `permission_denied`).
    Missing,
    /// Có `permission` nhưng bản ghi nằm ngoài phạm vi (403 `org_out_of_scope`; đọc thì service trả
    /// 404 để không lộ bản ghi tồn tại).
    OutOfScope,
}

/// Phần một permission cho phép: rút từ grant của người gọi, dùng chung cho điều kiện SQL và kiểm
/// từng bản ghi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Reach {
    /// Có độ sâu `tenant`: mọi nút.
    everywhere: bool,
    /// Độ sâu `unit`: đúng các nút này.
    units: Vec<OrgId>,
    /// Độ sâu `subtree`: các nút này và mọi con cháu.
    subtrees: Vec<OrgId>,
    /// Độ sâu `own`: bản ghi của chính người gọi; bản ghi không có người sở hữu thì như `unit` ở
    /// các nút này.
    owns: Vec<OrgId>,
    /// Hàng rào của API key gắn nút: mọi grant chỉ có hiệu lực trong nhánh này.
    fence: Option<OrgId>,
    tenant: TenantId,
    user: Uuid,
}

impl Reach {
    /// Điều kiện SQL: bản ghi có nút ở cột `org`, người sở hữu ở cột `owner` (nếu có).
    pub(super) fn condition<C: ColumnTrait>(&self, org: C, owner: Option<C>) -> Condition {
        let mut condition = Condition::all();
        if !self.everywhere {
            let mut any = Condition::any();
            if !self.subtrees.is_empty() {
                any = any.add(OrgTree::subtree_condition(self.tenant, &self.subtrees, org));
            }
            let mut units = self.units.clone();
            match owner {
                Some(owner) if !self.owns.is_empty() => any = any.add(owner.eq(self.user)),
                _ => units.extend(self.owns.iter().copied()),
            }
            if !units.is_empty() {
                any = any.add(org.is_in(units.iter().map(OrgId::as_uuid)));
            }
            condition = condition.add(any);
        }
        if let Some(fence) = self.fence {
            condition = condition.add(OrgTree::subtree_condition(self.tenant, &[fence], org));
        }
        condition
    }

    /// Bản ghi `target` có nằm trong phần này không — cùng nghĩa với [`condition`](Self::condition).
    pub(super) async fn covers<D: ConnectionTrait>(
        &self,
        db: &D,
        target: OrgTarget,
    ) -> Result<bool, Error> {
        let owned = match target.owner {
            Some(owner) => !self.owns.is_empty() && owner == self.user,
            None => self.owns.contains(&target.org),
        };
        let mut allowed = self.everywhere || owned || self.units.contains(&target.org);
        if !allowed && !self.subtrees.is_empty() {
            allowed = OrgTree::is_within(db, self.tenant, target.org, &self.subtrees).await?;
        }
        if allowed && let Some(fence) = self.fence {
            allowed = OrgTree::is_within(db, self.tenant, target.org, &[fence]).await?;
        }
        Ok(allowed)
    }
}

impl Principal {
    /// Phần `permission` cho phép; `None` khi không có grant nào.
    pub(super) fn reach(&self, permission: &Permission) -> Option<Reach> {
        let grants = self.grants(permission);
        if grants.is_empty() {
            return None;
        }
        let pick = |scope: Scope| -> Vec<OrgId> {
            grants.iter().filter(|grant| grant.scope == scope).map(|grant| grant.org).collect()
        };
        Some(Reach {
            everywhere: grants.iter().any(|grant| grant.scope == Scope::Tenant),
            units: pick(Scope::Unit),
            subtrees: pick(Scope::Subtree),
            owns: pick(Scope::Own),
            fence: self.fence(),
            tenant: self.tenant_id(),
            user: self.user_id(),
        })
    }

    /// Hàm quyết định duy nhất cho một bản ghi: người gọi có được dùng `permission` trên `target`
    /// không. [`require_at`](Self::require_at) đổi kết quả thành lỗi; danh sách dùng
    /// [`scope_condition`](Self::scope_condition), sinh từ cùng luật.
    pub async fn check<D: ConnectionTrait>(
        &self,
        db: &D,
        permission: &Permission,
        target: OrgTarget,
    ) -> Result<Decision, Error> {
        let Some(reach) = self.reach(permission) else { return Ok(Decision::Missing) };
        Ok(if reach.covers(db, target).await? { Decision::Allow } else { Decision::OutOfScope })
    }

    /// Điều kiện SeaORM giới hạn một danh sách vào phần `permission` cho phép: cột `org` là nút
    /// của bản ghi, `owner` là cột người sở hữu (nếu có; không có thì `own` hiểu như `unit`).
    /// `Condition::all()` (không giới hạn) khi có độ sâu `tenant` và không có hàng rào; lỗi
    /// `permission_denied` khi không có grant nào. Dùng cùng `.scoped(&tenant)`.
    pub fn scope_condition<C: ColumnTrait>(
        &self,
        permission: &Permission,
        org: C,
        owner: Option<C>,
    ) -> Result<Condition, Error> {
        let reach = self.reach(permission).ok_or_else(|| permission_denied(permission))?;
        Ok(reach.condition(org, owner))
    }

    /// Kiểm `permission` trên một bản ghi (đọc / sửa / xoá) hay nút đích của lệnh tạo / dời:
    /// thiếu permission → `permission_denied`; ngoài phạm vi → `org_out_of_scope`. Đọc ngoài phạm
    /// vi thì service trả 404 thay vì lỗi này (không lộ bản ghi tồn tại).
    pub async fn require_at<D: ConnectionTrait>(
        &self,
        db: &D,
        permission: &Permission,
        target: OrgTarget,
    ) -> Result<(), Error> {
        match self.check(db, permission, target).await? {
            Decision::Allow => Ok(()),
            Decision::Missing => Err(permission_denied(permission)),
            Decision::OutOfScope => Err(org_out_of_scope(target.org, permission)),
        }
    }

    /// Như [`require_at`](Self::require_at), thêm: nút đích và mọi tổ tiên đang hoạt động (không
    /// thì `org_frozen`). Gọi TRONG transaction ghi của module.
    pub async fn require_write_at<D: ConnectionTrait>(
        &self,
        transaction: &D,
        permission: &Permission,
        target: OrgTarget,
    ) -> Result<(), Error> {
        self.require_at(transaction, permission, target).await?;
        OrgTree::ensure_writable(transaction, self.tenant_id(), target.org).await
    }
}
