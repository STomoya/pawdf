//! Password authentication and file keys (§7.6.4.3 and §7.6.4.4).

use md5::Md5;
use sha2::{Digest, Sha256, Sha384, Sha512};

use super::SecurityHandler;
use super::cipher::{aes128_cbc_encrypt, aes256_decrypt_zero_iv, aes256_ecb_decrypt, rc4};
use crate::diag::{DiagnosticKind, Diagnostics, Location};
use crate::text::encode_pdf_doc;

/// Which password opened an encrypted file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordKind {
    /// The user password (possibly the empty one).
    User,
    /// The owner password.
    Owner,
}

/// Algorithm 2 step (a).
const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08,
    0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

/// Tries each byte form of `password` as the user then the owner password, then the empty
/// user password. Returns the file key and which password matched. Pushes `BadEncryption`
/// only for a failed `/Perms` check.
pub(crate) fn authenticate(
    h: &SecurityHandler,
    id: &[u8],
    password: Option<&str>,
    diags: &mut Diagnostics,
) -> Option<(Vec<u8>, PasswordKind)> {
    let modern = h.r >= 5;
    let mut forms: Vec<Vec<u8>> = Vec::new();
    if let Some(pw) = password {
        let prepared = if modern {
            // §7.6.4.3.3: SASLprep, but some writers skip it, so raw UTF-8 is tried too.
            stringprep::saslprep(pw).ok().map(|p| p.as_bytes().to_vec())
        } else {
            encode_pdf_doc(pw)
        };
        forms.extend(prepared);
        if !forms.iter().any(|f| f == pw.as_bytes()) {
            forms.push(pw.as_bytes().to_vec());
        }
    }
    let attempts = forms
        .iter()
        .flat_map(|f| [(f.as_slice(), PasswordKind::User), (f, PasswordKind::Owner)])
        .chain([(&b""[..], PasswordKind::User)]);
    for (i, (pw, kind)) in attempts.enumerate() {
        let key = if modern {
            let pw = &pw[..pw.len().min(127)];
            modern_key(h, pw, kind)
        } else {
            match kind {
                PasswordKind::User => user_key(h, id, pw),
                PasswordKind::Owner => owner_key(h, id, pw),
            }
        };
        if let Some(key) = key {
            if modern && !h.perms.is_empty() && !perms_ok(h, &key) {
                diags.push(DiagnosticKind::BadEncryption, Location::File(0));
            }
            if password.is_some() && i == forms.len() * 2 {
                diags.push(DiagnosticKind::WrongPassword, Location::File(0));
            }
            return Some((key, kind));
        }
    }
    None
}

fn padded(pw: &[u8]) -> [u8; 32] {
    let mut out = PAD;
    let n = pw.len().min(32);
    out[..n].copy_from_slice(&pw[..n]);
    out[n..].copy_from_slice(&PAD[..32 - n]);
    out
}

/// Algorithm 2: the file key for a (user) password, R2–R4.
fn file_key(h: &SecurityHandler, id: &[u8], pw: &[u8]) -> Option<Vec<u8>> {
    let mut md5 = Md5::new();
    md5.update(padded(pw));
    md5.update(h.o.get(..32)?);
    md5.update(h.p.to_le_bytes());
    md5.update(id);
    if h.r >= 4 && !h.encrypt_metadata {
        md5.update([0xFF; 4]);
    }
    let mut hash = md5.finalize();
    let n = h.key_len;
    if h.r >= 3 {
        for _ in 0..50 {
            hash = Md5::digest(&hash[..n]);
        }
    }
    Some(hash[..n].to_vec())
}

/// Algorithm 6: authenticates a user password via Algorithm 4 (R2) or 5 (R3/R4).
fn user_key(h: &SecurityHandler, id: &[u8], pw: &[u8]) -> Option<Vec<u8>> {
    let key = file_key(h, id, pw)?;
    let ok = if h.r == 2 {
        h.u.get(..32) == Some(&rc4(&key, &PAD)[..])
    } else {
        let mut md5 = Md5::new();
        md5.update(PAD);
        md5.update(id);
        let mut x = rc4(&key, &md5.finalize());
        for i in 1..=19u8 {
            x = rc4(&xor_key(&key, i), &x);
        }
        h.u.get(..16) == Some(&x[..])
    };
    ok.then_some(key)
}

fn xor_key(key: &[u8], i: u8) -> Vec<u8> {
    key.iter().map(|b| b ^ i).collect()
}

