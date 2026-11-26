- Stopping the server (SIGTERM) right after heavy traffic no longer keeps the process alive until every pending
  event is delivered: the event dispatcher and the job worker now check for the stop signal between passes even
  while work is waiting. Undelivered events and unclaimed jobs stay in the database and are handled by the next run.
