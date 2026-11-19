//! `SupportIntake`: module khác (vd `orders`, bước 45) mở phiếu thay người mua. Liên lạc đã được
//! module đó chứng minh (OTP của trang xem đơn) nên phiếu xác thực sẵn, không hỏi OTP lại.

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::sms::PhoneNumber;
use autoxide_kernel::tenancy::TenantContext;

use crate::api::{NewTicket, Requester, SupportIntake, TicketRef};
use crate::service::SupportService;
use crate::status::Channel;
use crate::tickets::{Opening, Who};

#[async_trait::async_trait]
impl SupportIntake for SupportService {
    async fn open(&self, tenant: &TenantContext, ticket: NewTicket) -> Result<TicketRef, Error> {
        let requester = match ticket.requester {
            Requester::User(id) => Who::User(id),
            Requester::VerifiedEmail(email) => {
                Who::Email(autoxide_kernel::email::normalize(&email))
            }
            Requester::VerifiedPhone(raw) => Who::Phone(
                PhoneNumber::parse(&raw, "84").map(|phone| phone.as_str().to_owned()).ok_or_else(
                    || Error::internal("SupportIntake: the phone number is not E.164"),
                )?,
            ),
        };
        let channel =
            if matches!(requester, Who::User(_)) { Channel::Web } else { Channel::OrderLookup };
        let opening = Opening {
            org_id: Some(ticket.org_id),
            requester,
            requester_name: ticket.requester_name,
            verified: true,
            category: ticket.category,
            subject: ticket.subject,
            body: ticket.body,
            priority: None,
            channel,
            media_ids: Vec::new(),
            links: ticket.links,
            reference: Some(ticket.reference),
            follow_up_of: None,
            locale: None,
        };
        let opened = self.open_ticket(tenant, Actor::system(), None, opening, None).await?;
        Ok(TicketRef {
            id: opened.ticket.id,
            number: opened.ticket.number,
            access_token: opened.access_token,
        })
    }
}

/// Bộ tra loại đối tượng `support.ticket` cho sổ đối tượng: phiếu có thật, không công khai.
pub(crate) struct TicketResolver;

#[async_trait::async_trait]
impl autoxide_kernel::resource::ResourceResolver for TicketResolver {
    async fn resolve(
        &self,
        tenant: &TenantContext,
        id: &str,
    ) -> Result<Option<autoxide_kernel::resource::Resource>, Error> {
        use autoxide_kernel::tenancy::ScopeExt;
        use sea_orm::EntityTrait;
        let Ok(id) = id.parse::<uuid::Uuid>() else {
            return Ok(None);
        };
        let row = crate::entity::ticket::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        Ok(row.map(|row| {
            autoxide_kernel::resource::Resource::new(format!("Support ticket {}", row.number))
                .url(Some(format!("/support/tickets/{id}")))
        }))
    }
}
