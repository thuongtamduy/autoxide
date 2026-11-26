//! RFC 3339 timestamps and clock times from request bodies and settings: parsing never panics.
#![no_main]

use autoxide_kernel::time::{self, Timestamp};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let _ = Timestamp::parse_rfc3339(text);
    let _ = time::parse_clock(text);
});
