- `autoxide-testkit`: new `TestAppBuilder::without_permission_gate()` builds a test app without the kernel layer
  that enforces `x-permissions`, so a test can compare what the services accept with the documented permission
  lists. It relies on the kernel feature `permission-audit`, which only the testkit turns on; a running application
  never has this switch.
- A `permission_denied` error from an endpoint that accepts several permissions now names every one of them and
  says that one is enough. Services can check "any of these permissions" with the new `Principal::require_any`,
  which asks for a recent sign-in (step-up) only when every listed permission the caller holds is high-risk; the
  order of the list does not matter. Services still call `Principal::require` for a high-risk permission where they
  use it.
- The permission layer follows an operation's own `HEAD` declaration (a public `HEAD` stays public); a route
  without one guards `HEAD` like its `GET`. It finds the operation of a request with a map lookup instead of a scan.
- Pausing, disabling or deleting an automation rule with a very large number of waiting runs no longer fails.
