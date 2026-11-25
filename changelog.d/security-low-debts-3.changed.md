- Kernel: `Delivered::org_id()` reads the organization unit an event is about (`org_id`, `after.org_id` or
  `before.org_id` of the payload); modules record events about a unit's data with its `org_id`.
- **Behaviour change**, module `automations`: a rule attached to a unit below the tenant's root no longer starts
  for events that do not carry an `org_id` (top level, `after` or `before`); rules at the root are unchanged.
  Events that carry no `org_id` yet, although their data belongs to a unit, so a branch rule on them stops
  starting until their module adds it: `notes`, `menus`, `forms` and `organizations` (every event); `promotions` (every event: `org_id` sits under
  `after.promotion`); `content` (`entry_*`, `review_*`, `type_changed`); `media` (`item_*`); `bookings`
  (`booking_*`, `schedule_replaced`, `exception_*`); `conversations` (`message_*`, `typing`, `recording_*`);
  `support` (`message_added`, `message_hidden`, `ticket_linked`, `satisfaction_recorded`, `auto_repl*`,
  `tickets_claimed`); `catalog` (`variant_*`, `preorder_date_changed`, `price_effective`, `downloads_*`);
  `customers` (`profile_updated`, `profile_linked`, `contact_revealed`, `group_*`); `inventory`
  (`reservation_*`, `transfer_*`, `preorder_allocated`: the unit is under another key); `orders`
  (`payment_recorded`); `invoices` (`document_accepted`, `_rejected`, `_cancelled`, `_corrected`); `ledger`
  (`payment_refunded`); `shipping` (`cod_discrepancy_resolved`); `compliance` (`website_updated`,
  `website_status_changed`); `automations` (`run_failed`); `auth` (`api_key_revoked`, `api_key_approved`,
  `memberships_changed`, `user_roles_changed`).
  Attach such rules to the root unit to keep them running.
