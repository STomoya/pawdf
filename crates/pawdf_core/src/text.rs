//! Text strings (§7.9.2.2).

/// PDFDocEncoding (Annex D) for bytes `0x18..=0x1F`.
const ACCENTS: [char; 8] = ['˘', 'ˇ', 'ˆ', '˙', '˝', '˛', '˚', '˜'];
/// PDFDocEncoding for bytes `0x80..=0x9E`.
const HIGH: [char; 31] = [
    '•', '†', '‡', '…', '—', '–', 'ƒ', '⁄', '‹', '›', '−', '‰', '„', '“', '”', '‘', '’', '‚', '™',
    'ﬁ', 'ﬂ', 'Ł', 'Œ', 'Š', 'Ÿ', 'Ž', 'ı', 'ł', 'œ', 'š', 'ž',
];

/// Decodes a PDF text string (§7.9.2.2): UTF-16BE after a `FE FF` mark, UTF-8 after an
/// `EF BB BF` mark, otherwise PDFDocEncoding. Invalid sequences become U+FFFD, and the
/// language escapes of the Unicode forms (`U+001B` … `U+001B`) are removed.
pub fn decode_text_string(bytes: &[u8]) -> String {
    let unicode = if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let (units, _) = rest.as_chunks::<2>();
        char::decode_utf16(units.iter().map(|&u| u16::from_be_bytes(u)))
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect::<String>()
    } else if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(rest).into_owned()
    } else {
        return bytes.iter().map(|&b| pdf_doc_char(b)).collect();
    };
    // An unterminated escape swallows the rest of the string.
    unicode.split('\u{1B}').step_by(2).collect::<String>()
}

fn pdf_doc_char(b: u8) -> char {
    match b {
        0x18..=0x1F => ACCENTS[usize::from(b - 0x18)],
        0x80..=0x9E => HIGH[usize::from(b - 0x80)],
        0xA0 => '€',
        0x7F | 0x9F | 0xAD => char::REPLACEMENT_CHARACTER,
        _ => char::from(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_string_forms() {
        let cases: [(&[u8], &str); 9] = [
            (b"\xFE\xFF\x00\x41\xD8\x3D\xDE\x00", "A😀"),
            (b"\xFE\xFF\x00\x41\xD8\x3D", "A\u{FFFD}"),
            (b"\xFE\xFF\x00\x41\x00", "A"),
            (b"\xEF\xBB\xBF\xC3\xA9", "é"),
            (b"\x80\x93A\xa0\xe9", "•ﬁA€é"),
            (b"\x18\x9f", "\u{2D8}\u{FFFD}"),
            (b"\xFE\xFF\x00\x1B\x00\x6A\x00\x61\x00\x1B\x00\x41", "A"),
            (b"\xFE\xFF\x00\x41\x00\x1B\x00\x6A", "A"),
            (b"", ""),
        ];
        for (input, want) in cases {
            assert_eq!(decode_text_string(input), want, "{input:02X?}");
        }
    }
}
