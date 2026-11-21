- Modules `orders` and `promotions`: per-buyer promotion limits now count every buyer, signed in or
  not, by account, phone number and delivery address; a promotion with a per-buyer limit is refused
  when the buyer cannot be identified (`promotions.buyer_unidentified`).
