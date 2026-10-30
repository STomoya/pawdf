#![no_main]

#[path = "common.rs"]
mod common;

use libfuzzer_sys::fuzz_target;
use pawdf_reader::Document;

// Input: objects 5.., split at 0xFF; object 5 is `/AcroForm`.
fuzz_target!(|data: &[u8]| {
    let pdf = common::objects("/AcroForm 5 0 R", "", b"", data);
    if let Ok(doc) = Document::from_bytes(pdf) {
        let _ = doc.form_fields();
    }
});
