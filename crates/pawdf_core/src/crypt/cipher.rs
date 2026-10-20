//! RC4 and AES decryption and per-object keys (§7.6.3).

use aes::cipher::{BlockCipherDecrypt, BlockModeDecrypt, BlockModeEncrypt, KeyInit, KeyIvInit};
use aes::{Aes128, Aes256, Block};
use md5::{Digest, Md5};
use rc4::{Rc4, StreamCipher};

use super::CryptFilter;
use crate::object::Ref;

/// RC4 with `key`. A key RC4 cannot take (empty, or over 256 bytes) leaves `data` as is.
pub(crate) fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut out = data.to_vec();
    if let Ok(mut c) = Rc4::new_from_slice(key) {
        c.apply_keystream(&mut out);
    }
    out
}

fn blocks(data: &[u8]) -> Vec<Block> {
    data.as_chunks::<16>()
        .0
        .iter()
        .map(|&c| Block::from(c))
        .collect()
}

fn flatten(blocks: &[Block]) -> Vec<u8> {
    blocks.iter().flat_map(|b| b.iter().copied()).collect()
}

/// AES-CBC with no padding, over whole blocks only. `None` for a key that is not 16 or
/// 32 bytes, or an IV that is not 16.
fn cbc_decrypt(key: &[u8], iv: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    let mut b = blocks(data);
    match key.len() {
        16 => cbc::Decryptor::<Aes128>::new_from_slices(key, iv)
            .ok()?
            .decrypt_blocks(&mut b),
        32 => cbc::Decryptor::<Aes256>::new_from_slices(key, iv)
            .ok()?
            .decrypt_blocks(&mut b),
        _ => return None,
    }
    Some(flatten(&b))
}

/// AES-128-CBC encryption with no padding, for Algorithm 2.B.
pub(super) fn aes128_cbc_encrypt(key: &[u8], iv: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    let mut b = blocks(data);
    cbc::Encryptor::<Aes128>::new_from_slices(key, iv)
        .ok()?
        .encrypt_blocks(&mut b);
    Some(flatten(&b))
}

/// AES-256-CBC decryption with a zero IV and no padding, for `/UE` and `/OE`.
pub(super) fn aes256_decrypt_zero_iv(key: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    if key.len() != 32 {
        return None;
    }
    cbc_decrypt(key, &[0; 16], data)
}

/// AES-256-ECB decryption of one block, for `/Perms` (Algorithm 13).
pub(super) fn aes256_ecb_decrypt(key: &[u8], block: &[u8; 16]) -> Option<[u8; 16]> {
    let mut b = Block::from(*block);
    Aes256::new_from_slice(key).ok()?.decrypt_block(&mut b);
    Some(b.into())
}

/// AES-CBC with the IV in the first 16 bytes (§7.6.3.2). `Err` keeps the whole blocks
/// decrypted (padding left as is) when the data is short, not block-aligned, or badly
/// padded.
pub(crate) fn aes_cbc_decrypt(key: &[u8], data: &[u8]) -> Result<Vec<u8>, Vec<u8>> {
    let Some((iv, body)) = data.split_at_checked(16) else {
        return Err(Vec::new());
    };
    let whole = body.len() / 16 * 16;
    let Some(mut plain) = cbc_decrypt(key, iv, &body[..whole]) else {
        return Err(data.to_vec());
    };
    if whole != body.len() {
        return Err(plain);
    }
    let n = plain.last().copied().unwrap_or(0);
    let pad_ok = (1..=16).contains(&n)
        && plain
            .len()
            .checked_sub(usize::from(n))
            .is_some_and(|start| plain[start..].iter().all(|&b| b == n));
    if !pad_ok {
        return Err(plain);
    }
    plain.truncate(plain.len() - usize::from(n));
    Ok(plain)
}

