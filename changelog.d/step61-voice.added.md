- Calls in customer chat (module `conversations`): a shop sets up a phone line per unit
  (`PUT /voice/lines/{org_id}`); customers call from the browser or ask to be called back at their
  verified phone number within the shop's call-back hours; open requests show in the work inbox;
  agents call back with the shop's hotline shown; each call ends with one summary line in the
  conversation. Recording is off by default, needs a provider that plays a notice and the
  customer's confirmation, is kept `AUTOXIDE_CONVERSATIONS_RECORDING_RETENTION_DAYS` (90) days and
  can be deleted on request. Phone numbers are never shown in full. MCP tools
  `conversations.list_callbacks`, `conversations.list_calls`.
- Kernel contract `autoxide_kernel::voice::VoiceProvider` for voice provider connectors (with
  `FakeVoiceProvider` in `autoxide-testkit`); sandboxes never reach a real voice provider.
