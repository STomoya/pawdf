#![no_main]

#[path = "common.rs"]
mod common;

use libfuzzer_sys::fuzz_target;
use pawdf_reader::Document;

fuzz_target!(|data: &[u8]| {
    let pdf = common::one_page(data, None);
    if let Ok(doc) = Document::from_bytes(pdf)
        && let Ok(page) = doc.page(0)
    {
        let _ = page.content();
    }
});
