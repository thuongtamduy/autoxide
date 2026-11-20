- Module `support`: a message sent while the ticket is being closed no longer reopens the closed
  ticket on Postgres; concurrent status changes no longer both start from the old status; an
  automatic reply re-checks inside its transaction that the ticket is still open and that no other
  automatic reply got in first.
