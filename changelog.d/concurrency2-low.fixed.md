- Module `bookings`: several people who are the first to book the same class at the same time all
  get a seat while the class has room, instead of `bookings.slot_taken`.
- Module `conversations`: an automatic reply is no longer added to a conversation that was just
  closed or handed to a person; marking as read from two devices at once never moves the read
  cursor back.
- Module `automations`: runs started at the same time can no longer go over
  `AUTOXIDE_AUTOMATIONS_MAX_RUNS_PER_HOUR`.
- Module `content`: banners created at the same time can no longer go over the placement's
  `max_items` or share a position.
- Module `newsletter`: confirming and unsubscribing at the same time always ends unsubscribed.
- Module `tax`: two changes of a tax profile with the same `If-Match` no longer both pass; the
  later one gets `412`.
