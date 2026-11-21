//! Capabilities every host has without configuration: `crypto` (Argon2id
//! password hashing, secure random bytes) and `http-client` (reqwest), plus
//! the `console` email provider.

use std::sync::Mutex;
use std::time::Duration;

use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use keel_sdk::async_trait;
use keel_sdk::capabilities::{
    CapResult, CapabilityError, Crypto, Email, EmailMessage, HttpClient, HttpRequest, HttpResponse,
};

/// Argon2id with the crate defaults (19 MiB, 2 iterations), as OWASP suggests.
#[derive(Default)]
pub struct DefaultCrypto;

impl Crypto for DefaultCrypto {
    fn random_bytes(&self, len: usize) -> Vec<u8> {
        let mut buf = vec![0u8; len];
        getrandom::getrandom(&mut buf).expect("the OS random source works");
        buf
    }

    fn hash_password(&self, password: &str) -> CapResult<String> {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|e| CapabilityError::Failed(format!("hash: {e}")))
    }

    fn verify_password(&self, password: &str, hash: &str) -> CapResult<bool> {
        let parsed = PasswordHash::new(hash).map_err(|e| CapabilityError::Failed(format!("hash: {e}")))?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok())
    }
}

/// Outbound HTTP with a 15 second timeout. Honors HTTPS_PROXY / NO_PROXY.
pub struct ReqwestClient(reqwest::Client);

impl Default for ReqwestClient {
    fn default() -> Self {
        Self(
            reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .user_agent(concat!("keel/", env!("CARGO_PKG_VERSION")))
                .build()
                .expect("http client builds"),
        )
    }
}

#[async_trait]
impl HttpClient for ReqwestClient {
    async fn send(&self, request: HttpRequest) -> CapResult<HttpResponse> {
        let fail = |e: reqwest::Error| CapabilityError::Failed(format!("http: {e}"));
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|e| CapabilityError::Failed(format!("http: bad method: {e}")))?;
        let mut rb = self.0.request(method, &request.url);
        for (k, v) in &request.headers {
            rb = rb.header(k, v);
        }
        let res = rb.body(request.body).send().await.map_err(fail)?;
        let status = res.status().as_u16();
        let headers = res
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_string()))
            .collect();
        let body = res.bytes().await.map_err(fail)?.to_vec();
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

/// Prints every email to the log instead of sending it. Keeps the last 100
/// so tests and `keel dev` users can read links out of them.
#[derive(Default)]
pub struct ConsoleEmail {
    pub from: String,
    sent: Mutex<Vec<EmailMessage>>,
}

impl ConsoleEmail {
    pub fn new(from: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            sent: Mutex::new(Vec::new()),
        }
    }

    /// Messages sent so far, oldest first.
    pub fn sent(&self) -> Vec<EmailMessage> {
        self.sent.lock().expect("email lock").clone()
    }
}

#[async_trait]
impl Email for ConsoleEmail {
    async fn send(&self, message: EmailMessage) -> CapResult<()> {
        tracing::info!(
            from = %self.from,
            to = %message.to.join(", "),
            subject = %message.subject,
            "email (console provider, not sent):\n{}",
            message.text
        );
        let mut sent = self.sent.lock().expect("email lock");
        if sent.len() >= 100 {
            sent.remove(0);
        }
        sent.push(message);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_and_verifies() {
        let c = DefaultCrypto;
        let h = c.hash_password("correct horse").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(c.verify_password("correct horse", &h).unwrap());
        assert!(!c.verify_password("wrong", &h).unwrap());
        assert_eq!(c.random_bytes(32).len(), 32);
        assert_ne!(c.random_bytes(16), c.random_bytes(16));
    }
}
