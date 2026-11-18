//! Permission của module `support`. Mọi permission theo nút (bước 34): nhân viên thấy phiếu của
//! nút mình và nhánh dưới. Người mua (tài khoản `customer`) không có permission nào: các endpoint
//! `/me/…` chỉ cần đăng nhập và chỉ chạm phiếu của chính người gọi (D-20261118-step53-support).

use autoxide_kernel::access::Permission;

pub(crate) const TICKETS_READ: Permission = Permission::new(
    "support.tickets.read",
    "See the support tickets of the units in your scope, with their messages and internal notes.",
)
.scoped();

pub(crate) const TICKETS_REPLY: Permission = Permission::new(
    "support.tickets.reply",
    "Answer support tickets in your scope, add internal notes, open tickets for customers, and \
     change status, priority, category or assignee.",
)
.scoped();

pub(crate) const TICKETS_MANAGE: Permission = Permission::new(
    "support.tickets.manage",
    "Hide messages of support tickets in your scope and hand tickets over as consumer complaints \
     or to the platform.",
)
.scoped();

pub(crate) const SETTINGS_MANAGE: Permission = Permission::new(
    "support.settings.manage",
    "Change support policies, teams, saved replies and routing rules of the units in your scope.",
)
.scoped();

pub(crate) const AUTO_REPLY_MANAGE: Permission = Permission::new(
    "support.auto_reply.manage",
    "Approve saved replies for automatic answers, try the automatic answer and read its log.",
)
.scoped();

pub(crate) fn all() -> Vec<Permission> {
    vec![TICKETS_READ, TICKETS_REPLY, TICKETS_MANAGE, SETTINGS_MANAGE, AUTO_REPLY_MANAGE]
}
