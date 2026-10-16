#![no_main]

use libfuzzer_sys::fuzz_target;
use pawdf_core::{Diagnostics, File, Location, Object};

fuzz_target!(|data: &[u8]| {
    let Ok(file) = File::from_bytes(data.to_vec()) else {
        return;
    };
    let mut diags = Diagnostics::new();
    for r in file.objects() {
        if let Object::Stream(s) = file.resolve(r) {
            s.decode(Location::File(0), &mut diags);
        }
    }
    let _ = file.diagnostics();
});
