#![no_main]

use libfuzzer_sys::fuzz_target;
use pawdf_core::{Diagnostics, Lexer, Origin, parse_object};

fuzz_target!(|data: &[u8]| {
    let mut lex = Lexer::new(data);
    while lex.next_token().is_some() {}
    let mut lex = Lexer::new(data);
    let mut diags = Diagnostics::new();
    while parse_object(&mut lex, Origin::File, &mut diags).is_some() {}
});
