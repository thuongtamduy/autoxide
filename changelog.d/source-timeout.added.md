- Setting `AUTOXIDE_SOURCE_TIMEOUT_MS` (50 to 60000): how long one module source may take in the work inbox,
  global search, summary, setup steps and open-by-code before it is skipped and listed in `incomplete`. Unset
  keeps the old limit (300 ms; 1500 ms in debug builds). `TestApp` sets it to 60000 unless the test sets it.
