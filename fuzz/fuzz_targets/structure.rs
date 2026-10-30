#![no_main]

#[path = "common.rs"]
mod common;

use libfuzzer_sys::fuzz_target;
use pawdf_reader::Document;

// Input: objects 5.., split at 0xFF; object 5 is `/StructTreeRoot`.
fuzz_target!(|data: &[u8]| {
    let pdf = common::objects(
        "/StructTreeRoot 5 0 R",
        "/StructParents 0",
        b"/P <</MCID 0>> BDC 0 0 m 1 1 l S EMC",
        data,
    );
    if let Ok(doc) = Document::from_bytes(pdf) {
        let _ = doc.structure_tree();
    }
});
