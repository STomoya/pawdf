#![no_main]

use libfuzzer_sys::fuzz_target;
use pawdf_core::{Diagnostics, parse_indirect};

fuzz_target!(|data: &[u8]| {
    let mut input = b"1 0 obj ".to_vec();
    input.extend_from_slice(data);
    let _ = parse_indirect(&input, 0, &|_| None, &mut Diagnostics::new());
});
