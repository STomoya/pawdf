#![no_main]

#[path = "common.rs"]
mod common;

use libfuzzer_sys::fuzz_target;
use pawdf_layout::pdfminer::{Params, analyze};
use pawdf_reader::Document;

fuzz_target!(|data: &[u8]| {
    let pdf = common::one_page(data, None);
    if let Ok(doc) = Document::from_bytes(pdf)
        && let Ok(page) = doc.page(0)
    {
        let content = page.content();
        let vertical = Params {
            boxes_flow: None,
            detect_vertical: true,
            ..Params::default()
        };
        for params in [Params::default(), vertical] {
            let _ = analyze(&content, page.media_box(), page.rotate(), &params);
        }
    }
});
