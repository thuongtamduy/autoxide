//! Từ vựng của phiếu và máy trạng thái. Đổi trạng thái chỉ qua [`transition`] — một chỗ duy nhất
//! quyết bước nào hợp lệ (như `orders`).
//!
//! ```text
//! open ──(người mua viết)──────────────┐
//!  │                                    ▼
//!  ├──(nhân viên trả lời)──▶ pending_customer ⇄ pending_agent
//!  │                                    │
//!  └──────────────▶ solved ◀────────────┘   (người mua viết vào `solved` → `pending_agent`)
//!                     │
//!                     ▼
//!                   closed   (không mở lại; người mua tạo phiếu mới `follow_up_of`)
//! ```

use autoxide_kernel::error::Error;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Defines a string enum with `as_str` / `parse`, shared by the database, the API and OpenAPI.
macro_rules! vocabulary {
    ($(#[$meta:meta])* $name:ident as $alias:ident { $($(#[$vmeta:meta])* $variant:ident = $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
        #[schema(as = $alias)]
        #[serde(rename_all = "snake_case")]
        pub(crate) enum $name {
            $($(#[$vmeta])* $variant),+
        }

        impl $name {
            pub(crate) fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $text),+
                }
            }

            pub(crate) fn parse(raw: &str) -> Option<Self> {
                match raw {
                    $($text => Some($name::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

vocabulary! {
    /// Where a ticket stands.
    Status as SupportTicketStatus {
        /// New; nobody answered yet.
        Open = "open",
        /// The customer wrote last: staff should answer.
        PendingAgent = "pending_agent",
        /// Staff answered: waiting for the customer (the response-time clock stops).
        PendingCustomer = "pending_customer",
        /// Staff marked it done; the customer may still write (it reopens).
        Solved = "solved",
        /// Final; the customer opens a new ticket instead.
        Closed = "closed",
    }
}

vocabulary! {
    /// How a ticket ended.
    Resolution as SupportResolution {
        Answered = "answered",
        /// Handed over as a consumer complaint or to the platform.
        Escalated = "escalated",
        Duplicate = "duplicate",
        Spam = "spam",
        /// The customer did not answer.
        NoReply = "no_reply",
    }
}

vocabulary! {
    /// How urgent a ticket is.
    Priority as SupportPriority {
        Low = "low",
        Normal = "normal",
        High = "high",
        Urgent = "urgent",
    }
}

vocabulary! {
    /// Where a ticket came from.
    Channel as SupportChannel {
        /// A signed-in customer on the website.
        Web = "web",
        /// A guest (no account) on the website.
        Guest = "guest",
        /// The order page (step 45), through `SupportIntake`.
        OrderLookup = "order_lookup",
        /// Staff wrote it down for the customer (a phone call…).
        Staff = "staff",
        /// A reply to a ticket email.
        Email = "email",
    }
}

vocabulary! {
    /// Who sees a message.
    MessageKind as SupportMessageKind {
        /// The customer sees it.
        Public = "public",
        /// Staff only (internal note).
        Internal = "internal",
        /// Written by the server (status change, assignment…).
        System = "system",
    }
}

vocabulary! {
    /// Who wrote a message.
    AuthorKind as SupportAuthorKind {
        Customer = "customer",
        Agent = "agent",
        System = "system",
    }
}

/// Kiểm bước `from` → `to`. Bước giữ nguyên trạng thái luôn hợp lệ (gọi lại không lỗi), trừ phiếu
/// đã đóng.
pub(crate) fn transition(from: Status, to: Status) -> Result<(), Error> {
    use Status::*;
    if from == Closed {
        return Err(ticket_closed());
    }
    let allowed = from == to
        || matches!(
            (from, to),
            (Open | PendingAgent | PendingCustomer | Solved, PendingAgent)
                | (Open | PendingAgent | PendingCustomer, PendingCustomer)
                | (Open | PendingAgent | PendingCustomer, Solved)
                | (_, Closed)
        );
    if allowed {
        Ok(())
    } else {
        Err(Error::conflict(
            "support.invalid_transition",
            "A ticket cannot go from `{from}` to `{to}`.",
        )
        .with_param("from", from.as_str())
        .with_param("to", to.as_str()))
    }
}

/// Trạng thái sau một tin công khai: người mua viết → chờ nhân viên (phiếu mới vẫn `open`); nhân
/// viên trả lời → chờ người mua.
pub(crate) fn after_message(from: Status, author: AuthorKind) -> Status {
    match (author, from) {
        (AuthorKind::Customer, Status::Open) => Status::Open,
        (AuthorKind::Customer, _) => Status::PendingAgent,
        (AuthorKind::Agent, _) => Status::PendingCustomer,
        (AuthorKind::System, status) => status,
    }
}

pub(crate) fn ticket_closed() -> Error {
    Error::conflict(
        "support.ticket_closed",
        "This ticket is closed. Open a new ticket and mention this one in `follow_up_of`.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Status; 5] = [
        Status::Open,
        Status::PendingAgent,
        Status::PendingCustomer,
        Status::Solved,
        Status::Closed,
    ];

    #[test]
    fn phieu_dong_khong_mo_lai() {
        for to in &ALL {
            assert_eq!(
                transition(Status::Closed, *to).unwrap_err().code(),
                "support.ticket_closed"
            );
        }
    }

    #[test]
    fn may_trang_thai_chi_cho_buoc_hop_le() {
        assert!(transition(Status::Open, Status::PendingCustomer).is_ok());
        assert!(transition(Status::Solved, Status::PendingAgent).is_ok());
        assert!(transition(Status::Solved, Status::Closed).is_ok());
        let error = transition(Status::Solved, Status::PendingCustomer).unwrap_err();
        assert_eq!(error.code(), "support.invalid_transition");
        assert!(transition(Status::PendingAgent, Status::Open).is_err());
    }

    #[test]
    fn tin_nhan_doi_trang_thai() {
        assert_eq!(after_message(Status::Open, AuthorKind::Customer), Status::Open);
        assert_eq!(after_message(Status::Solved, AuthorKind::Customer), Status::PendingAgent);
        assert_eq!(after_message(Status::PendingAgent, AuthorKind::Agent), Status::PendingCustomer);
        for status in &ALL {
            assert_eq!(Status::parse(status.as_str()), Some(*status));
        }
    }
}
