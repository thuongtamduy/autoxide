- New experimental module `momo`: the MoMo payment gateway (Vietnam) behind `PaymentGateway`, with one MoMo business
  account per tenant (`momo.connection.account.partner_code`, `.access_key`, `.secret_key`, optional `.environment`
  and `.return_url`). Payment links and QR codes, payment notifications (IPN) at each tenant's own address, status
  checks, full and partial refunds, and "test connection" from the settings API. New settings `AUTOXIDE_MOMO_API_URL`
  and `AUTOXIDE_MOMO_SANDBOX_API_URL`; new error codes `momo.account_rejected`, `momo.amount_out_of_range`,
  `momo.currency_not_supported`, `momo.notification_not_final`, `momo.order_already_created`,
  `momo.refund_already_sent` and `momo.return_url_required`. Written from the provider's public documentation; not yet
  verified against the live service. Turn it on with `AUTOXIDE_MODULES=official,momo`. Guide:
  `docs/guides/providers/momo.md`.
