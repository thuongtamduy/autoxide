- Module `orders`: guests look an order up by its number and phone number, confirm a one-time SMS
  code and get an `Order-Token` that opens that one order (phone, name and address masked); with it
  they cancel under the shop's policy, change the delivery address with a second code before the
  goods leave, or open a support ticket. Signed-in buyers attach their guest orders with
  `POST /me/orders/claim` and open support tickets from their orders.
- Module `orders`: answers "what is the status of my order?" for automatic support replies, only to
  the order's owner.
- Kernel: `SecretKey::phone_key` and `SecretKey::email_key` give a per-tenant keyed hash of a phone
  number or email, to match buyers without storing or comparing raw contact data across tenants.