/// Algorithm 7: recovers the user password from `/O` with the owner password (Algorithm 3
/// steps (a)–(d)), then authenticates it.
fn owner_key(h: &SecurityHandler, id: &[u8], pw: &[u8]) -> Option<Vec<u8>> {
    let mut hash = Md5::digest(padded(pw));
    if h.r >= 3 {
        for _ in 0..50 {
            hash = Md5::digest(hash);
        }
    }
    let n = h.key_len;
    let key = &hash[..n];
    let o = h.o.get(..32)?;
    let user = if h.r == 2 {
        rc4(key, o)
    } else {
        (0..=19u8)
            .rev()
            .fold(o.to_vec(), |x, i| rc4(&xor_key(key, i), &x))
    };
    user_key(h, id, &user)
}

/// Algorithms 2.A, 11 and 12: R5 and R6. `/U` and `/O` are a 32-byte hash, an 8-byte
/// validation salt, and an 8-byte key salt.
fn modern_key(h: &SecurityHandler, pw: &[u8], kind: PasswordKind) -> Option<Vec<u8>> {
    let u = h.u.get(..48)?;
    let (entry, udata, wrapped) = match kind {
        PasswordKind::User => (u, &[][..], &h.ue),
        PasswordKind::Owner => (h.o.get(..48)?, u, &h.oe),
    };
    if hash(h.r, pw, &entry[32..40], udata) != entry[..32] {
        return None;
    }
    let wrapping = hash(h.r, pw, &entry[40..48], udata);
    aes256_decrypt_zero_iv(&wrapping, wrapped.get(..32)?)
}

/// R5: SHA-256 of password ‖ salt ‖ udata. R6: Algorithm 2.B.
fn hash(r: u8, pw: &[u8], salt: &[u8], udata: &[u8]) -> Vec<u8> {
    let mut k = Sha256::new()
        .chain_update(pw)
        .chain_update(salt)
        .chain_update(udata)
        .finalize()
        .to_vec();
    if r == 5 {
        return k;
    }
    let mut round = 0u32;
    loop {
        let unit = [pw, &k, udata].concat();
        let k1 = unit.repeat(64);
        // k is always 32..=64 bytes here, and k1 a multiple of 64 bytes.
        let Some(e) = aes128_cbc_encrypt(&k[..16], &k[16..32], &k1) else {
            return k;
        };
        let sum: u32 = e[..16].iter().map(|&b| u32::from(b)).sum();
        k = match sum % 3 {
            0 => Sha256::digest(&e).to_vec(),
            1 => Sha384::digest(&e).to_vec(),
            _ => Sha512::digest(&e).to_vec(),
        };
        round += 1;
        let last = u32::from(e.last().copied().unwrap_or(0));
        if round >= 64 && last + 32 <= round {
            break;
        }
    }
    k.truncate(32);
    k
}

