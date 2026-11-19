- Module `orders`: returns and refunds — buyers ask to return items of a delivered order inside the shop's return window
  with an allowed reason (`POST /me/orders/{id}/returns`, setting `orders.allowed_return_reasons`); staff approve,
  reject, receive (restocked or damaged, sent to inventory), refund and close returns (`/returns`,
  `POST /returns/{id}/transitions`); refunds are proportional to what was paid, add the shipping fee when the seller is
  at fault, take back order discounts or gift values per the promotion snapshot, and never exceed what was paid, even
  when sent concurrently; `POST /{id}/refunds` and `GET /{id}/refunds`; refunds go through the payment partner or are
  recorded as paid by the shop, with reversing ledger entries; MCP tools `orders.refund_order`, `orders.list_returns`,
  `orders.decide_return`.
