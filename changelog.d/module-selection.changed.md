- **Upgrade action:** experimental modules (`bookings`, `newsletter`, `comments`, `support`,
  `conversations`, `analytics`, `reporting`, `automations`) no longer run unless
  `AUTOXIDE_MODULES` names them. If you use any of them, add for example
  `AUTOXIDE_MODULES=official,newsletter,comments` (or `AUTOXIDE_MODULES=all` for the behavior of
  0.17) before restarting. Nothing is deleted: when the database holds rows of a module that is
  off, `autoxide serve` logs a warning and `autoxide doctor` reports `modules_turned_off`, both
  with the exact line to add; `autoxide copy` and `autoxide backup create` / `restore` refuse to
  run until those modules are on. See "Upgrading from 0.17 or earlier" in `docs/MODULES.md`.
