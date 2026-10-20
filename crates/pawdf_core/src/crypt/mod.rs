//! Standard security handler (ISO 32000-2 §7.6.4): the encryption dictionary, password
//! authentication, and per-object decryption.

mod cipher;
mod key;

pub use key::PasswordKind;
pub(crate) use key::authenticate;

use crate::diag::{DiagnosticKind, Diagnostics, Location};
use crate::object::{Dict, Object, Ref, Stream};

/// How one crypt filter decrypts (§7.6.6 Table 25 `/CFM`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CryptFilter {
    Identity,
    Rc4,
    AesV2,
    AesV3,
    Unknown,
}

/// The parsed encryption dictionary of the standard security handler (Tables 20 and 21).
pub(crate) struct SecurityHandler {
    pub(crate) v: u8,
    pub(crate) r: u8,
    /// File key length in bytes: 5..=16 for V1–4, 32 for V5.
    pub(crate) key_len: usize,
    pub(crate) o: Vec<u8>,
    pub(crate) u: Vec<u8>,
    pub(crate) oe: Vec<u8>,
    pub(crate) ue: Vec<u8>,
    pub(crate) perms: Vec<u8>,
    /// The low 32 bits of `/P`.
    pub(crate) p: u32,
    pub(crate) encrypt_metadata: bool,
    pub(crate) stm_f: CryptFilter,
    pub(crate) str_f: CryptFilter,
    pub(crate) eff: CryptFilter,
    /// `/CF` entries by name.
    pub(crate) filters: Vec<(Vec<u8>, CryptFilter)>,
}

impl SecurityHandler {
    /// Parses an encryption dictionary. `Err` names what is unsupported, for
    /// `ErrorKind::UnsupportedEncryption`.
    pub(crate) fn parse(dict: &Dict, diags: &mut Diagnostics) -> Result<SecurityHandler, String> {
        match dict.get(b"Filter").and_then(Object::as_name) {
            Some(b"Standard") => {}
            Some(other) => return Err(String::from_utf8_lossy(other).into_owned()),
            None => return Err("missing /Filter".into()),
        }
        let int = |key: &[u8]| dict.get(key).and_then(Object::as_int);
        let v = int(b"V").unwrap_or(0);
        let r = int(b"R").unwrap_or(0);
        let r_ok = match v {
            1..=3 => matches!(r, 2 | 3),
            4 => r == 4,
            5 => matches!(r, 5 | 6),
            _ => return Err(format!("Standard V{v}")),
        };
        if !r_ok {
            return Err(format!("Standard R{r}"));
        }
        let key_len = match v {
            1 => 5,
            5 => 32,
            // Table 20: /Length is in bits, a multiple of 8 in 40..=128; absent means 40 for
            // V2/V3 and 128 for V4.
            _ => {
                let default = if v == 4 { 128 } else { 40 };
                let bits = dict.get(b"Length").map_or(Some(default), Object::as_int);
                let fixed = match bits {
                    Some(b) if v == 4 && !(40..=128).contains(&b) => default,
                    Some(b) => (b / 8 * 8).clamp(40, 128),
                    None => default,
                };
                if bits != Some(fixed) {
                    diags.push(DiagnosticKind::BadEncryption, Location::File(0));
                }
                fixed as usize / 8
            }
        };
        let filters: Vec<(Vec<u8>, CryptFilter)> = match dict.get(b"CF") {
            Some(Object::Dict(cf)) if v >= 4 => cf
                .iter()
                .map(|(k, f)| (k.as_bytes().to_vec(), cfm(f)))
                .collect(),
            _ => Vec::new(),
        };
        let mut h = SecurityHandler {
            v: v as u8,
            r: r as u8,
            key_len,
            o: string(dict, b"O"),
            u: string(dict, b"U"),
            oe: string(dict, b"OE"),
            ue: string(dict, b"UE"),
            perms: string(dict, b"Perms"),
            p: int(b"P").unwrap_or(0) as u32,
            encrypt_metadata: dict
                .get(b"EncryptMetadata")
                .and_then(Object::as_bool)
                .unwrap_or(true),
            stm_f: CryptFilter::Rc4,
            str_f: CryptFilter::Rc4,
            eff: CryptFilter::Rc4,
            filters,
        };
        if v >= 4 {
            h.stm_f = default_filter(&h, dict, b"StmF")?;
            h.str_f = default_filter(&h, dict, b"StrF")?;
            h.eff = match dict.get(b"EFF").and_then(Object::as_name) {
                Some(name) => h.filter(name),
                None => h.stm_f,
            };
        }
        Ok(h)
    }

