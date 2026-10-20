//! Test-only encryption with the standard security handler, so tests can build encrypted
//! files of every revision. Known-answer tests in `src/crypt/key.rs` pin the algorithms
//! against pikepdf, so this helper and the reader cannot share a bug unnoticed.

use aes::cipher::{BlockCipherEncrypt, BlockModeEncrypt, KeyInit, KeyIvInit};
use aes::{Aes128, Aes256, Block};
use md5::{Digest, Md5};
use rc4::{Rc4, StreamCipher};
use sha2::{Sha256, Sha384, Sha512};

const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08,
    0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];
const IV: [u8; 16] = [7; 16];
const R6_KEY: [u8; 32] = [0x42; 32];

/// The crypt method used for strings and streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cfm {
    Rc4,
    AesV2,
    AesV3,
}

/// One encryption setup.
pub struct Enc {
    pub r: u8,
    pub cfm: Cfm,
    pub key_bits: usize,
    pub user: Vec<u8>,
    pub owner: Vec<u8>,
    pub p: i32,
    pub encrypt_metadata: bool,
    pub id: Vec<u8>,
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut out = data.to_vec();
    Rc4::new_from_slice(key).unwrap().apply_keystream(&mut out);
    out
}

fn blocks(data: &[u8]) -> Vec<Block> {
    data.chunks(16)
        .map(|c| Block::try_from(c).unwrap())
        .collect()
}

fn flatten(b: &[Block]) -> Vec<u8> {
    b.iter().flat_map(|b| b.iter().copied()).collect()
}

fn cbc_encrypt(key: &[u8], iv: &[u8], data: &[u8]) -> Vec<u8> {
    let mut b = blocks(data);
    match key.len() {
        16 => cbc::Encryptor::<Aes128>::new_from_slices(key, iv)
            .unwrap()
            .encrypt_blocks(&mut b),
        _ => cbc::Encryptor::<Aes256>::new_from_slices(key, iv)
            .unwrap()
            .encrypt_blocks(&mut b),
    }
    flatten(&b)
}

fn padded(pw: &[u8]) -> Vec<u8> {
    let n = pw.len().min(32);
    [&pw[..n], &PAD[..32 - n]].concat()
}

fn xor(key: &[u8], i: u8) -> Vec<u8> {
    key.iter().map(|b| b ^ i).collect()
}

/// Algorithm 2.B (R6) or plain SHA-256 (R5).
fn hash(r: u8, pw: &[u8], salt: &[u8], udata: &[u8]) -> Vec<u8> {
    let mut k = Sha256::digest([pw, salt, udata].concat()).to_vec();
    if r == 5 {
        return k;
    }
    let mut round = 0;
    loop {
        let e = cbc_encrypt(&k[..16], &k[16..32], &[pw, &k, udata].concat().repeat(64));
        k = match e[..16].iter().map(|&b| u32::from(b)).sum::<u32>() % 3 {
            0 => Sha256::digest(&e).to_vec(),
            1 => Sha384::digest(&e).to_vec(),
            _ => Sha512::digest(&e).to_vec(),
        };
        round += 1;
        if round >= 64 && u32::from(*e.last().unwrap()) + 32 <= round {
            return k[..32].to_vec();
        }
    }
}

impl Enc {
    /// R2 RC4/40, R3 RC4/128, R4 AESV2/128, R5/R6 AESV3/256.
    pub fn new(r: u8, user: &str, owner: &str) -> Enc {
        let (cfm, key_bits) = match r {
            2 => (Cfm::Rc4, 40),
            3 => (Cfm::Rc4, 128),
            4 => (Cfm::AesV2, 128),
            _ => (Cfm::AesV3, 256),
        };
        Enc {
            r,
            cfm,
            key_bits,
            user: user.as_bytes().to_vec(),
            owner: owner.as_bytes().to_vec(),
            p: -3904,
            encrypt_metadata: true,
            id: vec![0xAB; 16],
        }
    }

    fn v(&self) -> u8 {
        match self.r {
            2 => 1,
            3 => 2,
            4 => 4,
            _ => 5,
        }
    }

    fn n(&self) -> usize {
        self.key_bits / 8
    }

    fn pw(&self, pw: &[u8]) -> Vec<u8> {
        pw[..pw.len().min(127)].to_vec()
    }

    /// The file key: Algorithm 2 for R2–R4, a fixed key for R5/R6.
    pub fn key(&self) -> Vec<u8> {
        if self.r >= 5 {
            return R6_KEY.to_vec();
        }
        let mut input = padded(&self.user);
        input.extend(self.o());
        input.extend(self.p.to_le_bytes());
        input.extend(&self.id);
        if self.r >= 4 && !self.encrypt_metadata {
            input.extend([0xFF; 4]);
        }
        let mut h = Md5::digest(&input).to_vec();
        if self.r >= 3 {
            for _ in 0..50 {
                h = Md5::digest(&h[..self.n()]).to_vec();
            }
        }
        h[..self.n()].to_vec()
    }

    /// `/O`: Algorithm 3, or Algorithm 9 for R5/R6.
    pub fn o(&self) -> Vec<u8> {
        if self.r >= 5 {
            let (vs, ks) = ([3u8; 8], [4u8; 8]);
            let owner = self.pw(&self.owner);
            return [&hash(self.r, &owner, &vs, &self.u())[..], &vs, &ks].concat();
        }
        let mut h = Md5::digest(padded(&self.owner)).to_vec();
        if self.r >= 3 {
            for _ in 0..50 {
                h = Md5::digest(&h).to_vec();
            }
        }
        let key = &h[..self.n()];
        let mut x = rc4(key, &padded(&self.user));
        if self.r >= 3 {
            for i in 1..=19 {
                x = rc4(&xor(key, i), &x);
            }
        }
        x
    }

