- Modules `bookings`, `inventory` and `orders`: the personal data export
  (`GET /api/v1/auth/me/export`) now includes a user's bookings, restock alerts and orders (with the
  contact and delivery details given at checkout). Deleting an account removes the name, phone, email
  and note from the user's bookings (the booking itself stays as a deposit record) and deletes their
  restock alerts; orders are kept as accounting records.
