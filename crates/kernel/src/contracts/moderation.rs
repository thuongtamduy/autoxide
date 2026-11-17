//! Ẩn tạm một đối tượng vì vụ việc pháp lý (bước 37, D-363): báo vi phạm sở hữu trí tuệ, hàng giả,
//! khiếu nại, yêu cầu của cơ quan có thẩm quyền.
//!
//! Module xử lý vụ việc (`compliance`) **không** đụng bảng của module khác (luật kiến trúc 1): nó
//! phát event [`TARGET_RESTRICTED`] / [`TARGET_RESTORED`] với payload [`Restriction`] (đích dạng
//! `<module>.<loại>:<id>` của sổ đối tượng, D-123). Module sở hữu đích nghe hai event đó qua
//! `Module::subscriptions` rồi tự ẩn / hiện lại (vd `content` trả 451, `comments` ẩn bình luận) — lấy
//! dữ liệu bằng [`restriction`]. Event được giao lại tới khi bên nhận xong, nên bên nhận phải
//! idempotent (ẩn hai lần như ẩn một lần).
//!
//! Như [`USER_DELETED`](crate::privacy::USER_DELETED), đây là hợp đồng của kernel: module sở hữu
//! đích được đăng ký nhận hai event này kể cả khi chưa cài module xử lý vụ việc.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::event::Delivered;

/// Đích bị ẩn tạm (chờ xử lý vụ việc, hay vụ việc kết luận có vi phạm).
pub const TARGET_RESTRICTED: &str = "compliance.target_restricted";

/// Đích được hiện lại (vụ việc bác, rút, hay người xử lý bỏ ẩn).
pub const TARGET_RESTORED: &str = "compliance.target_restored";

/// Payload của [`TARGET_RESTRICTED`] / [`TARGET_RESTORED`]: chỉ ID và loại, không tên người báo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Restriction {
    /// Vụ việc gây ra thay đổi.
    pub case_id: Uuid,
    /// Đích, dạng `<module>.<loại>:<id>` (vd `content.entry:0190…`).
    pub target: String,
    /// Loại vụ việc (`ip_infringement`, `counterfeit`, `prohibited_goods`, `consumer_complaint`,
    /// `other`).
    pub case_kind: String,
}

impl Restriction {
    /// Restriction of `target` because of case `case_id` of kind `case_kind`.
    pub fn new(case_id: Uuid, target: impl Into<String>, case_kind: impl Into<String>) -> Self {
        Restriction { case_id, target: target.into(), case_kind: case_kind.into() }
    }

    /// ID của đích khi nó thuộc loại `kind` (vd `content.entry`); `None` khi khác loại.
    pub fn id_in(&self, kind: &str) -> Option<&str> {
        self.target
            .strip_prefix(kind)
            .and_then(|rest| rest.strip_prefix(':'))
            .filter(|id| !id.is_empty())
    }
}

/// Thay đổi một event mang tới.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RestrictionChange {
    /// Ẩn đích.
    Restricted,
    /// Hiện lại đích.
    Restored,
}

/// Thay đổi và payload của một event [`TARGET_RESTRICTED`] / [`TARGET_RESTORED`]; `None` khi event
/// khác hay payload sai hình dạng.
pub fn restriction(event: &Delivered) -> Option<(RestrictionChange, Restriction)> {
    let change = match event.name.as_str() {
        TARGET_RESTRICTED => RestrictionChange::Restricted,
        TARGET_RESTORED => RestrictionChange::Restored,
        _ => return None,
    };
    serde_json::from_value(event.payload.clone()).ok().map(|payload| (change, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_cua_dich_theo_loai() {
        let id = Uuid::now_v7();
        let restriction = Restriction::new(id, format!("content.entry:{id}"), "counterfeit");
        assert_eq!(restriction.id_in("content.entry"), Some(id.to_string()).as_deref());
        assert_eq!(restriction.id_in("content.term"), None);
        assert_eq!(restriction.id_in("content"), None);
        assert_eq!(Restriction::new(id, "content.entry:", "other").id_in("content.entry"), None);
    }
}