    /// The crypt filter named `name`: `Identity`, or a `/CF` entry; `Unknown` if absent.
    pub(crate) fn filter(&self, name: &[u8]) -> CryptFilter {
        if name == b"Identity" {
            return CryptFilter::Identity;
        }
        self.filters
            .iter()
            .find(|(n, _)| n == name)
            .map_or(CryptFilter::Unknown, |&(_, f)| f)
    }
}

/// The method used to encrypt a file's strings and streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Nothing is encrypted (`StmF` and `StrF` are `Identity`).
    Identity,
    /// RC4 (V1–V4, crypt filter method `V2`).
    Rc4,
    /// AES-128 (`AESV2`).
    Aes128,
    /// AES-256 (`AESV3`).
    Aes256,
}

/// The user access permissions `/P` (§7.6.4.2, Table 22). Reported, never enforced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permissions(u32);

impl Permissions {
    /// The raw `/P` value, as an unsigned 32-bit integer.
    pub fn bits(self) -> u32 {
        self.0
    }

    fn bit(self, n: u32) -> bool {
        self.0 & (1 << (n - 1)) != 0
    }

    /// Bit 3: print (at low quality unless [`Permissions::print_high_quality`]).
    pub fn print(self) -> bool {
        self.bit(3)
    }

    /// Bit 4: modify the contents.
    pub fn modify(self) -> bool {
        self.bit(4)
    }

    /// Bit 5: copy or extract text and graphics.
    pub fn copy(self) -> bool {
        self.bit(5)
    }

    /// Bit 6: add or modify annotations and fill forms.
    pub fn annotate(self) -> bool {
        self.bit(6)
    }

    /// Bit 9: fill in form fields, even when [`Permissions::annotate`] is not set.
    pub fn fill_forms(self) -> bool {
        self.bit(9)
    }

    /// Bit 10: extract text and graphics for accessibility.
    pub fn extract_for_accessibility(self) -> bool {
        self.bit(10)
    }

    /// Bit 11: assemble the document (insert, rotate, delete pages, bookmarks).
    pub fn assemble(self) -> bool {
        self.bit(11)
    }

    /// Bit 12: print at full quality.
    pub fn print_high_quality(self) -> bool {
        self.bit(12)
    }
}

/// How an encrypted file was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Encryption {
    /// The standard security handler revision `/R` (2–6).
    pub revision: u8,
    /// The first method of `/StmF`, `/StrF` and `/EFF` that is not `Identity`.
    pub method: Method,
    /// The file key length in bits.
    pub key_bits: u16,
    /// The permission flags.
    pub permissions: Permissions,
    /// Which password matched.
    pub authenticated_as: PasswordKind,
}

/// An authenticated security handler: decrypts objects as they are loaded.
pub(crate) struct Crypt {
    pub(crate) handler: SecurityHandler,
    key: Vec<u8>,
    pub(crate) kind: PasswordKind,
    /// The encryption dictionary's own object, whose strings are not encrypted (§7.6.2).
    encrypt_ref: Option<Ref>,
}

impl Crypt {
    pub(crate) fn new(
        handler: SecurityHandler,
        key: Vec<u8>,
        kind: PasswordKind,
        encrypt_ref: Option<Ref>,
    ) -> Crypt {
        Crypt {
            handler,
            key,
            kind,
            encrypt_ref,
        }
    }

    pub(crate) fn info(&self) -> Encryption {
        let h = &self.handler;
        let method = [h.stm_f, h.str_f, h.eff]
            .into_iter()
            .find_map(|f| match f {
                CryptFilter::Rc4 => Some(Method::Rc4),
                CryptFilter::AesV2 => Some(Method::Aes128),
                CryptFilter::AesV3 => Some(Method::Aes256),
                CryptFilter::Identity | CryptFilter::Unknown => None,
            })
            .unwrap_or(Method::Identity);
        Encryption {
            revision: h.r,
            method,
            key_bits: (h.key_len * 8) as u16,
            permissions: Permissions(h.p),
            authenticated_as: self.kind,
        }
    }

