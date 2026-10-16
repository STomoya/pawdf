/// A lexical token (ISO 32000-2 §7.2, §7.3).
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// Integer.
    Int(i64),
    /// Real number.
    Real(f64),
    /// Literal or hex string, already decoded.
    String(Vec<u8>),
    /// Name without the leading `/`, `#xx` escapes decoded.
    Name(Vec<u8>),
    /// `[`
    ArrayOpen,
    /// `]`
    ArrayClose,
    /// `<<`
    DictOpen,
    /// `>>`
    DictClose,
    /// Any other run of regular characters (`obj`, `true`, operators, ...), or a stray
    /// delimiter (`)`, `>`, `{`, `}`) as a single byte.
    Keyword(Vec<u8>),
}

/// §7.2.3 Table 1.
pub fn is_whitespace(b: u8) -> bool {
    matches!(b, 0 | 9 | 10 | 12 | 13 | 32)
}

/// §7.2.3 Table 2.
pub fn is_delimiter(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

/// Neither whitespace nor a delimiter (§7.2.3).
pub fn is_regular(b: u8) -> bool {
    !is_whitespace(b) && !is_delimiter(b)
}

/// A tokenizer over a byte slice. It never fails: malformed input becomes the closest
/// sensible token.
#[derive(Debug, Clone)]
pub struct Lexer<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    /// A lexer at the start of `data`.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// A lexer starting at `pos` (clamped to the end of `data`).
    pub fn at(data: &'a [u8], pos: usize) -> Self {
        Self {
            data,
            pos: pos.min(data.len()),
        }
    }

    /// The input.
    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    /// Current byte offset.
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Moves to `pos` (clamped to the end of the input).
    pub fn set_pos(&mut self, pos: usize) {
        self.pos = pos.min(self.data.len());
    }

    /// Skips whitespace and `%` comments.
    pub fn skip_whitespace(&mut self) {
        while let Some(&b) = self.data.get(self.pos) {
            if is_whitespace(b) {
                self.pos += 1;
            } else if b == b'%' {
                while let Some(&c) = self.data.get(self.pos) {
                    if c == b'\n' || c == b'\r' {
                        break;
                    }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    /// The next token and its start offset, or `None` at end of input.
    pub fn next_token(&mut self) -> Option<(usize, Token)> {
        self.skip_whitespace();
        let start = self.pos;
        let b = *self.data.get(start)?;
        self.pos += 1;
        let tok = match b {
            b'[' => Token::ArrayOpen,
            b']' => Token::ArrayClose,
            b'(' => Token::String(self.literal_string()),
            b'/' => Token::Name(self.name()),
            b'<' if self.data.get(self.pos) == Some(&b'<') => {
                self.pos += 1;
                Token::DictOpen
            }
            b'<' => Token::String(self.hex_string()),
            b'>' if self.data.get(self.pos) == Some(&b'>') => {
                self.pos += 1;
                Token::DictClose
            }
            b')' | b'>' | b'{' | b'}' => Token::Keyword(vec![b]),
            _ => {
                while self.data.get(self.pos).is_some_and(|&c| is_regular(c)) {
                    self.pos += 1;
                }
                let word = &self.data[start..self.pos];
                parse_number(word).unwrap_or_else(|| Token::Keyword(word.to_vec()))
            }
        };
        Some((start, tok))
    }

    /// The next token without consuming it.
    pub fn peek_token(&self) -> Option<(usize, Token)> {
        self.clone().next_token()
    }

    /// §7.3.4.2. Called after the opening `(`. An unterminated string runs to end of input.
    fn literal_string(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut depth = 1usize;
        while let Some(&b) = self.data.get(self.pos) {
            self.pos += 1;
            match b {
                b'(' => {
                    depth += 1;
                    out.push(b);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(b);
                }
                b'\\' => self.escape(&mut out),
                b'\r' => {
                    if self.data.get(self.pos) == Some(&b'\n') {
                        self.pos += 1;
                    }
                    out.push(b'\n');
                }
                _ => out.push(b),
            }
        }
        out
    }

    fn escape(&mut self, out: &mut Vec<u8>) {
        let Some(&b) = self.data.get(self.pos) else {
            return;
        };
        self.pos += 1;
        match b {
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'0'..=b'7' => {
                let mut v = u32::from(b - b'0');
                for _ in 0..2 {
                    match self.data.get(self.pos) {
                        Some(&d @ b'0'..=b'7') => {
                            v = v * 8 + u32::from(d - b'0');
                            self.pos += 1;
                        }
                        _ => break,
                    }
                }
                // High-order overflow is ignored (§7.3.4.2).
                out.push((v & 0xff) as u8);
            }
            b'\r' => {
                if self.data.get(self.pos) == Some(&b'\n') {
                    self.pos += 1;
                }
            }
            b'\n' => {}
            // Covers `\(`, `\)`, `\\`, and unknown escapes, where the backslash is dropped.
            _ => out.push(b),
        }
    }

    /// §7.3.4.3. Called after `<`. Invalid characters are skipped; an odd final digit
    /// is padded with 0.
    fn hex_string(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut hi: Option<u8> = None;
        while let Some(&b) = self.data.get(self.pos) {
            self.pos += 1;
            if b == b'>' {
                break;
            }
            let Some(v) = hex_value(b) else { continue };
            match hi.take() {
                Some(h) => out.push(h << 4 | v),
                None => hi = Some(v),
            }
        }
        if let Some(h) = hi {
            out.push(h << 4);
        }
        out
    }

    /// §7.3.5. Called after `/`. A `#` not followed by two hex digits is kept literally.
    fn name(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        while let Some(&b) = self.data.get(self.pos) {
            if !is_regular(b) {
                break;
            }
            self.pos += 1;
            if b == b'#' {
                let hi = self.data.get(self.pos).copied().and_then(hex_value);
                let lo = self.data.get(self.pos + 1).copied().and_then(hex_value);
                if let (Some(h), Some(l)) = (hi, lo) {
                    out.push(h << 4 | l);
                    self.pos += 2;
                    continue;
                }
            }
            out.push(b);
        }
        out
    }
}

/// Value of an ASCII hex digit.
pub fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// §7.3.3: `[+-]? (digits [. digits*] | . digits)`. Integers that overflow `i64` become reals.
fn parse_number(word: &[u8]) -> Option<Token> {
    let body = match word.first() {
        Some(b'+' | b'-') => &word[1..],
        _ => word,
    };
    let mut digits = 0;
    let mut dots = 0;
    for &b in body {
        match b {
            b'0'..=b'9' => digits += 1,
            b'.' => dots += 1,
            _ => return None,
        }
    }
    if digits == 0 || dots > 1 {
        return None;
    }
    let text = std::str::from_utf8(word).ok()?;
    if dots == 0
        && let Ok(i) = text.parse::<i64>()
    {
        return Some(Token::Int(i));
    }
    text.parse::<f64>().ok().map(Token::Real)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(input: &[u8]) -> Vec<Token> {
        let mut lex = Lexer::new(input);
        let mut out = Vec::new();
        while let Some((_, t)) = lex.next_token() {
            out.push(t);
        }
        out
    }

    fn kw(s: &str) -> Token {
        Token::Keyword(s.as_bytes().to_vec())
    }

    #[test]
    fn numbers() {
        assert_eq!(
            tokens(b"1 -2 +3 4.5 -.5 6. 99999999999999999999"),
            vec![
                Token::Int(1),
                Token::Int(-2),
                Token::Int(3),
                Token::Real(4.5),
                Token::Real(-0.5),
                Token::Real(6.0),
                Token::Real(1e20),
            ]
        );
    }

    #[test]
    fn non_numbers_become_keywords() {
        assert_eq!(
            tokens(b"1.2.3 - . inf"),
            vec![kw("1.2.3"), kw("-"), kw("."), kw("inf")]
        );
    }

    #[test]
    fn literal_strings() {
        assert_eq!(
            tokens(b"(a(b)c) (\\n\\(\\101\\0053) (x\\\ny) (l\r\nm)"),
            vec![
                Token::String(b"a(b)c".to_vec()),
                Token::String(b"\n(A\x053".to_vec()),
                Token::String(b"xy".to_vec()),
                Token::String(b"l\nm".to_vec()),
            ]
        );
    }

    #[test]
    fn unterminated_string_runs_to_end() {
        assert_eq!(tokens(b"(abc"), vec![Token::String(b"abc".to_vec())]);
    }

    #[test]
    fn hex_strings() {
        assert_eq!(
            tokens(b"<48 65 6c6C6f> <7> <zz41>"),
            vec![
                Token::String(b"Hello".to_vec()),
                Token::String(vec![0x70]),
                Token::String(b"A".to_vec()),
            ]
        );
    }

    #[test]
    fn names() {
        assert_eq!(
            tokens(b"/Type /A#20B /C#zz / /D"),
            vec![
                Token::Name(b"Type".to_vec()),
                Token::Name(b"A B".to_vec()),
                Token::Name(b"C#zz".to_vec()),
                Token::Name(vec![]),
                Token::Name(b"D".to_vec()),
            ]
        );
    }

    #[test]
    fn delimiters_and_comments() {
        assert_eq!(
            tokens(b"<</A [1]>> % comment\n) } true"),
            vec![
                Token::DictOpen,
                Token::Name(b"A".to_vec()),
                Token::ArrayOpen,
                Token::Int(1),
                Token::ArrayClose,
                Token::DictClose,
                kw(")"),
                kw("}"),
                kw("true"),
            ]
        );
    }

    #[test]
    fn offsets_point_at_token_start() {
        let mut lex = Lexer::new(b"  /A  12");
        assert_eq!(lex.next_token(), Some((2, Token::Name(b"A".to_vec()))));
        assert_eq!(lex.next_token(), Some((6, Token::Int(12))));
        assert_eq!(lex.next_token(), None);
    }
}
