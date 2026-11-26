//! `limit`, `cursor`, `order`, `count` of every list endpoint arrive as arbitrary text. Whatever they hold, validating
//! them must return `Ok` or an `Error`, never panic; a cursor that validates must also survive `text_key_after`.
#![no_main]

use autoxide_kernel::pagination::{PageQuery, PageRequest};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: (Option<String>, Option<String>, Option<String>, Option<String>)| {
    let (limit, cursor, order, count) = input;
    let mut query = PageQuery::new(limit, cursor, order);
    query.count = count;
    if let Ok(request) = PageRequest::try_from(query) {
        let _ = request.text_key_after("name");
        let _ = request.cursor();
        let _ = request.wants_total();
    }
});
