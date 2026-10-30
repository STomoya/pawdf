#![no_main]

#[path = "common.rs"]
mod common;

use libfuzzer_sys::fuzz_target;
use pawdf_reader::Document;

// Input: objects 5.., split at 0xFF; object 5 is `/Names /Dests` and 6 the legacy
// `/Dests`, looked up from two link annotations.
fuzz_target!(|data: &[u8]| {
    let pdf = common::objects(
        "/Names << /Dests 5 0 R >> /Dests 6 0 R",
        "/Annots [<< /Subtype /Link /Rect [0 0 1 1] /Dest (a) >> << /Subtype /Link /Rect [0 0 1 1] /Dest /a >>]",
        b"",
        data,
    );
    if let Ok(doc) = Document::from_bytes(pdf)
        && let Ok(page) = doc.page(0)
    {
        let _ = page.annotations();
    }
});
