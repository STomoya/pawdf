#![no_main]

#[path = "common.rs"]
mod common;

use libfuzzer_sys::fuzz_target;
use pawdf_reader::Document;

// Input: CMap source, then 0xFF, then the bytes to show. The low bit of the first byte
// selects Identity-V instead of the CMap as the encoding.
fuzz_target!(|data: &[u8]| {
    let (cmap, shown) = match data.iter().position(|&b| b == 0xFF) {
        Some(i) => (&data[..i], &data[i + 1..]),
        None => (data, &[][..]),
    };
    let vertical = data.first().is_some_and(|b| b & 1 == 1);
    if let Ok(doc) = Document::from_bytes(common::type0_page(cmap, shown, vertical))
        && let Ok(page) = doc.page(0)
    {
        let _ = page.content();
    }
});
