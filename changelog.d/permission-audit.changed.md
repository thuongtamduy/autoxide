- `autoxide-testkit`: new `TestAppBuilder::without_permission_gate()` builds a test app without the kernel layer
  that enforces `x-permissions`, so a test can compare what the services accept with the documented permission
  lists. It relies on the kernel feature `permission-audit`, which only the testkit turns on; a running application
  never has this switch.
- A `permission_denied` error from an endpoint that accepts several permissions now names every one of them.
  Services can check "any of these permissions" with the new `Principal::require_any`.
- The permission layer also guards operations that declare their own `HEAD`, and finds the operation of a request
  with a map lookup instead of a scan.
