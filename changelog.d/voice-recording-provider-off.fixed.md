- `DELETE /api/v1/conversations/calls/{id}/recording` and the recording retention job no longer forget a recording that
  is still kept by a voice provider whose connector is off: they answer `503`
  `conversations.voice_provider_unavailable` without deleting anything, and the job retries the call once the
  connector is on. Other expired recordings, and other tenants on the same database, are still purged in the same run.
  Deleting an account while that connector is off no longer blocks the `auth.user_deleted` event: messages and
  callback requests are anonymised and the stored copy of the recording is deleted at once, while the recording at the
  provider is handed to the retention job, which deletes it once the connector is on
  (D-20261127-voice-recording-provider-off).
