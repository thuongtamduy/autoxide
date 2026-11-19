//! Permission của module `conversations`. Mọi permission theo nút (bước 34): nhân viên thấy cuộc trò
//! chuyện của nút mình và nhánh dưới (độ sâu `own`: cuộc được giao cho mình). Người mua (tài khoản
//! `customer`) không có permission nào: các endpoint `/me/…` chỉ cần đăng nhập và chỉ chạm cuộc trò
//! chuyện của chính người gọi.

use autoxide_kernel::access::{Permission, Risk};

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

pub(crate) const CALL: Permission = Permission::new(
    "conversations.call",
    "Work on call-back requests of the units in your scope, call customers back and see the calls \
     of a conversation (phone numbers stay masked).",
)
// Dữ liệu cá nhân của người mua (số đã che, lịch gọi lại): mức nhạy cảm, không phải ghi thường.
.with_risk(Risk::SensitiveData)
.scoped();

pub(crate) const RECORDINGS_READ: Permission = Permission::new(
    "conversations.recordings.read",
    "Listen to call recordings of the units in your scope; every listen is recorded.",
)
// Giọng nói của người mua: dữ liệu cá nhân nhạy cảm.
.with_risk(Risk::SensitiveData)
.scoped();

pub(crate) const VOICE_MANAGE: Permission = Permission::new(
    "conversations.voice.manage",
    "Set up the phone line of the units in your scope, switch call recording on or off, and delete \
     recordings when a customer asks.",
)
// Xoá bản ghi không lấy lại được.
.with_risk(Risk::Destructive)
.scoped();

pub(crate) fn all() -> Vec<Permission> {
    vec![READ, REPLY, MANAGE, CALL, RECORDINGS_READ, VOICE_MANAGE]
}
