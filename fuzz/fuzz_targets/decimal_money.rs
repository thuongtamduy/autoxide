//! Amounts and currency codes in request bodies: parsing never panics, whatever the text.
#![no_main]

use autoxide_kernel::{decimal, money};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: (&str, &str)| {
    let (amount, currency) = input;
    let _ = decimal::parse(amount);
    let _ = money::Money::parse(amount, currency);
    let _ = money::Currency::parse(currency);
});