    /// Whether object `num` is the encryption dictionary.
    pub(crate) fn is_encrypt_dict(&self, num: u32) -> bool {
        self.encrypt_ref.is_some_and(|r| r.num == num)
    }

    /// Decrypts every string in `obj` (recursively, a stream's dictionary included) with
    /// `/StrF`. `r` is the object's own `N G obj` header.
    pub(crate) fn decrypt_strings(
        &self,
        r: Ref,
        obj: &mut Object,
        at: Location,
        diags: &mut Diagnostics,
    ) {
        match obj {
            Object::String(s) => {
                *s = match cipher::decrypt(&self.key, self.handler.str_f, r, s) {
                    Ok(plain) => plain,
                    Err(partial) => {
                        diags.push(DiagnosticKind::BadEncryption, at);
                        partial
                    }
                };
            }
            Object::Array(items) => {
                for item in items {
                    self.decrypt_strings(r, item, at, diags);
                }
            }
            Object::Dict(d) => self.decrypt_dict_strings(r, d, at, diags),
            Object::Stream(s) => self.decrypt_dict_strings(r, &mut s.dict, at, diags),
            _ => {}
        }
    }

    fn decrypt_dict_strings(&self, r: Ref, d: &mut Dict, at: Location, diags: &mut Diagnostics) {
        // §7.6.2: a signature's /Contents is not encrypted. Some writers omit /Type, so
        // /ByteRange marks one too (as in qpdf).
        let sig = matches!(
            d.get(b"Type").and_then(Object::as_name),
            Some(b"Sig" | b"DocTimeStamp")
        ) || d.contains_key(b"ByteRange");
        for (k, v) in d.iter_mut() {
            if !(sig && k.as_bytes() == b"Contents") {
                self.decrypt_strings(r, v, at, diags);
            }
        }
    }

    /// Decrypts a stream's raw bytes with its own `/Crypt` filter (then removed from the
    /// filter chain), `/EFF` for embedded files, or `/StmF`. Cross-reference streams, and
    /// metadata when `/EncryptMetadata` is false, are left alone (§7.6.2, §7.5.8.2).
    pub(crate) fn decrypt_stream(
        &self,
        r: Ref,
        s: &mut Stream,
        at: Location,
        diags: &mut Diagnostics,
    ) {
        let h = &self.handler;
        let method = match s.dict.get(b"Type").and_then(Object::as_name) {
            Some(b"XRef") => return,
            Some(b"Metadata") if !h.encrypt_metadata => return,
            ty => {
                let embedded = ty == Some(b"EmbeddedFile");
                match take_crypt_filter(&mut s.dict) {
                    Some(name) => h.filter(&name),
                    None if embedded => h.eff,
                    None => h.stm_f,
                }
            }
        };
        if method == CryptFilter::Unknown {
            diags.push(DiagnosticKind::BadEncryption, at);
            return;
        }
        s.raw = match cipher::decrypt(&self.key, method, r, &s.raw) {
            Ok(plain) => plain,
            Err(partial) => {
                diags.push(DiagnosticKind::BadEncryption, at);
                partial
            }
        };
    }
}

/// If the first `/Filter` is `/Crypt` (§7.4.10), removes it and its `/DecodeParms` slot and
/// returns the crypt filter's `/Name` (default `Identity`).
fn take_crypt_filter(dict: &mut Dict) -> Option<Vec<u8>> {
    let first = |o: &Object| match o {
        Object::Array(a) => a.first().cloned(),
        o => Some(o.clone()),
    };
    let filter = dict.get(b"Filter").and_then(first)?;
    if filter.as_name() != Some(b"Crypt") {
        return None;
    }
    let params = dict.get(b"DecodeParms").and_then(first);
    let name = params
        .as_ref()
        .and_then(Object::as_dict)
        .and_then(|p| p.get(b"Name"))
        .and_then(Object::as_name)
        .unwrap_or(b"Identity")
        .to_vec();
    for key in [&b"Filter"[..], b"DecodeParms"] {
        let rest = match dict.remove(key) {
            Some(Object::Array(mut a)) if !a.is_empty() => {
                a.remove(0);
                a
            }
            _ => continue,
        };
        match rest.len() {
            0 => {}
            1 => dict.insert(key.into(), rest.into_iter().next().unwrap_or(Object::Null)),
            _ => dict.insert(key.into(), Object::Array(rest)),
        }
    }
    Some(name)
}

