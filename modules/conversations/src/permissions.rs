//! Permission của module `conversations`. Mọi permission theo nút (bước 34): nhân viên thấy cuộc trò
//! chuyện của nút mình và nhánh dưới (độ sâu `own`: cuộc được giao cho mình). Người mua (tài khoản
//! `customer`) không có permission nào: các endpoint `/me/…` chỉ cần đăng nhập và chỉ chạm cuộc trò
//! chuyện của chính người gọi.

use autoxide_kernel::access::Permission;

pub(crate) const READ: Permission = Permission::new(
    "conversations.read",
    "See the customer conversations of the units in your scope, with their messages, and receive \
     their events in real time.",
)
.scoped();

pub(crate) const REPLY: Permission = Permission::new(
    "conversations.reply",
    "Answer customer conversations in your scope, take them, close them, ask for an AI draft and \
     hand them over as support tickets.",
)
.scoped();

pub(crate) const MANAGE: Permission = Permission::new(
    "conversations.manage",
    "Assign customer conversations of your scope to colleagues, block customers from writing to a \
     unit and review reports of abuse.",
)
.scoped();

pub(crate) fn all() -> Vec<Permission> {
    vec![READ, REPLY, MANAGE]
}
