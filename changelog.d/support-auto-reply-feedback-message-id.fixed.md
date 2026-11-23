- Module `support`: buyers and guests can rate an automatic answer on a ticket with the id of the message they see
  (`auto_reply: true`) in `POST …/tickets/{id}/auto-replies/{reply_id}/feedback`. Before, the endpoint only accepted
  the id from the staff-only log, so buyers could not call it.
