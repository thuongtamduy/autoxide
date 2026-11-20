- Module `automations`: enabling a rule checks that it is still the version the enabler validated
  (`412` otherwise) — an agent edit in between can no longer be enabled under a human's
  permissions without review. Concurrent edits get distinct version numbers.
