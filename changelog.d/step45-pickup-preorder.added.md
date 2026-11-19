- Module `orders`: collecting in store and pre-orders — `POST /{id}/pickup/ready` gives the buyer a 6-digit pickup code
  and holds the items for the pickup method's hold days; `POST /{id}/pickup/handover` with the code or the proxy given
  when ordering (`contact.proxy_name`, `contact.proxy_phone`); a reminder 24 hours before the hold ends, then
  `orders.pickup_expired_action` (`cancel_refund` by default, or `keep_waiting`). Checkout requires
  `accept_preorder_terms` when the cart has pre-order items; orders mixing in-stock and pre-order items ship in two
  deliveries (cash on delivery collected per delivery); a later pre-order date notifies the buyer and allows free
  cancellation; a pre-order variant switched off cancels and refunds the items not yet arrived. Order lists filter
  `pickup_org_id` and `pickup_status`; `orders.nothing_to_ship` when nothing is ready.
