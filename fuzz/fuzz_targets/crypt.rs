#![no_main]

mod common;

use libfuzzer_sys::fuzz_target;
use pawdf_core::{Diagnostics, File, Location, Object, Ref};

// The input is `password \0 encryption dictionary`.
fuzz_target!(|data: &[u8]| {
    let (password, dict) = match data.iter().position(|&b| b == 0) {
        Some(i) => (&data[..i], &data[i + 1..]),
        None => (&b""[..], data),
    };
    let password = String::from_utf8_lossy(password);
    let Ok(file) = File::from_bytes_with_password(common::encrypted(dict), &password) else {
        return;
    };
    let _ = file.resolve(Ref { num: 2, generation: 0 });
    if let Object::Stream(s) = file.resolve(Ref { num: 3, generation: 0 }) {
        s.decode(Location::File(0), &mut Diagnostics::new());
    }
    let _ = file.encryption();
});
