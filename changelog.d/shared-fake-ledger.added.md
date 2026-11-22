- Module `ledger`: feature `testing` adds `autoxide_ledger::api::testing::FakeLedger` and `FakeLedgerModule`, an
  in-memory ledger for the tests of modules that post entries, take payments or listen to
  `ledger.payment_succeeded`, so module authors no longer copy a fake ledger into their tests.