/// Algorithm 13: `/Perms` decrypts to `/P` (little-endian), `T`/`F` for `/EncryptMetadata` at
/// byte 8, and `adb` at bytes 9..12.
fn perms_ok(h: &SecurityHandler, key: &[u8]) -> bool {
    let Ok(block) = <[u8; 16]>::try_from(h.perms.as_slice()) else {
        return false;
    };
    let meta = if h.encrypt_metadata { b'T' } else { b'F' };
    aes256_ecb_decrypt(key, &block)
        .is_some_and(|p| p[9..12] == *b"adb" && p[..4] == h.p.to_le_bytes() && p[8] == meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypt::{CryptFilter, cipher, hex};
    use crate::diag::DiagnosticKind;
    use crate::object::Ref;
    use crate::text::encode_pdf_doc;

    struct Kat {
        label: &'static str,
        v: u8,
        r: u8,
        length: usize,
        cfm: &'static str,
        p: i64,
        encrypt_metadata: bool,
        o: &'static str,
        u: &'static str,
        oe: &'static str,
        ue: &'static str,
        perms: &'static str,
        id: &'static str,
        string_num: u32,
        string: &'static str,
        stream_num: u32,
        stream: &'static str,
    }

    // Generated by pikepdf 10.17 (libqpdf 12.4.2) with user "u", owner "o": the /Info
    // /Title string "Hello KAT" and the page content stream, both still encrypted.
    const KATS: &[Kat] = &[
        Kat {
            label: "R2",
            v: 1,
            r: 2,
            length: 40,
            cfm: "None",
            p: -12,
            encrypt_metadata: true,
            o: "853fee3f6550fc3bc212797eaed99cc9be53347583a738e25fdfb1242bf93366",
            u: "d233a934f8a794e352f70550015850f781b97a386736c78724ffbb51bebc0b59",
            oe: "",
            ue: "",
            perms: "",
            id: "31415926535897932384626433832795",
            string_num: 2,
            string: "19ef40d96635b07482",
            stream_num: 5,
            stream: "77960ca1215880442876ce4ef8a1729d9edce5da00dc4dbd",
        },
        Kat {
            label: "R3",
            v: 2,
            r: 3,
            length: 128,
            cfm: "None",
            p: -1028,
            encrypt_metadata: true,
            o: "2a2f0a1990192c60114730bdcd39f37828a53c89a340dd473c85299dc5258e1c",
            u: "a09e2aa09050292242ca44e3ea530b0e0021446990b9e4114071a4d9104984c1",
            oe: "",
            ue: "",
            perms: "",
            id: "31415926535897932384626433832795",
            string_num: 2,
            string: "662316621683ec06ab",
            stream_num: 5,
            stream: "07ef647c1af188eb3025234b26db7ebce99428cd00e7bd0c",
        },
        Kat {
            label: "R4_RC4",
            v: 4,
            r: 4,
            length: 128,
            cfm: "V2",
            p: -1028,
            encrypt_metadata: false,
            o: "2a2f0a1990192c60114730bdcd39f37828a53c89a340dd473c85299dc5258e1c",
            u: "365c703bf35b560dd3ae850d5acc67190021446990b9e4114071a4d9104984c1",
            oe: "",
            ue: "",
            perms: "",
            id: "31415926535897932384626433832795",
            string_num: 2,
            string: "dc1eb1e66d68be0a29",
            stream_num: 5,
            stream: "7ff84db467e1e960c49ae845a065063130936074a324dc32",
        },
        Kat {
            label: "R4_AES",
            v: 4,
            r: 4,
            length: 128,
            cfm: "AESV2",
            p: -1028,
            encrypt_metadata: true,
            o: "2a2f0a1990192c60114730bdcd39f37828a53c89a340dd473c85299dc5258e1c",
            u: "a09e2aa09050292242ca44e3ea530b0e0021446990b9e4114071a4d9104984c1",
            oe: "",
            ue: "",
            perms: "",
            id: "31415926535897932384626433832795",
            string_num: 2,
            string: "91129a4b973c0fc9429654415580045b9a29fc9d544693e52cccbc0a98c3abda",
            stream_num: 5,
            stream: "63be1f01a45e6bb7142f23396cebb6ea221f51bff0fb8cd7d3effeed5f378ff3737a6de833e7ff78145e6cd7ff4a59b3",
        },
        Kat {
            label: "R5",
            v: 5,
            r: 5,
            length: 256,
            cfm: "AESV3",
            p: -1028,
            encrypt_metadata: true,
            o: "803e4d21b0d42ed5028b23f0d9f0faa33fe28fb1fdb4b3bc08b0442b1290550b45722a6bead1c2d6b94f8929d72abc24",
            u: "f52ad0e76d5be4f3bafadc5ba2b63356f38a418e6200a17238191d01446e494b465d08fbdea83c8be64c170e0c0a28b7",
            oe: "d943be9cebc0498c5841e76f47987e945cf3a488a3ab062f3472ecec19fffb83",
            ue: "4d872930b272aed2bd7bbf077330235251bcc7886eb9d44637088a2e1ba3734c",
            perms: "f6a1f48e0d9bd312a2de34549580cdba",
            id: "31415926535897932384626433832795",
            string_num: 2,
            string: "64be783b63a5ef4b65aa5c39499d3fcaf3f4c8ec884ff97167b52d9262091a74",
            stream_num: 5,
            stream: "398b550ab44f923aad7ad042c2d91860e40cd071204e4fe6dedf0c767e6129cd314ff0b7ef8a23bec3a58f136fb1c692",
        },
        Kat {
            label: "R6",
            v: 5,
            r: 6,
            length: 256,
            cfm: "AESV3",
            p: -1028,
            encrypt_metadata: true,
            o: "032d448327f132a5de8700ae8fc5dcdce728f71bcf5e006f58369a833ef654c5c236fb4847c6c457f62d17dd48160adf",
            u: "aa7e624e63c80251c24affcc1c68f17dd89a77d6219a2b2728abe0c6c010345c1a1ccbc623e8097c103c4072d8c70094",
            oe: "a15024c8210101a9b6ac57bab9ed8c68725052ff912b02c7aa4ea8a0f8cd40a8",
            ue: "32dbec132621ec00d7cb6682b41f78ae402db7ba0a438457cf6b9ef543fd1a62",
            perms: "06aa5f81d15e9c3295fb1e203ef2140e",
            id: "31415926535897932384626433832795",
            string_num: 2,
            string: "d5f7506bb5f859c4051dc1b7f0d6d7314b3e016991e403b36cabc4870bb91f08",
            stream_num: 5,
            stream: "070967b33263f268b72c85dc1bf8740676b5d57a1f72e9a6bdfa24e6f7bf42f4d45abbcbc281b3b7add93944b1571122",
        },
    ];

    fn method(kat: &Kat) -> CryptFilter {
        match kat.cfm {
            "AESV2" => CryptFilter::AesV2,
            "AESV3" => CryptFilter::AesV3,
            _ => CryptFilter::Rc4,
        }
    }

    fn handler(kat: &Kat) -> SecurityHandler {
        let m = method(kat);
        SecurityHandler {
            r: kat.r,
            key_len: if kat.v == 1 { 5 } else { kat.length / 8 },
            o: hex(kat.o),
            u: hex(kat.u),
            oe: hex(kat.oe),
            ue: hex(kat.ue),
            perms: hex(kat.perms),
            p: kat.p as u32,
            encrypt_metadata: kat.encrypt_metadata,
            stm_f: m,
            str_f: m,
            eff: m,
            filters: Vec::new(),
        }
    }

    fn auth(
        h: &SecurityHandler,
        kat: &Kat,
        pw: Option<&str>,
    ) -> (Option<(Vec<u8>, PasswordKind)>, usize) {
        let mut d = Diagnostics::new();
        let got = authenticate(h, &hex(kat.id), pw, &mut d);
        let bad = d
            .as_slice()
            .iter()
            .filter(|x| x.kind == DiagnosticKind::BadEncryption)
            .count();
        (got, bad)
    }

    fn r(num: u32) -> Ref {
        Ref { num, generation: 0 }
    }

    #[test]
    fn kat_all_revisions() {
        for kat in KATS {
            let h = handler(kat);
            let (user, bad) = auth(&h, kat, Some("u"));
            let (key, kind) = user.expect(kat.label);
            assert_eq!(kind, PasswordKind::User, "{}", kat.label);
            assert_eq!(bad, 0, "{}", kat.label);
            let (owner, bad) = auth(&h, kat, Some("o"));
            assert_eq!(
                owner,
                Some((key.clone(), PasswordKind::Owner)),
                "{}",
                kat.label
            );
            assert_eq!(bad, 0, "{}", kat.label);
            assert_eq!(auth(&h, kat, None).0, None, "{}", kat.label);
            assert_eq!(auth(&h, kat, Some("x")).0, None, "{}", kat.label);
            let m = method(kat);
            assert_eq!(
                cipher::decrypt(&key, m, r(kat.string_num), &hex(kat.string)),
                Ok(b"Hello KAT".to_vec()),
                "{}",
                kat.label
            );
            assert_eq!(
                cipher::decrypt(&key, m, r(kat.stream_num), &hex(kat.stream)),
                Ok(b"BT /F1 12 Tf (KAT) Tj ET".to_vec()),
                "{}",
                kat.label
            );
        }
    }

    fn kat(label: &str) -> &'static Kat {
        KATS.iter().find(|k| k.label == label).unwrap()
    }

    #[test]
    fn perms_mismatch_is_diagnostic() {
        let k = kat("R6");
        let mut h = handler(k);
        h.perms[0] ^= 1;
        let (got, bad) = auth(&h, k, Some("u"));
        assert_eq!(got.map(|(_, kind)| kind), Some(PasswordKind::User));
        assert_eq!(bad, 1);

        let mut h = handler(k);
        h.encrypt_metadata = !h.encrypt_metadata;
        assert_eq!(auth(&h, k, Some("u")).1, 1);
    }

    #[test]
    fn password_forms() {
        assert_eq!(encode_pdf_doc("é€"), Some(vec![0xE9, 0xA0]));
        assert_eq!(encode_pdf_doc("日"), None);
        assert_eq!(encode_pdf_doc("😀"), None);
        for label in ["R4_AES", "R6"] {
            let k = kat(label);
            let h = handler(k);
            for pw in ["日本", "😀", "\u{FFFF}"] {
                assert_eq!(auth(&h, k, Some(pw)).0, None, "{label} {pw}");
            }
        }
    }

    #[test]
    fn long_multibyte_password_truncates() {
        for (label, n) in [("R4_AES", 40), ("R6", 50)] {
            let k = kat(label);
            assert_eq!(
                auth(&handler(k), k, Some(&"日".repeat(n))).0,
                None,
                "{label}"
            );
        }
    }

    #[test]
    fn malformed_key_entries_fail_cleanly() {
        for k in KATS {
            for len in [0, 5, 200] {
                for field in 0..5 {
                    let mut h = handler(k);
                    let slot = match field {
                        0 => &mut h.u,
                        1 => &mut h.o,
                        2 => &mut h.oe,
                        3 => &mut h.ue,
                        _ => &mut h.perms,
                    };
                    *slot = vec![0x5A; len];
                    for pw in [None, Some("u"), Some("o")] {
                        let _ = auth(&h, k, pw);
                    }
                }
            }
        }
    }
}
