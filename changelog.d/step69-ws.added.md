- `GET /api/v1/kernel/events/ws` (step 69): the events of the SSE stream over a WebSocket, with
  the same permissions, resuming and connection limits; the client sends only control messages
  (`subscribe`, `unsubscribe`, `ack`, `join`, `leave`, `typing`, `ping`) — data is still written
  over REST. Credentials are checked again every `AUTOXIDE_EVENTS_WS_RECHECK_SECONDS` (close code
  `4401` when they expired). Modules open live "typing" / "online" signals in their topics with
  `SignalTopics`.
