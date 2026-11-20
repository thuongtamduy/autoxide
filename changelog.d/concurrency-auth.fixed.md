- Module `auth`: an invitation revoked while it is being accepted either stays revoked (the
  acceptance fails) or is accepted (the revoke answers `409`) — never both; revoke no longer
  emits an event for an invitation that was accepted in the meantime.
