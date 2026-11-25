- Per-tenant links in mail: each mail link (password reset, email verification, invitation, sign-in
  link, email-change report, guest support ticket, complaint tracking, back-in-stock confirmation,
  advertising unsubscribe, comment verification and unsubscribe, newsletter confirmation and
  preferences) is a tenant setting of group `link` (`<module>.link.<page>`) holding the whole page
  address, path included. It is accepted only on a host the tenant controls (a verified domain, the
  host of `kernel.public_url`) and checked again on every mail; otherwise the server's address is used
  as before. Support, compliance, inventory and notifications mails now follow `kernel.public_url`
  too.
- Tenant settings `newsletter.postal_address` (defaults to the owner's name and address in the tenant's
  legal profile, then `AUTOXIDE_NEWSLETTER_POSTAL_ADDRESS`), `content.embed_hosts` and
  `analytics.allowed_hosts`.
