#![no_main]

#[path = "common.rs"]
mod common;

use libfuzzer_sys::fuzz_target;
use pawdf_reader::Document;

// Input: objects 5.., split at 0xFF; object 5 is `/OCProperties`, 6 the `/OC` of marked
// content and 7 the page's `/Annots`.
fuzz_target!(|data: &[u8]| {
    let pdf = common::objects(
        "/OCProperties 5 0 R",
        "/Resources << /Properties << /P 6 0 R >> >> /Annots 7 0 R",
        b"/OC /P BDC 0 0 m 1 1 l S EMC",
        data,
    );
    if let Ok(doc) = Document::from_bytes(pdf)
        && let Ok(page) = doc.page(0)
    {
        let _ = page.content();
        let _ = page.annotations();
    }
});