/// `/StmF` or `/StrF` (default `Identity`). Nothing could be read with an unknown default
/// method, so it is unsupported rather than a per-object diagnostic.
fn default_filter(h: &SecurityHandler, dict: &Dict, key: &[u8]) -> Result<CryptFilter, String> {
    let name = dict
        .get(key)
        .and_then(Object::as_name)
        .unwrap_or(b"Identity");
    let f = h.filter(name);
    if f != CryptFilter::Unknown {
        return Ok(f);
    }
    let cfm = dict
        .get(b"CF")
        .and_then(Object::as_dict)
        .and_then(|cf| cf.get(name))
        .and_then(Object::as_dict)
        .and_then(|f| f.get(b"CFM"))
        .and_then(Object::as_name);
    let what = String::from_utf8_lossy(cfm.unwrap_or(name));
    Err(format!("Standard {} {what}", String::from_utf8_lossy(key)))
}

/// A wrong type is read as empty, so a malformed value fails authentication later.
fn string(dict: &Dict, key: &[u8]) -> Vec<u8> {
    dict.get(key)
        .and_then(Object::as_string)
        .unwrap_or_default()
        .to_vec()
}

fn cfm(filter: &Object) -> CryptFilter {
    // Table 25: /CFM defaults to /None, meaning the data is left as is.
    match filter
        .as_dict()
        .and_then(|f| f.get(b"CFM"))
        .and_then(Object::as_name)
    {
        None | Some(b"None") => CryptFilter::Identity,
        Some(b"V2") => CryptFilter::Rc4,
        Some(b"AESV2") => CryptFilter::AesV2,
        Some(b"AESV3") => CryptFilter::AesV3,
        Some(_) => CryptFilter::Unknown,
    }
}

