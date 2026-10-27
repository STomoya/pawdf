#![no_main]

#[path = "common.rs"]
mod common;

use libfuzzer_sys::fuzz_target;
use pawdf_reader::Document;

/// Pages extracted per input; more only slows the fuzzer down.
const MAX_PAGES: usize = 64;

fuzz_target!(|data: &[u8]| {
    // Inputs that are not PDFs exercise the ToUnicode parser instead (until M3's `cmap` target).
    let pdf = if data.starts_with(b"%PDF") {
        data.to_vec()
    } else {
        let codes: String = (0..=255u8).map(|b| format!("{b:02x}")).collect();
        common::one_page(
            format!("BT /F1 12 Tf <{codes}> Tj ET").as_bytes(),
            Some(data),
        )
    };
    let Ok(doc) = Document::from_bytes(pdf) else {
        return;
    };
    let _ = doc.diagnostics();
    let _ = doc.outline();
    let _ = doc.form_fields();
    if doc.page_count() <= MAX_PAGES {
        for i in 0..doc.page_count() {
            if let Ok(page) = doc.page(i) {
                let _ = page.content();
                let _ = page.annotations();
            }
        }
    }
});
