//! `auth-mfa`: a second factor for auth-core logins.
//!
//! 1. `POST /auth/mfa/totp/setup` (logged in) returns a secret and an
//!    `otpauth://` URL for an authenticator app.
//! 2. `POST /auth/mfa/totp/confirm {code}` turns it on and returns one-time
//!    recovery codes, shown only this once.
//! 3. From then on, every login (password, magic link, social) answers
//!    `{mfa_required: true, mfa_token}`; `POST /auth/mfa/verify
//!    {mfa_token, code | recovery_code}` finishes it with tokens.
//!
//! A code works once (replay protection). After `max_failures` wrong codes
//! in a row the account's second factor locks for 5 minutes.

mod totp;

use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use keel_sdk::auth::{bearer, IdentityProvider, SecondFactor, IDENTITY_PROVIDER};
use keel_sdk::capabilities::{Crypto, Log, Sql};
use keel_sdk::prelude::*;
use keel_sdk::Emitter;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const LOCK_SECS: u64 = 300;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn sha256_hex(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn i(v: Option<&Value>) -> i64 {
    v.and_then(Value::as_i64).unwrap_or(0)
}

struct Mfa {
    identity: Arc<dyn IdentityProvider>,
    sql: Arc<dyn Sql>,
    crypto: Arc<dyn Crypto>,
    log: Arc<dyn Log>,
    events: Emitter,
    issuer: String,
    recovery_count: usize,
    max_failures: i64,
    totp: String,
    recovery: String,
}

#[derive(Deserialize)]
struct CodeBody {
    code: String,
}

#[derive(Deserialize)]
struct VerifyBody {
    mfa_token: String,
    code: Option<String>,
    recovery_code: Option<String>,
}

fn body<T: serde::de::DeserializeOwned>(b: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(b)
        .map_err(|e| ApiError::bad_request("invalid_body", format!("the JSON body is not valid: {e}")))
}

fn wrong_code() -> ApiError {
    ApiError::unauthorized(
        "invalid_code",
        "that code is not right; check the time on your device",
    )
}

impl Mfa {
    async fn user_of(&self, headers: &HeaderMap) -> Result<String, ApiError> {
        let token = bearer(headers).ok_or_else(|| {
            ApiError::unauthorized(
                "missing_token",
                "send the access token as `Authorization: Bearer <token>`",
            )
        })?;
        Ok(self.identity.verify_access_token(token).await?.sub)
    }

    async fn row(&self, user_id: &str) -> Result<Option<serde_json::Map<String, Value>>, ApiError> {
        Ok(self
            .sql
            .query(
                &format!(
                    "SELECT secret, enabled, last_step, failures, locked_until FROM {} WHERE user_id = ?",
                    self.totp
                ),
                &[json!(user_id)],
            )
            .await?
            .into_iter()
            .next())
    }

    async fn status(&self, user_id: &str) -> Result<Value, ApiError> {
        let enabled = self.row(user_id).await?.is_some_and(|r| i(r.get("enabled")) == 1);
        let left = self
            .sql
            .query(
                &format!(
                    "SELECT COUNT(*) AS n FROM {} WHERE user_id = ? AND used_at IS NULL",
                    self.recovery
                ),
                &[json!(user_id)],
            )
            .await?;
        Ok(json!({ "enabled": enabled, "recovery_codes_left": i(left.first().and_then(|r| r.get("n"))) }))
    }

    async fn setup(&self, user_id: &str) -> Result<Value, ApiError> {
        if self.row(user_id).await?.is_some_and(|r| i(r.get("enabled")) == 1) {
            return Err(ApiError::conflict(
                "mfa_already_enabled",
                "turn it off first to set up a new device",
            ));
        }
        let secret = totp::base32(&self.crypto.random_bytes(20));
        self.sql
            .execute(
                &format!("DELETE FROM {} WHERE user_id = ?", self.totp),
                &[json!(user_id)],
            )
            .await?;
        self.sql
            .execute(
                &format!(
                    "INSERT INTO {} (user_id, secret, enabled, last_step, failures, locked_until, created_at) VALUES (?, ?, 0, 0, 0, 0, ?)",
                    self.totp
                ),
                &[json!(user_id), json!(secret), json!(now() as i64)],
            )
            .await?;
        let label = match self.identity.user(user_id).await?.and_then(|u| u.email) {
            Some(e) => format!("{}:{e}", self.issuer),
            None => self.issuer.clone(),
        };
        let url = format!(
            "otpauth://totp/{}?secret={secret}&issuer={}&algorithm=SHA1&digits=6&period=30",
            urlencoding::encode(&label),
            urlencoding::encode(&self.issuer)
        );
        Ok(
            json!({ "secret": secret, "otpauth_url": url, "next": "scan it, then POST /auth/mfa/totp/confirm with a code" }),
        )
    }

    /// Checks a TOTP code for `user_id`, with lockout and replay protection.
    async fn check_totp(&self, user_id: &str, code: &str, need_enabled: bool) -> Result<(), ApiError> {
        let row = self
            .row(user_id)
            .await?
            .ok_or_else(|| ApiError::bad_request("mfa_not_set_up", "set up an authenticator first"))?;
        if need_enabled && i(row.get("enabled")) != 1 {
            return Err(ApiError::bad_request(
                "mfa_not_enabled",
                "two-factor sign-in is off",
            ));
        }
        let t = now();
        let locked_until = i(row.get("locked_until")) as u64;
        if locked_until > t {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "mfa_locked",
                format!("too many wrong codes; try again in {} seconds", locked_until - t),
            ));
        }
        let secret = row.get("secret").and_then(Value::as_str).unwrap_or_default();
        let key =
            totp::unbase32(secret).ok_or_else(|| ApiError::internal("stored TOTP secret is not base32"))?;
        let step = totp::matching_step(&key, code, t);
        let fresh = step.filter(|s| (*s as i64) > i(row.get("last_step")));
        match fresh {
            Some(s) => {
                // Only advance if no one else used this step meanwhile.
                let n = self
                    .sql
                    .execute(
                        &format!(
                            "UPDATE {} SET last_step = ?, failures = 0 WHERE user_id = ? AND last_step < ?",
                            self.totp
                        ),
                        &[json!(s as i64), json!(user_id), json!(s as i64)],
                    )
                    .await?;
                if n == 1 {
                    Ok(())
                } else {
                    Err(wrong_code())
                }
            }
            None => {
                let failures = i(row.get("failures")) + 1;
                let lock = if failures >= self.max_failures {
                    (t + LOCK_SECS) as i64
                } else {
                    0
                };
                self.sql
                    .execute(
                        &format!(
                            "UPDATE {} SET failures = ?, locked_until = ? WHERE user_id = ?",
                            self.totp
                        ),
                        &[
                            json!(if lock > 0 { 0 } else { failures }),
                            json!(lock),
                            json!(user_id),
                        ],
                    )
                    .await?;
                if lock > 0 {
                    self.log.warn(&format!(
                        "second factor locked for user {user_id} after {failures} wrong codes"
                    ));
                }
                Err(wrong_code())
            }
        }
    }

    async fn new_recovery_codes(&self, user_id: &str) -> Result<Vec<String>, ApiError> {
        self.sql
            .execute(
                &format!("DELETE FROM {} WHERE user_id = ?", self.recovery),
                &[json!(user_id)],
            )
            .await?;
        let mut codes = Vec::new();
        for _ in 0..self.recovery_count {
            let raw = totp::base32(&self.crypto.random_bytes(10)).to_lowercase();
            let code = format!("{}-{}", &raw[..8], &raw[8..16]);
            self.sql
                .execute(
                    &format!("INSERT INTO {} (hash, user_id) VALUES (?, ?)", self.recovery),
                    &[json!(sha256_hex(&code)), json!(user_id)],
                )
                .await?;
            codes.push(code);
        }
        Ok(codes)
    }

    async fn confirm(&self, user_id: &str, code: &str) -> Result<Value, ApiError> {
        self.check_totp(user_id, code, false).await?;
        self.sql
            .execute(
                &format!("UPDATE {} SET enabled = 1 WHERE user_id = ?", self.totp),
                &[json!(user_id)],
            )
            .await?;
        let codes = self.new_recovery_codes(user_id).await?;
        let _ = self
            .events
            .emit("auth.mfa.enabled", json!({ "user_id": user_id }))
            .await;
        Ok(
            json!({ "enabled": true, "recovery_codes": codes, "note": "store these now; they are not shown again" }),
        )
    }

    async fn use_recovery(&self, user_id: &str, code: &str) -> Result<(), ApiError> {
        let n = self
            .sql
            .execute(
                &format!(
                    "UPDATE {} SET used_at = ? WHERE hash = ? AND user_id = ? AND used_at IS NULL",
                    self.recovery
                ),
                &[
                    json!(now() as i64),
                    json!(sha256_hex(code.trim().to_lowercase().as_str())),
                    json!(user_id),
                ],
            )
            .await?;
        if n == 1 {
            Ok(())
        } else {
            Err(ApiError::unauthorized(
                "invalid_code",
                "that recovery code is wrong or already used",
            ))
        }
    }

    async fn verify(&self, b: VerifyBody) -> Result<Value, ApiError> {
        let user_id = self
            .identity
            .take_mfa_challenge(&b.mfa_token)
            .await?
            .ok_or_else(|| {
                ApiError::unauthorized("invalid_mfa_token", "this sign-in expired; log in again")
            })?;
        let checked = match (&b.code, &b.recovery_code) {
            (Some(c), _) => self.check_totp(&user_id, c, true).await,
            (None, Some(r)) => self.use_recovery(&user_id, r).await,
            (None, None) => Err(ApiError::bad_request(
                "missing_code",
                "send `code` or `recovery_code`",
            )),
        };
        match checked {
            Ok(()) => Ok(json!(self.identity.issue_tokens(&user_id).await?)),
            Err(e) => {
                // The challenge was used up; hand back a fresh one to retry.
                let retry = self.identity.login(&user_id).await?;
                let token = match retry {
                    keel_sdk::auth::LoginOutcome::MfaRequired { mfa_token, .. } => Some(mfa_token),
                    keel_sdk::auth::LoginOutcome::Tokens(_) => None,
                };
                Err(match token {
                    Some(t) => e.with("mfa_token", t),
                    None => e,
                })
            }
        }
    }

    async fn disable(&self, user_id: &str, code: &str) -> Result<Value, ApiError> {
        self.check_totp(user_id, code, true).await?;
        self.sql
            .execute(
                &format!("DELETE FROM {} WHERE user_id = ?", self.totp),
                &[json!(user_id)],
            )
            .await?;
        self.sql
            .execute(
                &format!("DELETE FROM {} WHERE user_id = ?", self.recovery),
                &[json!(user_id)],
            )
            .await?;
        let _ = self
            .events
            .emit("auth.mfa.disabled", json!({ "user_id": user_id }))
            .await;
        Ok(json!({ "enabled": false }))
    }
}

