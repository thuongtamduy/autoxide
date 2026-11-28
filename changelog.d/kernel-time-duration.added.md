- Kernel `time`: new types `Duration` (signed, microsecond precision; `from_secs`/`from_mins`/`from_hours`/
  `from_days`/`from_millis`, `as_secs`/`as_millis`…, no `serde`), `TimeZone` (IANA, `TimeZone::get`),
  `TimeOfDay` (`parse_hm`), `Weekday` and `DateTime` (local wall-clock time), so modules no longer need `jiff`.
  `Timestamp + Duration` / `- Duration` never panic (they clamp at `Timestamp::MIN`/`MAX`; `checked_add`/
  `checked_sub` return `None` on overflow), `Timestamp::since` gives a `Duration`, `Timestamp::to_local` and
  `TimeZone::to_timestamp` convert between instants and local time, `Date::day_of_week` and `Date::at` complete
  the calendar. `date_in`, `to_rfc3339_in`, `in_zone` and `Date::start_of_day_in` accept either zone type, and
  `OneTimeTokens::issue` accepts either duration type. The re-export `time::SignedDuration` and the functions
  `time::zone`, `time::from_local`, `time::parse_clock` still work and will be deprecated once no module in the
  repository uses them.
