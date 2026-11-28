- Events: a subscribing module that fails, panics or times out no longer holds up the modules
  subscribed after it. Every module receives the event in the same pass; only the failing module is
  retried, with its own backoff and its own dead-letter entry in the new `kernel_event_deliveries`
  table (account deletion events still retry every hour until they succeed). Events waiting for a
  retry when you upgrade are delivered to every subscriber again; subscribers already accept a
  repeated event. `events_pending` and `events_dead` now also count those per-module retries.
