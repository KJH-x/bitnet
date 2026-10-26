//! SRUN encoding adapted from bitgateway-client (MIT); see ../NOTICE.
use base64::{
    alphabet::Alphabet,
    engine::{general_purpose::GeneralPurposeConfig, GeneralPurpose},
    Engine,
};
use hmac::{Hmac, Mac};
use md5::Md5;
use serde::Serialize;
use sha1::{Digest, Sha1};
use zeroize::Zeroizing;

use crate::GatewayError;

const ALPHABET: &str = "LVoJPiCN2R8G90yg+hmFHuacZ1OWMnrsSTXkYpUq/3dlbfKwv6xztjI7DeBE45QA";

pub(crate) fn password_hash(token: &str) -> Result<String, GatewayError> {
    let mac =
        Hmac::<Md5>::new_from_slice(token.as_bytes()).map_err(|_| GatewayError::InvalidResponse)?;
    Ok(format!("{:x}", mac.finalize().into_bytes()))
}

#[derive(Serialize)]
struct LoginInfo<'a> {
    username: &'a str,
    password: &'a str,
    ip: &'a str,
    acid: &'a str,
    enc_ver: &'static str,
}

pub(crate) fn login_info(
    username: &str,
    password: &str,
    ip: &str,
    ac_id: &str,
    token: &str,
) -> Result<String, GatewayError> {
    let plain = Zeroizing::new(
        serde_json::to_string(&LoginInfo {
            username,
            password,
            ip,
            acid: ac_id,
            enc_ver: "srun_bx1",
        })
        .map_err(|_| GatewayError::InvalidInput)?,
    );
    let encoded = xencode(plain.as_bytes(), token.as_bytes())?;
    let alphabet = Alphabet::new(ALPHABET).map_err(|_| GatewayError::InvalidResponse)?;
    let engine = GeneralPurpose::new(&alphabet, GeneralPurposeConfig::new());
    Ok(format!("{{SRBX1}}{}", engine.encode(encoded)))
}

pub(crate) fn checksum(
    token: &str,
    username: &str,
    hmd5: &str,
    ac_id: &str,
    ip: &str,
    info: &str,
) -> String {
    let mut hash = Sha1::new();
    // Incremental hashing avoids retaining another combined authentication string.
    for field in [username, hmd5, ac_id, ip, "200", "1", info] {
        hash.update(token.as_bytes());
        hash.update(field.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

pub(crate) fn valid_challenge(token: &str) -> bool {
    (16..=256).contains(&token.len()) && token.bytes().all(|b| b.is_ascii_hexdigit())
}

fn mix(buffer: &[u8], append_size: bool) -> Vec<u32> {
    let mut result = Vec::with_capacity(buffer.len().div_ceil(4) + usize::from(append_size));
    for chunk in buffer.chunks(4) {
        let mut word = [0; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        result.push(u32::from_le_bytes(word));
    }
    if append_size {
        result.push(buffer.len() as u32);
    }
    result
}

pub(crate) fn xencode(message: &[u8], token: &[u8]) -> Result<Vec<u8>, GatewayError> {
    if message.is_empty() {
        return Ok(Vec::new());
    }
    if token.len() < 16 {
        return Err(GatewayError::InvalidResponse);
    }
    let mut message = Zeroizing::new(mix(message, true));
    let key = Zeroizing::new(mix(token, false));
    let len = message.len();
    let mut right = message[len - 1];
    let mut delta = 0_u32;
    for _ in 0..6 + 52 / len {
        delta = delta.wrapping_add(0x9e37_79b9);
        let offset = (delta >> 2) & 3;
        for position in 0..len {
            let left = message[(position + 1) % len];
            right = ((right >> 5) ^ (left << 2))
                .wrapping_add(((left >> 3) ^ (right << 4)) ^ (delta ^ left))
                .wrapping_add(key[(position & 3) ^ offset as usize] ^ right)
                .wrapping_add(message[position]);
            message[position] = right;
        }
    }
    Ok(message.iter().flat_map(|word| word.to_le_bytes()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xencode_matches_independent_python_reference_vectors() {
        // Fixed vectors from a separate struct/u32 Python implementation of the
        // SRUN algorithm, not calculated by the implementation under test.
        for (plain, expected) in [
            (&b"hello world"[..], "2d325ef3aaf1546308a57508d6b613e2"),
            (&b"a"[..], "0b315eb3f861d1ce"),
            (&[1, 2, 3, 4, 5, 6, 7][..], "5c8c7100c1589739bb2b1b73"),
        ] {
            let actual = xencode(plain, b"0123456789abcdef").unwrap();
            assert_eq!(
                actual
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
                expected
            );
        }
    }

    #[test]
    fn full_login_encoding_matches_independent_reference_vector() {
        const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let hmd5 = password_hash(TOKEN).unwrap();
        assert_eq!(hmd5, "12b7df856574be74a29fa124bd70e85d");
        let info = login_info("fixture-user", "fixture-password", "10.1.2.3", "1", TOKEN).unwrap();
        assert_eq!(info, "{SRBX1}Zi7fAy5n+bOmEgxp8HsoY9560GWwpWGG4FJELcILYpGOzCaIXdlee3FNxII1tU+DhbBk2pSvwXh5MbHzEipPX//jqqWhGOFOnT6GQuzznEy/LsHNRpKlZkfO9vJYLrcuGoFNcZKtEWB4G8c6NwoaQv==");
        assert_eq!(
            checksum(TOKEN, "fixture-user", &hmd5, "1", "10.1.2.3", &info),
            "da8efde5c17b7396742f6f21974eb757afc82595"
        );
    }

    #[test]
    fn malformed_challenge_cannot_panic_or_encode() {
        assert!(xencode(b"hello", b"").is_err());
        assert!(!valid_challenge("a"));
        assert!(!valid_challenge("not-a-challenge!"));
        assert!(valid_challenge("0123456789abcdef"));
    }
}
