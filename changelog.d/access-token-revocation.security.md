- Module `auth`: signing out, ending a session, changing or resetting the password and disabling an
  account now stop the access tokens of the ended sessions at once (`401 auth.session_ended`), instead
  of letting them work until they expire (15 minutes by default). Checked in the same database query
  as before, so requests make no extra round trip.
