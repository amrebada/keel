//! Apple's client secret is a short-lived ES256 JWT signed with the
//! Sign in with Apple key (.p8), instead of a fixed string.

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use p256::pkcs8::DecodePrivateKey;
use serde_json::json;

/// Builds the client secret. `pem` is the .p8 file content; `\n` escapes
/// (common when a key is stored in one env var line) are accepted.
pub fn client_secret(
    team_id: &str,
    key_id: &str,
    client_id: &str,
    pem: &str,
    now: u64,
) -> Result<String, String> {
    let pem = pem.replace("\\n", "\n");
    let key = SigningKey::from_pkcs8_pem(pem.trim())
        .map_err(|e| format!("OAUTH_APPLE_PRIVATE_KEY is not a valid .p8 key: {e}"))?;
    let header = json!({ "alg": "ES256", "kid": key_id });
    let claims = json!({
        "iss": team_id,
        "iat": now,
        "exp": now + 3600,
        "aud": "https://appleid.apple.com",
        "sub": client_id,
    });
    let input = format!(
        "{}.{}",
        B64.encode(header.to_string()),
        B64.encode(claims.to_string())
    );
    let sig: Signature = key.sign(input.as_bytes());
    Ok(format!("{input}.{}", B64.encode(sig.to_bytes())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::signature::Verifier;
    use p256::ecdsa::VerifyingKey;
    use p256::pkcs8::{EncodePrivateKey, LineEnding};

    #[test]
    fn signs_a_verifiable_es256_token() {
        let key = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let pem = key.to_pkcs8_pem(LineEnding::LF).unwrap().to_string();
        let one_line = pem.replace('\n', "\\n");
        let t = client_secret("TEAM", "KEY", "com.example.web", &one_line, 1000).unwrap();
        let parts: Vec<&str> = t.split('.').collect();
        let claims: serde_json::Value = serde_json::from_slice(&B64.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(claims["iss"], "TEAM");
        assert_eq!(claims["sub"], "com.example.web");
        assert_eq!(claims["exp"], 4600);
        let sig = Signature::from_slice(&B64.decode(parts[2]).unwrap()).unwrap();
        VerifyingKey::from(&key)
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig)
            .unwrap();
        assert!(client_secret("T", "K", "C", "not a key", 0).is_err());
    }
}
