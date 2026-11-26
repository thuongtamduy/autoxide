- An idle server runs far fewer SQL statements in the background: each second the event dispatcher and the job
  worker ask in one statement per database which modules have work, instead of querying every module's table.
  Delivery of events and jobs is as fast as before.