/// Algorithm 1 (RC4, AESV2) or 1.A (AESV3): the key for one object.
pub(crate) fn object_key(file_key: &[u8], method: CryptFilter, r: Ref) -> Vec<u8> {
    if method == CryptFilter::AesV3 {
        return file_key.to_vec();
    }
    let mut md5 = Md5::new();
    md5.update(file_key);
    md5.update(&r.num.to_le_bytes()[..3]);
    md5.update(r.generation.to_le_bytes());
    if method == CryptFilter::AesV2 {
        md5.update(b"sAlT");
    }
    let hash = md5.finalize();
    let n = if method == CryptFilter::AesV2 {
        16
    } else {
        (file_key.len() + 5).min(16)
    };
    hash[..n].to_vec()
}

/// Decrypts one string or stream of object `r`. `Identity` returns the data unchanged;
/// `Unknown` returns it as `Err`.
pub(crate) fn decrypt(
    file_key: &[u8],
    method: CryptFilter,
    r: Ref,
    data: &[u8],
) -> Result<Vec<u8>, Vec<u8>> {
    match method {
        CryptFilter::Identity => Ok(data.to_vec()),
        CryptFilter::Unknown => Err(data.to_vec()),
        CryptFilter::Rc4 => Ok(rc4(&object_key(file_key, method, r), data)),
        CryptFilter::AesV2 | CryptFilter::AesV3 => {
            aes_cbc_decrypt(&object_key(file_key, method, r), data)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypt::hex;

    #[test]
    fn rc4_known_vector() {
        assert_eq!(rc4(b"Key", b"Plaintext"), hex("BBF316E8D940AF0AD3"));
    }

    fn encrypt_pkcs7(key: &[u8], iv: &[u8], plain: &[u8]) -> Vec<u8> {
        let n = 16 - plain.len() % 16;
        let mut padded = plain.to_vec();
        padded.extend(std::iter::repeat_n(n as u8, n));
        let mut out = iv.to_vec();
        out.extend(aes128_cbc_encrypt(key, iv, &padded).unwrap());
        out
    }

    #[test]
    fn aes_padding_rules() {
        let key = [7u8; 16];
        let iv = [9u8; 16];
        let ct = encrypt_pkcs7(&key, &iv, b"hello");
        assert_eq!(aes_cbc_decrypt(&key, &ct), Ok(b"hello".to_vec()));
        assert_eq!(aes_cbc_decrypt(&key, &ct[..10]), Err(Vec::new()));
        assert_eq!(aes_cbc_decrypt(&key, &iv), Err(Vec::new()));

        let two = encrypt_pkcs7(&key, &iv, &[b'a'; 20]);
        assert_eq!(aes_cbc_decrypt(&key, &two[..33]), Err(vec![b'a'; 16]));

        for last in [0x00, 0x11] {
            let mut block = [b'z'; 16];
            block[15] = last;
            let mut ct = iv.to_vec();
            ct.extend(aes128_cbc_encrypt(&key, &iv, &block).unwrap());
            assert_eq!(aes_cbc_decrypt(&key, &ct), Err(block.to_vec()), "{last}");
        }
    }

    #[test]
    fn object_key_algorithm_1() {
        use md5::{Digest, Md5};
        let key = [1u8, 2, 3, 4, 5];
        let r = Ref {
            num: 0x0A0B0C0D,
            generation: 0x0102,
        };
        let base = [1u8, 2, 3, 4, 5, 0x0D, 0x0C, 0x0B, 0x02, 0x01];
        let rc4_key = Md5::digest(base);
        assert_eq!(object_key(&key, CryptFilter::Rc4, r), rc4_key[..10]);
        let mut salted = base.to_vec();
        salted.extend_from_slice(b"sAlT");
        let aes_key = Md5::digest(&salted);
        assert_eq!(object_key(&key, CryptFilter::AesV2, r), aes_key[..]);
        assert_eq!(object_key(&key, CryptFilter::AesV3, r), key);
    }
}