#[async_trait]
impl SecondFactor for Mfa {
    async fn required(&self, user_id: &str) -> Result<bool, ApiError> {
        Ok(self.row(user_id).await?.is_some_and(|r| i(r.get("enabled")) == 1))
    }
}

#[derive(Default)]
pub struct AuthMfa {
    inner: OnceLock<Arc<Mfa>>,
}

impl AuthMfa {
    pub fn new() -> Self {
        Self::default()
    }
}

macro_rules! route {
    ($m:expr, |$s:ident, $($arg:ident : $t:ty),*| $body:expr) => {{
        let m = $m.clone();
        move |$($arg: $t),*| {
            let $s = m.clone();
            async move { let r: Result<Value, ApiError> = async { $body }.await; r.map(Json) }
        }
    }};
}

#[async_trait]
impl Plugin for AuthMfa {
    fn manifest(&self) -> &Manifest {
        keel_sdk::manifest!()
    }

    fn migrations(&self, _dialect: &str) -> Vec<Migration> {
        vec![Migration::new(
            "001_init",
            "CREATE TABLE {ns}totp (
                user_id VARCHAR(36) NOT NULL PRIMARY KEY,
                secret VARCHAR(64) NOT NULL,
                enabled BIGINT NOT NULL DEFAULT 0,
                last_step BIGINT NOT NULL DEFAULT 0,
                failures BIGINT NOT NULL DEFAULT 0,
                locked_until BIGINT NOT NULL DEFAULT 0,
                created_at BIGINT NOT NULL
            );
            CREATE TABLE {ns}recovery (
                hash VARCHAR(64) NOT NULL PRIMARY KEY,
                user_id VARCHAR(36) NOT NULL,
                used_at BIGINT
            );
            CREATE INDEX {ns}recovery_user ON {ns}recovery (user_id)",
        )]
    }

    async fn init(&self, ctx: &mut InitContext) -> Result<(), PluginError> {
        let c = &ctx.config;
        let caps = &ctx.capabilities;
        let m = Arc::new(Mfa {
            identity: ctx.interface::<dyn IdentityProvider>(IDENTITY_PROVIDER)?,
            sql: caps.sql()?,
            crypto: caps.crypto()?,
            log: caps.log()?,
            events: ctx.emitter(),
            issuer: c
                .get("issuer")
                .and_then(Value::as_str)
                .unwrap_or("Keel")
                .to_string(),
            recovery_count: c.get("recovery_codes").and_then(Value::as_u64).unwrap_or(10) as usize,
            max_failures: c.get("max_failures").and_then(Value::as_i64).unwrap_or(5),
            totp: caps.table("totp"),
            recovery: caps.table("recovery"),
        });
        let _ = self.inner.set(m.clone());
        m.identity.set_second_factor(m.clone());

        let r = ctx.routes();
        r.get(
            "/auth/mfa/status",
            route!(m, |s, h: HeaderMap| s.status(&s.user_of(&h).await?).await),
        )?;
        r.post(
            "/auth/mfa/totp/setup",
            route!(m, |s, h: HeaderMap| s.setup(&s.user_of(&h).await?).await),
        )?;
        r.post(
            "/auth/mfa/totp/confirm",
            route!(m, |s, h: HeaderMap, b: Bytes| {
                let user = s.user_of(&h).await?;
                s.confirm(&user, &body::<CodeBody>(&b)?.code).await
            }),
        )?;
        r.post(
            "/auth/mfa/verify",
            route!(m, |s, b: Bytes| s.verify(body(&b)?).await),
        )?;
        r.post(
            "/auth/mfa/disable",
            route!(m, |s, h: HeaderMap, b: Bytes| {
                let user = s.user_of(&h).await?;
                s.disable(&user, &body::<CodeBody>(&b)?.code).await
            }),
        )?;
        r.post(
            "/auth/mfa/recovery-codes",
            route!(m, |s, h: HeaderMap, b: Bytes| {
                let user = s.user_of(&h).await?;
                s.check_totp(&user, &body::<CodeBody>(&b)?.code, true).await?;
                Ok(json!({ "recovery_codes": s.new_recovery_codes(&user).await? }))
            }),
        )?;
        Ok(())
    }
}

/// Exposed for tests and tools that need the current code for a secret.
pub fn totp_code(base32_secret: &str, unix_time: u64) -> Option<String> {
    totp::unbase32(base32_secret).map(|k| totp::code(&k, unix_time / totp::STEP))
}
