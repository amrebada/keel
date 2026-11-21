//! RFC 6238 TOTP (HMAC-SHA1, 6 digits, 30 second steps) and RFC 4648 base32.

use hmac::{Hmac, Mac};
use sha1::Sha1;

pub const STEP: u64 = 30;
const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

pub fn base32(bytes: &[u8]) -> String {
    let mut out = String::new();
    let (mut buf, mut bits) = (0u32, 0);
    for &b in bytes {
        buf = (buf << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buf >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

pub fn unbase32(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut buf, mut bits) = (0u32, 0);
    for c in s.chars().filter(|c| *c != '=' && !c.is_whitespace()) {
        let v = ALPHABET
            .iter()
            .position(|a| *a as char == c.to_ascii_uppercase())? as u32;
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

/// The 6-digit code for time step `step`.
pub fn code(secret: &[u8], step: u64) -> String {
    let mut mac = Hmac::<Sha1>::new_from_slice(secret).expect("hmac takes any key length");
    mac.update(&step.to_be_bytes());
    let h = mac.finalize().into_bytes();
    let o = (h[19] & 0xf) as usize;
    let n = (u32::from(h[o] & 0x7f) << 24)
        | (u32::from(h[o + 1]) << 16)
        | (u32::from(h[o + 2]) << 8)
        | u32::from(h[o + 3]);
    format!("{:06}", n % 1_000_000)
}

/// The step `code` matches within one step of `now`, if any.
pub fn matching_step(secret: &[u8], code_in: &str, now: u64) -> Option<u64> {
    let code_in: String = code_in.chars().filter(|c| c.is_ascii_digit()).collect();
    if code_in.len() != 6 {
        return None;
    }
    let cur = now / STEP;
    [cur.saturating_sub(1), cur, cur + 1]
        .into_iter()
        .find(|s| constant_eq(code(secret, *s).as_bytes(), code_in.as_bytes()))
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6238_vectors() {
        // RFC 6238 appendix B, SHA1 secret, last 6 digits.
        let secret = b"12345678901234567890";
        assert_eq!(code(secret, 59 / STEP), "287082");
        assert_eq!(code(secret, 1_111_111_109 / STEP), "081804");
        assert_eq!(code(secret, 1_234_567_890 / STEP), "005924");
        assert_eq!(code(secret, 2_000_000_000 / STEP), "279037");
    }

    #[test]
    fn base32_round_trip_and_window() {
        assert_eq!(base32(b"foobar"), "MZXW6YTBOI");
        assert_eq!(unbase32("mzxw6ytboi").unwrap(), b"foobar");
        let s = b"12345678901234567890";
        let now = 1_000_000;
        let c = code(s, now / STEP - 1);
        assert_eq!(matching_step(s, &c, now), Some(now / STEP - 1));
        assert_eq!(matching_step(s, &code(s, now / STEP - 2), now), None);
        assert_eq!(matching_step(s, "12 34", now), None);
    }
}
