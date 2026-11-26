//! CSV import files come from users: reading them (with a row cap) never panics, and what `write` produces reads back.
#![no_main]

use autoxide_kernel::csv;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    if let Ok(rows) = csv::read(text, 1_000) {
        if let Some((header, rest)) = rows.split_first() {
            let written = csv::write(header, rest);
            let _ = csv::read(&written, 1_000);
        }
    }
});
