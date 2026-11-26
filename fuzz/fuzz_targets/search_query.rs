//! The search box (`is:`, `-word`, `field:>value`, quoted phrases) is typed by end users: parsing never panics.
#![no_main]

use autoxide_kernel::find::SearchQuery;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let query = SearchQuery::parse(text);
    // Parsing again what we kept must not panic either.
    let _ = SearchQuery::parse(&query.text);
});