    /// `/U`: Algorithm 4, 5, or 8.
    pub fn u(&self) -> Vec<u8> {
        if self.r >= 5 {
            let (vs, ks) = ([1u8; 8], [2u8; 8]);
            let user = self.pw(&self.user);
            return [&hash(self.r, &user, &vs, &[])[..], &vs, &ks].concat();
        }
        let key = self.key();
        if self.r == 2 {
            return rc4(&key, &PAD);
        }
        let mut x = rc4(&key, &Md5::digest([&PAD[..], &self.id].concat()));
        for i in 1..=19 {
            x = rc4(&xor(&key, i), &x);
        }
        x.extend([0u8; 16]);
        x
    }

    fn ue(&self) -> Vec<u8> {
        let k = hash(self.r, &self.pw(&self.user), &[2u8; 8], &[]);
        cbc_encrypt(&k, &[0; 16], &R6_KEY)
    }

    fn oe(&self) -> Vec<u8> {
        let k = hash(self.r, &self.pw(&self.owner), &[4u8; 8], &self.u());
        cbc_encrypt(&k, &[0; 16], &R6_KEY)
    }

    fn perms(&self) -> Vec<u8> {
        let mut block = [0u8; 16];
        block[..4].copy_from_slice(&self.p.to_le_bytes());
        block[4..8].copy_from_slice(&[0xFF; 4]);
        block[8] = if self.encrypt_metadata { b'T' } else { b'F' };
        block[9..12].copy_from_slice(b"adb");
        let mut b = Block::from(block);
        Aes256::new_from_slice(&R6_KEY)
            .unwrap()
            .encrypt_block(&mut b);
        b.to_vec()
    }

    /// The full encryption dictionary, with hex strings.
    pub fn dict(&self) -> String {
        let mut d = format!(
            "<< /Filter /Standard /V {} /R {} /Length {} /P {} /O <{}> /U <{}>",
            self.v(),
            self.r,
            self.key_bits,
            self.p,
            hex(&self.o()),
            hex(&self.u())
        );
        if self.r >= 5 {
            d += &format!(
                " /OE <{}> /UE <{}> /Perms <{}>",
                hex(&self.oe()),
                hex(&self.ue()),
                hex(&self.perms())
            );
        }
        if self.r >= 4 {
            let (cfm, len) = match self.cfm {
                Cfm::Rc4 => ("V2", 16),
                Cfm::AesV2 => ("AESV2", 16),
                Cfm::AesV3 => ("AESV3", 32),
            };
            d += &format!(
                " /CF << /StdCF << /CFM /{cfm} /Length {len} >> >> /StmF /StdCF /StrF /StdCF"
            );
        }
        if !self.encrypt_metadata {
            d += " /EncryptMetadata false";
        }
        d + " >>"
    }

    /// `/Encrypt N 0 R /ID [...]`, with no `/ID` when `id` is empty.
    pub fn trailer(&self, encrypt_num: u32) -> String {
        let mut t = format!("/Encrypt {encrypt_num} 0 R");
        if !self.id.is_empty() {
            t += &format!(" /ID [<{0}> <{0}>]", hex(&self.id));
        }
        t
    }

    /// Algorithm 1 / 1.A.
    pub fn object_key(&self, num: u32) -> Vec<u8> {
        let key = self.key();
        if self.cfm == Cfm::AesV3 {
            return key;
        }
        let mut input = key.clone();
        input.extend(&num.to_le_bytes()[..3]);
        input.extend([0, 0]);
        if self.cfm == Cfm::AesV2 {
            input.extend(b"sAlT");
        }
        let n = if self.cfm == Cfm::AesV2 {
            16
        } else {
            (key.len() + 5).min(16)
        };
        Md5::digest(&input)[..n].to_vec()
    }

    /// Ciphertext of `plain` in object `num`, generation 0.
    pub fn encrypt(&self, num: u32, plain: &[u8]) -> Vec<u8> {
        let key = self.object_key(num);
        if self.cfm == Cfm::Rc4 {
            return rc4(&key, plain);
        }
        let n = 16 - plain.len() % 16;
        let mut padded = plain.to_vec();
        padded.extend(std::iter::repeat_n(n as u8, n));
        [&IV[..], &cbc_encrypt(&key, &IV, &padded)].concat()
    }

    /// `<hex ciphertext>`.
    pub fn string(&self, num: u32, plain: &[u8]) -> String {
        format!("<{}>", hex(&self.encrypt(num, plain)))
    }

    /// `<< dict_extra /Length n >>\nstream\n…\nendstream`, encrypted.
    pub fn stream(&self, num: u32, dict_extra: &str, plain: &[u8]) -> Vec<u8> {
        raw_stream(dict_extra, &self.encrypt(num, plain))
    }
}

/// `<< dict_extra /Length n >>\nstream\n…\nendstream` around `data` as is.
pub fn raw_stream(dict_extra: &str, data: &[u8]) -> Vec<u8> {
    let mut s = format!("<< {dict_extra} /Length {} >>\nstream\n", data.len()).into_bytes();
    s.extend_from_slice(data);
    s.extend_from_slice(b"\nendstream");
    s
}
