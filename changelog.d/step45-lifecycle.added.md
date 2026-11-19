- Module `orders`: order lifecycle — staff moves orders with `If-Match` (`POST /{id}/transitions`), buyers cancel
  their own orders up to the shop's policy (`POST /me/orders/{id}/cancel`), cancelled paid orders are refunded through
  the ledger, `POST /{id}/shipments` asks the shipping module for a shipment (cash to collect included), carrier events
  move orders to shipping / delivered / delivery failed, revenue is posted to the ledger on delivery, orders complete
  when the return window ends unless held (`OrderHolds`), shops can be given hours to accept orders; per-unit
  settings `orders.buyer_cancel_until`, `orders.acceptance_hours`, `orders.return_window_days`,
  `orders.cod_enabled`; `POST /{id}/payment-link`; MCP tools `orders.list_orders`, `orders.get_order`,
  `orders.transition_order`, `orders.request_shipment`.
