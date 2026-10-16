#![no_main]

use libfuzzer_sys::fuzz_target;
use pawdf_core::{Diagnostics, Dict, Location, Name, Object, Stream};

const FILTERS: [&str; 5] = [
    "FlateDecode",
    "LZWDecode",
    "ASCIIHexDecode",
    "ASCII85Decode",
    "RunLengthDecode",
];

// Byte 0 picks the filter, bytes 1-4 pick predictor parameters, the rest is the data.
fuzz_target!(|data: &[u8]| {
    let [f, predictor, columns, colors, bpc, rest @ ..] = data else {
        return;
    };
    let mut params = Dict::new();
    params.insert(
        Name::from("Predictor"),
        Object::Int(i64::from(*predictor % 16)),
    );
    params.insert(Name::from("Columns"), Object::Int(i64::from(*columns)));
    params.insert(Name::from("Colors"), Object::Int(i64::from(*colors % 5)));
    params.insert(Name::from("BitsPerComponent"), Object::Int(1 << (bpc % 5)));
    let mut dict = Dict::new();
    dict.insert(
        Name::from("Filter"),
        Object::Name(Name::from(FILTERS[usize::from(*f) % FILTERS.len()])),
    );
    dict.insert(Name::from("DecodeParms"), Object::Dict(params));
    Stream::new(dict, rest.to_vec()).decode(Location::File(0), &mut Diagnostics::new());
});
