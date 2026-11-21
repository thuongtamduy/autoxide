# Issuing e-invoices through SePay eInvoice

With the `einvoice_sepay` adapter, an invoice issued in Autoxide is sent to the tax authority through
[SePay eInvoice](https://developer.sepay.vn/vi/einvoice-api/v1/tong-quan), and its result (accepted, with
the tax authority's code, or refused, with the reason) comes back onto the document.

Vietnamese: [`sepay-einvoice.vi.md`](sepay-einvoice.vi.md).

> **Experimental.** This adapter was written from the provider's public documentation; not yet verified
> against the live service. Try it against SePay's sandbox before relying on it. You stay responsible for
> the content of your invoices; this guide is not legal advice.

Each tenant connects **its own** SePay eInvoice account. The client secret is stored encrypted and is
never shown again.

## 1. Store your SePay eInvoice account

You need the permission `kernel.settings.manage`.

```bash
for field in client_id client_secret provider_account_id; do
  read -r -p "$field: " value
  curl -X PUT "$API/api/v1/kernel/settings/einvoice_sepay.connection.api.$field" \
    -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
    -d "{\"value\": \"$value\"}"
done

# Test it: gets a token and reads the provider accounts
curl -X POST "$API/api/v1/kernel/settings/einvoice_sepay.connection.api.client_id/test" \
  -H "Authorization: Bearer $TOKEN"
```

| Setting | What it is |
| --- | --- |
| `einvoice_sepay.connection.api.client_id` | Client id of your SePay eInvoice account. |
| `einvoice_sepay.connection.api.client_secret` | Its client secret. |
| `einvoice_sepay.connection.api.provider_account_id` | Which e-invoice provider account to issue with. May be left unset when your SePay account has exactly one active provider account. |

## 2. Point an invoice series at SePay

Set `einvoice_provider` of the invoice series to `sepay`. Documents of that series are then sent when
they are issued, and asked about until the tax authority answers.

## What is sent, and what is not

- VAT invoices (SePay template `1`) and sales invoices (template `2`), in Vietnamese dong.
- The invoice symbol comes from your SePay account; the one of the Autoxide series is not used.
- Tax rates 0, 5, 8 and 10 %, and "not subject to tax". Another rate is refused before anything is sent.
- A corrected invoice is sent as a replacement or an adjustment of the original one.
- Sending the same document again never creates a second invoice: SePay knows it by the document's id.
- **The invoice number is given by the provider.** It appears on the PDF and XML you download; the number
  of the document in Autoxide may differ.
- **Cancelling is not available** through SePay's API: correct an invoice with a replacement or an
  adjustment.
- The operator can use SePay's sandbox with
  `AUTOXIDE_EINVOICE_SEPAY_API_URL=https://einvoice-api-sandbox.sepay.vn`.