#[cfg(test)]
pub(crate) fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::Name;

    fn name(s: &str) -> Object {
        Object::Name(Name::from(s))
    }

    fn dict(entries: &[(&str, Object)]) -> Dict {
        let mut d = Dict::new();
        for (k, v) in entries {
            d.insert(Name::from(*k), v.clone());
        }
        d
    }

    fn standard(extra: &[(&str, Object)]) -> Dict {
        let mut d = dict(&[("Filter", name("Standard"))]);
        for (k, v) in extra {
            d.insert(Name::from(*k), v.clone());
        }
        d
    }

    fn cf(cfm: &str, length: Option<i64>) -> Object {
        let mut f = dict(&[("CFM", name(cfm))]);
        if let Some(l) = length {
            f.insert("Length".into(), Object::Int(l));
        }
        Object::Dict(dict(&[("StdCF", Object::Dict(f))]))
    }

    fn bad_encryption(d: &Diagnostics) -> usize {
        d.as_slice()
            .iter()
            .filter(|x| x.kind == DiagnosticKind::BadEncryption)
            .count()
    }

    #[test]
    fn key_length_by_version() {
        let v4 = |len| {
            standard(&[
                ("V", Object::Int(4)),
                ("R", Object::Int(4)),
                ("CF", cf("AESV2", Some(len))),
                ("StmF", name("StdCF")),
                ("StrF", name("StdCF")),
            ])
        };
        let cases: Vec<(Dict, usize, usize)> = vec![
            (
                standard(&[("V", Object::Int(1)), ("R", Object::Int(2))]),
                5,
                0,
            ),
            (
                standard(&[
                    ("V", Object::Int(2)),
                    ("R", Object::Int(3)),
                    ("Length", Object::Int(128)),
                ]),
                16,
                0,
            ),
            (
                standard(&[("V", Object::Int(2)), ("R", Object::Int(3))]),
                5,
                0,
            ),
            (
                standard(&[
                    ("V", Object::Int(2)),
                    ("R", Object::Int(3)),
                    ("Length", Object::Int(200)),
                ]),
                16,
                1,
            ),
            (
                standard(&[
                    ("V", Object::Int(2)),
                    ("R", Object::Int(3)),
                    ("Length", Object::Int(44)),
                ]),
                5,
                1,
            ),
            (v4(16), 16, 0),
            (v4(128), 16, 0),
            (
                standard(&[
                    ("V", Object::Int(5)),
                    ("R", Object::Int(6)),
                    ("CF", cf("AESV3", Some(32))),
                    ("StmF", name("StdCF")),
                    ("StrF", name("StdCF")),
                ]),
                32,
                0,
            ),
        ];
        for (i, (d, key_len, diags)) in cases.into_iter().enumerate() {
            let mut dg = Diagnostics::new();
            let h = SecurityHandler::parse(&d, &mut dg).expect("supported");
            assert_eq!(h.key_len, key_len, "case {i}");
            assert_eq!(bad_encryption(&dg), diags, "case {i}");
        }
    }

    #[test]
    fn crypt_filters_resolve() {
        let mut dg = Diagnostics::new();
        let base = [
            ("V", Object::Int(4)),
            ("R", Object::Int(4)),
            ("CF", cf("AESV2", None)),
        ];
        let mut d = standard(&base);
        d.insert("StmF".into(), name("StdCF"));
        d.insert("StrF".into(), name("Identity"));
        let h = SecurityHandler::parse(&d, &mut dg).expect("supported");
        assert_eq!(h.stm_f, CryptFilter::AesV2);
        assert_eq!(h.str_f, CryptFilter::Identity);
        assert_eq!(h.eff, CryptFilter::AesV2);
        assert_eq!(h.filter(b"Missing"), CryptFilter::Unknown);
        assert_eq!(h.filter(b"Identity"), CryptFilter::Identity);

        let mut d = standard(&base);
        d.insert("StmF".into(), name("Identity"));
        d.insert("EFF".into(), name("StdCF"));
        let h = SecurityHandler::parse(&d, &mut dg).expect("supported");
        assert_eq!(h.stm_f, CryptFilter::Identity);
        assert_eq!(h.str_f, CryptFilter::Identity);
        assert_eq!(h.eff, CryptFilter::AesV2);

        let d = standard(&[("V", Object::Int(2)), ("R", Object::Int(3))]);
        let h = SecurityHandler::parse(&d, &mut dg).expect("supported");
        assert_eq!(
            (h.stm_f, h.str_f, h.eff),
            (CryptFilter::Rc4, CryptFilter::Rc4, CryptFilter::Rc4)
        );
        assert!(dg.is_empty());
    }

    #[test]
    fn unsupported_handlers_are_named() {
        let err = |d: Dict| SecurityHandler::parse(&d, &mut Diagnostics::new()).err();
        let some = |s: &str| Some(s.to_string());
        assert_eq!(
            err(dict(&[("Filter", name("Adobe.PubSec"))])),
            some("Adobe.PubSec")
        );
        assert_eq!(
            err(dict(&[("Filter", name("FOPN_fLock"))])),
            some("FOPN_fLock")
        );
        assert_eq!(err(dict(&[("V", Object::Int(1))])), some("missing /Filter"));
        assert_eq!(
            err(standard(&[("V", Object::Int(4)), ("R", Object::Int(7))])),
            some("Standard R7")
        );
        assert_eq!(err(standard(&[])), some("Standard V0"));
        assert_eq!(
            err(standard(&[
                ("V", Object::Int(4)),
                ("R", Object::Int(4)),
                ("StmF", name("Foo")),
            ])),
            some("Standard StmF Foo")
        );
        assert_eq!(
            err(standard(&[
                ("V", Object::Int(4)),
                ("R", Object::Int(4)),
                ("CF", cf("Weird", None)),
                ("StmF", name("StdCF")),
            ])),
            some("Standard StmF Weird")
        );
    }
}
