- Module `support`: parallel guesses of a guest ticket's verification code all count towards the
  five-attempt limit on Postgres (they used to see the same attempt count, so the limit did not
  stop a burst of guesses).
