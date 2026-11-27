- `DELETE /api/v1/conversations/calls/{id}/recording` and the recording retention job no longer forget a recording that
  is still kept by a voice provider whose connector is off: they answer `503`
  `conversations.voice_provider_unavailable` without deleting anything, and the job retries the call once the
  connector is on. Other expired recordings are still purged in the same run.
