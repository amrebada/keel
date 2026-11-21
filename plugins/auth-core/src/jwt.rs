//! HS256 JSON Web Tokens, enough for Keel access tokens.

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use hmac::{Hmac, Mac};
use keel_sdk::auth::Claims;
use sha2::Sha256;

const HEADER: &str = r#"{"alg":"HS256","typ":"JWT"}"#;

pub fn sign(claims: &Claims, key: &[u8]) -> String {
    let body = serde_json::to_vec(claims).expect("claims serialize");
    let signing_input = format!("{}.{}", B64.encode(HEADER), B64.encode(body));
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("hmac takes any key length");
    mac.update(signing_input.as_bytes());
    let sig = B64.encode(mac.finalize().into_bytes());
    format!("{signing_input}.{sig}")
}

#[derive(Debug, PartialEq, Eq)]
pub enum JwtError {
    Malformed,
    BadSignature,
    Expired,
}

/// Checks the signature (constant time) and expiry.
pub fn verify(token: &str, key: &[u8], now: u64) -> Result<Claims, JwtError> {
    let mut parts = token.split('.');
    let (Some(h), Some(b), Some(s), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err(JwtError::Malformed);
    };
    let header = B64.decode(h).map_err(|_| JwtError::Malformed)?;
    let header: serde_json::Value = serde_json::from_slice(&header).map_err(|_| JwtError::Malformed)?;
    if header.get("alg").and_then(|a| a.as_str()) != Some("HS256") {
        return Err(JwtError::Malformed);
    }
    let sig = B64.decode(s).map_err(|_| JwtError::Malformed)?;
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("hmac takes any key length");
    mac.update(format!("{h}.{b}").as_bytes());
    mac.verify_slice(&sig).map_err(|_| JwtError::BadSignature)?;
    let body = B64.decode(b).map_err(|_| JwtError::Malformed)?;
    let claims: Claims = serde_json::from_slice(&body).map_err(|_| JwtError::Malformed)?;
    if claims.exp <= now {
        return Err(JwtError::Expired);
    }
    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(exp: u64) -> Claims {
        Claims {
            sub: "u1".into(),
            sid: "s1".into(),
            iat: 100,
            exp,
        }
    }

    #[test]
    fn round_trip_and_failures() {
        let key = b"k".repeat(32);
        let t = sign(&claims(200), &key);
        assert_eq!(verify(&t, &key, 150).unwrap(), claims(200));
        assert_eq!(verify(&t, &key, 200), Err(JwtError::Expired));
        assert_eq!(verify(&t, b"other-key", 150), Err(JwtError::BadSignature));
        let mut parts: Vec<&str> = t.split('.').collect();
        let forged = B64.encode(br#"{"sub":"admin","sid":"s1","iat":100,"exp":999}"#);
        parts[1] = &forged;
        assert_eq!(verify(&parts.join("."), &key, 150), Err(JwtError::BadSignature));
        assert_eq!(verify("a.b", &key, 150), Err(JwtError::Malformed));
        let none = format!("{}.{}.", B64.encode(r#"{"alg":"none"}"#), B64.encode("{}"));
        assert_eq!(verify(&none, &key, 150), Err(JwtError::Malformed));
    }
}
