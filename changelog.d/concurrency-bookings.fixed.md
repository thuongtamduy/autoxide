- Module `bookings`: changes to one booking are serialised — two reschedules at once no longer
  leak a slot or release the old session seat twice, and a deposit that arrives while the hold
  expires no longer confirms a booking whose slot was already released (Postgres).
