//! Accounts, tokens and sessions. Handlers in `lib.rs` call into [`Service`],
//! which also serves other plugins as the `identity-provider` interface.

use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use keel_sdk::auth::{AuthUser, Claims, IdentityProvider, LoginOutcome, SecondFactor, Tokens};
use keel_sdk::capabilities::{Crypto, Email, EmailMessage, Log, Secrets, Sql};
use keel_sdk::{async_trait, ApiError, Emitter};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::jwt;

pub const JWT_SECRET: &str = "AUTH_JWT_SECRET";
const MAGIC_LINK_MAX_SECS: u64 = 900;
const MFA_CHALLENGE_SECS: u64 = 300;

/// Settings from the plugin's `config:` with defaults applied.
#[derive(Debug, Clone)]
pub struct Settings {
    pub app_url: String,
    pub verify_link: String,
    pub reset_link: String,
    pub magic_link_url: String,
    pub signup: bool,
    pub require_verified_email: bool,
    pub magic_link: bool,
    pub password_min_length: usize,
    pub access_ttl: u64,
    pub refresh_ttl: u64,
    pub token_ttl: u64,
}

impl Settings {
    pub fn from_config(c: &Value) -> Self {
        let s = |k: &str, d: &str| c.get(k).and_then(Value::as_str).unwrap_or(d).to_string();
        let b = |k: &str, d: bool| c.get(k).and_then(Value::as_bool).unwrap_or(d);
        let n = |k: &str, d: u64| c.get(k).and_then(Value::as_u64).unwrap_or(d);
        let link = |k: &str, d: &str| {
            c.pointer(&format!("/links/{k}"))
                .and_then(Value::as_str)
                .unwrap_or(d)
                .to_string()
        };
        Self {
            app_url: s("app_url", "http://localhost:8080").trim_end_matches('/').to_string(),
            verify_link: link("verify", "{app_url}/auth/verify?token={token}"),
            reset_link: link("reset", "{app_url}/reset-password?token={token}"),
            magic_link_url: link("magic_link", "{app_url}/auth/magic-link/verify?token={token}"),
            signup: b("signup", true),
            require_verified_email: b("require_verified_email", true),
            magic_link: b("magic_link", true),
            password_min_length: n("password_min_length", 8) as usize,
            access_ttl: n("access_ttl_secs", 900),
            refresh_ttl: n("refresh_ttl_secs", 2_592_000),
            token_ttl: n("token_ttl_secs", 86_400),
        }
    }

    fn link(&self, template: &str, token: &str) -> String {
        template.replace("{app_url}", &self.app_url).replace("{token}", token)
    }
}

pub struct Service {
    pub settings: Settings,
    pub sql: Arc<dyn Sql>,
    pub crypto: Arc<dyn Crypto>,
    pub email: Arc<dyn Email>,
    pub secrets: Arc<dyn Secrets>,
    pub log: Arc<dyn Log>,
    pub events: Emitter,
    /// `auth_core_` table prefix.
    pub ns: String,
    pub second_factor: RwLock<Option<Arc<dyn SecondFactor>>>,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn sha256_hex(s: &str) -> String {
    Sha256::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn normalize_email(raw: &str) -> Result<String, ApiError> {
    let e = raw.trim().to_lowercase();
    let valid = e.len() <= 254
        && e.split_once('@')
            .is_some_and(|(l, d)| !l.is_empty() && d.contains('.') && !d.starts_with('.') && !d.ends_with('.'))
        && !e.contains(char::is_whitespace);
    if valid {
        Ok(e)
    } else {
        Err(ApiError::bad_request("invalid_email", "that does not look like an email address"))
    }
}

fn user_from_row(r: &Map<String, Value>) -> AuthUser {
    let s = |k: &str| r.get(k).and_then(Value::as_str).map(str::to_string);
    AuthUser {
        id: s("id").unwrap_or_default(),
        email: s("email"),
        email_verified: r.get("email_verified").and_then(Value::as_i64).unwrap_or(0) == 1,
        name: s("name"),
    }
}

pub fn user_json(u: &AuthUser) -> Value {
    json!({ "id": u.id, "email": u.email, "email_verified": u.email_verified, "name": u.name })
}

impl Service {
    fn t(&self, name: &str) -> String {
        format!("{}{name}", self.ns)
    }

    fn jwt_key(&self) -> Result<Vec<u8>, ApiError> {
        match self.secrets.get(JWT_SECRET) {
            Some(k) if k.len() >= 32 => Ok(k.into_bytes()),
            _ => Err(ApiError::new(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "setup_incomplete",
                "AUTH_JWT_SECRET is missing or shorter than 32 characters",
            )
            .with("step", "auth-core/jwt-secret")),
        }
    }

    /// For the `jwt-secret` setup step.
    pub fn check_jwt_secret(&self) -> Result<(), String> {
        self.jwt_key().map(|_| ()).map_err(|e| e.message)
    }

    fn random_token(&self) -> String {
        self.crypto
            .random_bytes(32)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    async fn emit(&self, name: &str, payload: Value) {
        if let Err(e) = self.events.emit(name, payload).await {
            self.log.warn(&format!("event {name}: {e}"));
        }
    }

    pub fn check_password(&self, password: &str) -> Result<(), ApiError> {
        let n = password.chars().count();
        if n < self.settings.password_min_length {
            return Err(ApiError::bad_request(
                "weak_password",
                format!("use at least {} characters", self.settings.password_min_length),
            ));
        }
        if n > 1024 {
            return Err(ApiError::bad_request("weak_password", "use at most 1024 characters"));
        }
        Ok(())
    }

    async fn user_row(&self, field: &str, value: &str) -> Result<Option<Map<String, Value>>, ApiError> {
        let rows = self
            .sql
            .query(
                &format!(
                    "SELECT id, email, email_verified, name, password_hash FROM {} WHERE {field} = ?",
                    self.t("users")
                ),
                &[json!(value)],
            )
            .await?;
        Ok(rows.into_iter().next())
    }

    async fn insert_user(
        &self,
        email: Option<&str>,
        verified: bool,
        name: Option<&str>,
        password_hash: Option<&str>,
    ) -> Result<AuthUser, ApiError> {
        if let Some(e) = email {
            if self.user_row("email", e).await?.is_some() {
                return Err(ApiError::conflict("email_taken", "an account with this email already exists"));
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        let t = now() as i64;
        self.sql
            .execute(
                &format!(
                    "INSERT INTO {} (id, email, email_verified, password_hash, name, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
                    self.t("users")
                ),
                &[
                    json!(id),
                    json!(email),
                    json!(i64::from(verified)),
                    json!(password_hash),
                    json!(name),
                    json!(t),
                    json!(t),
                ],
            )
            .await
            .map_err(|e| {
                if e.to_string().to_lowercase().contains("unique") || e.to_string().contains("Duplicate") {
                    ApiError::conflict("email_taken", "an account with this email already exists")
                } else {
                    ApiError::internal(e)
                }
            })?;
        let user = AuthUser {
            id,
            email: email.map(str::to_string),
            email_verified: verified,
            name: name.map(str::to_string),
        };
        self.emit("auth.user.created", json!({ "user_id": user.id, "email": user.email }))
            .await;
        Ok(user)
    }

    /// A single-use token for `purpose`, stored hashed.
    async fn new_token(&self, purpose: &str, user_id: &str, ttl: u64) -> Result<String, ApiError> {
        let token = self.random_token();
        self.sql
            .execute(
                &format!(
                    "INSERT INTO {} (hash, purpose, user_id, expires_at) VALUES (?, ?, ?, ?)",
                    self.t("tokens")
                ),
                &[
                    json!(sha256_hex(&token)),
                    json!(purpose),
                    json!(user_id),
                    json!((now() + ttl) as i64),
                ],
            )
            .await?;
        Ok(token)
    }

    /// Consumes a token: returns its user if valid, unused and unexpired.
    async fn use_token(&self, purpose: &str, token: &str) -> Result<Option<String>, ApiError> {
        let hash = sha256_hex(token.trim());
        let t = now() as i64;
        // The UPDATE is the gate, so two concurrent uses cannot both pass.
        let n = self
            .sql
            .execute(
                &format!(
                    "UPDATE {} SET used_at = ? WHERE hash = ? AND purpose = ? AND used_at IS NULL AND expires_at > ?",
                    self.t("tokens")
                ),
                &[json!(t), json!(hash), json!(purpose), json!(t)],
            )
            .await?;
        if n == 0 {
            return Ok(None);
        }
        let rows = self
            .sql
            .query(
                &format!("SELECT user_id FROM {} WHERE hash = ?", self.t("tokens")),
                &[json!(hash)],
            )
            .await?;
        Ok(rows
            .first()
            .and_then(|r| r.get("user_id"))
            .and_then(Value::as_str)
            .map(str::to_string))
    }

    async fn send(&self, to: &str, subject: &str, text: String) -> Result<(), ApiError> {
        self.email
            .send(EmailMessage {
                to: vec![to.to_string()],
                subject: subject.to_string(),
                text,
                html: None,
            })
            .await
            .map_err(ApiError::internal)
    }

    pub async fn send_verification(&self, user: &AuthUser) -> Result<(), ApiError> {
        let Some(email) = &user.email else {
            return Ok(());
        };
        let token = self.new_token("verify", &user.id, self.settings.token_ttl).await?;
        let link = self.settings.link(&self.settings.verify_link, &token);
        self.send(
            email,
            "Confirm your email",
            format!("Confirm your email address by opening this link:\n\n{link}\n\nIf you did not sign up, ignore this email."),
        )
        .await
    }

    // ---- handlers' logic ----

    pub async fn signup(&self, email: &str, password: &str, name: Option<&str>) -> Result<Value, ApiError> {
        if !self.settings.signup {
            return Err(ApiError::forbidden("signup_disabled", "new accounts are not accepted"));
        }
        let email = normalize_email(email)?;
        self.check_password(password)?;
        let hash = self.crypto.hash_password(password)?;
        let user = self.insert_user(Some(&email), false, name, Some(&hash)).await?;
        self.send_verification(&user).await?;
        let mut body = json!({ "user": user_json(&user), "verification_sent": true });
        if !self.settings.require_verified_email {
            body["login"] = serde_json::to_value(IdentityProvider::login(self, &user.id).await?)
                .map_err(ApiError::internal)?;
        }
        Ok(body)
    }

    pub async fn verify_email(&self, token: &str) -> Result<Value, ApiError> {
        let user_id = self
            .use_token("verify", token)
            .await?
            .ok_or_else(|| ApiError::bad_request("invalid_token", "this link is invalid, used or expired"))?;
        IdentityProvider::mark_email_verified(self, &user_id).await?;
        Ok(json!({ "verified": true }))
    }

    pub async fn resend_verification(&self, email: &str) -> Result<(), ApiError> {
        let email = normalize_email(email)?;
        if let Some(row) = self.user_row("email", &email).await? {
            let user = user_from_row(&row);
            if !user.email_verified {
                self.send_verification(&user).await?;
            }
        }
        Ok(())
    }

    pub async fn password_login(&self, email: &str, password: &str) -> Result<LoginOutcome, ApiError> {
        let invalid = || ApiError::unauthorized("invalid_credentials", "wrong email or password");
        let email = normalize_email(email).map_err(|_| invalid())?;
        let row = self.user_row("email", &email).await?;
        let hash = row
            .as_ref()
            .and_then(|r| r.get("password_hash"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let Some(hash) = hash else {
            // Same work as a real check, so timing does not reveal accounts.
            let _ = self.crypto.hash_password(password);
            return Err(invalid());
        };
        if !self.crypto.verify_password(password, &hash)? {
            return Err(invalid());
        }
        let user = user_from_row(row.as_ref().expect("row exists"));
        if self.settings.require_verified_email && !user.email_verified {
            return Err(ApiError::forbidden(
                "email_not_verified",
                "confirm your email first; POST /auth/verify/resend sends the link again",
            ));
        }
        IdentityProvider::login(self, &user.id).await
    }

    pub async fn refresh(&self, refresh_token: &str) -> Result<Tokens, ApiError> {
        let invalid = || ApiError::unauthorized("invalid_refresh_token", "log in again");
        let old = sha256_hex(refresh_token.trim());
        let t = now() as i64;
        let rows = self
            .sql
            .query(
                &format!(
                    "SELECT id, user_id FROM {} WHERE refresh_hash = ? AND revoked_at IS NULL AND expires_at > ?",
                    self.t("sessions")
                ),
                &[json!(old), json!(t)],
            )
            .await?;
        let row = rows.first().ok_or_else(invalid)?;
        let sid = row.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        let user_id = row.get("user_id").and_then(Value::as_str).unwrap_or_default().to_string();
        // Rotate: the old refresh token stops working.
        let new_refresh = self.random_token();
        let n = self
            .sql
            .execute(
                &format!(
                    "UPDATE {} SET refresh_hash = ?, expires_at = ? WHERE id = ? AND refresh_hash = ?",
                    self.t("sessions")
                ),
                &[
                    json!(sha256_hex(&new_refresh)),
                    json!((now() + self.settings.refresh_ttl) as i64),
                    json!(sid),
                    json!(old),
                ],
            )
            .await?;
        if n == 0 {
            return Err(invalid());
        }
        self.tokens_for(&user_id, &sid, new_refresh)
    }

    fn tokens_for(&self, user_id: &str, sid: &str, refresh_token: String) -> Result<Tokens, ApiError> {
        let key = self.jwt_key()?;
        let iat = now();
        let claims = Claims {
            sub: user_id.to_string(),
            sid: sid.to_string(),
            iat,
            exp: iat + self.settings.access_ttl,
        };
        Ok(Tokens {
            access_token: jwt::sign(&claims, &key),
            refresh_token,
            token_type: "Bearer".into(),
            expires_in: self.settings.access_ttl,
        })
    }

    pub async fn logout(&self, refresh_token: &str) -> Result<(), ApiError> {
        let rows = self
            .sql
            .query(
                &format!("SELECT user_id FROM {} WHERE refresh_hash = ?", self.t("sessions")),
                &[json!(sha256_hex(refresh_token.trim()))],
            )
            .await?;
        self.sql
            .execute(
                &format!(
                    "UPDATE {} SET revoked_at = ? WHERE refresh_hash = ? AND revoked_at IS NULL",
                    self.t("sessions")
                ),
                &[json!(now() as i64), json!(sha256_hex(refresh_token.trim()))],
            )
            .await?;
        if let Some(uid) = rows.first().and_then(|r| r.get("user_id")).and_then(Value::as_str) {
            self.emit("auth.logout", json!({ "user_id": uid, "all": false })).await;
        }
        Ok(())
    }

    pub async fn revoke_all(&self, user_id: &str) -> Result<u64, ApiError> {
        let n = self
            .sql
            .execute(
                &format!(
                    "UPDATE {} SET revoked_at = ? WHERE user_id = ? AND revoked_at IS NULL",
                    self.t("sessions")
                ),
                &[json!(now() as i64), json!(user_id)],
            )
            .await?;
        Ok(n)
    }

    pub async fn logout_all(&self, access_token: &str) -> Result<Value, ApiError> {
        let c = IdentityProvider::verify_access_token(self, access_token).await?;
        let n = self.revoke_all(&c.sub).await?;
        self.emit("auth.logout", json!({ "user_id": c.sub, "all": true })).await;
        Ok(json!({ "revoked_sessions": n }))
    }

    pub async fn me(&self, access_token: &str) -> Result<Value, ApiError> {
        let c = IdentityProvider::verify_access_token(self, access_token).await?;
        let user = IdentityProvider::user(self, &c.sub)
            .await?
            .ok_or_else(|| ApiError::unauthorized("invalid_token", "this account no longer exists"))?;
        Ok(json!({ "user": user_json(&user) }))
    }

    pub async fn forgot_password(&self, email: &str) -> Result<(), ApiError> {
        let email = normalize_email(email)?;
        if let Some(row) = self.user_row("email", &email).await? {
            let user = user_from_row(&row);
            let token = self.new_token("reset", &user.id, self.settings.token_ttl.min(3600)).await?;
            let link = self.settings.link(&self.settings.reset_link, &token);
            self.send(
                &email,
                "Reset your password",
                format!("Choose a new password here:\n\n{link}\n\nThe link works once, for one hour. If you did not ask for this, ignore this email."),
            )
            .await?;
        }
        Ok(())
    }

    pub async fn reset_password(&self, token: &str, password: &str) -> Result<Value, ApiError> {
        self.check_password(password)?;
        let user_id = self
            .use_token("reset", token)
            .await?
            .ok_or_else(|| ApiError::bad_request("invalid_token", "this link is invalid, used or expired"))?;
        let hash = self.crypto.hash_password(password)?;
        self.sql
            .execute(
                &format!(
                    "UPDATE {} SET password_hash = ?, email_verified = 1, updated_at = ? WHERE id = ?",
                    self.t("users")
                ),
                &[json!(hash), json!(now() as i64), json!(user_id)],
            )
            .await?;
        // A reset ends every session, in case the old password leaked.
        let revoked = self.revoke_all(&user_id).await?;
        self.emit("auth.password.reset", json!({ "user_id": user_id })).await;
        Ok(json!({ "reset": true, "revoked_sessions": revoked }))
    }

    pub async fn request_magic_link(&self, email: &str) -> Result<(), ApiError> {
        if !self.settings.magic_link {
            return Err(ApiError::forbidden("magic_link_disabled", "magic links are turned off"));
        }
        let email = normalize_email(email)?;
        if let Some(row) = self.user_row("email", &email).await? {
            let user = user_from_row(&row);
            let ttl = self.settings.token_ttl.min(MAGIC_LINK_MAX_SECS);
            let token = self.new_token("magic", &user.id, ttl).await?;
            let link = self.settings.link(&self.settings.magic_link_url, &token);
            self.send(
                &email,
                "Your sign-in link",
                format!("Sign in by opening this link:\n\n{link}\n\nIt works once, for {} minutes.", ttl / 60),
            )
            .await?;
        }
        Ok(())
    }

    pub async fn use_magic_link(&self, token: &str) -> Result<LoginOutcome, ApiError> {
        if !self.settings.magic_link {
            return Err(ApiError::forbidden("magic_link_disabled", "magic links are turned off"));
        }
        let user_id = self
            .use_token("magic", token)
            .await?
            .ok_or_else(|| ApiError::bad_request("invalid_token", "this link is invalid, used or expired"))?;
        // Opening the link proves the user owns the address.
        IdentityProvider::mark_email_verified(self, &user_id).await?;
        IdentityProvider::login(self, &user_id).await
    }
}

#[async_trait]
impl IdentityProvider for Service {
    async fn user(&self, id: &str) -> Result<Option<AuthUser>, ApiError> {
        Ok(self.user_row("id", id).await?.as_ref().map(user_from_row))
    }

    async fn user_by_email(&self, email: &str) -> Result<Option<AuthUser>, ApiError> {
        let email = normalize_email(email)?;
        Ok(self.user_row("email", &email).await?.as_ref().map(user_from_row))
    }

    async fn create_user(
        &self,
        email: Option<&str>,
        email_verified: bool,
        name: Option<&str>,
    ) -> Result<AuthUser, ApiError> {
        let email = email.map(normalize_email).transpose()?;
        self.insert_user(email.as_deref(), email_verified, name, None).await
    }

    async fn mark_email_verified(&self, user_id: &str) -> Result<(), ApiError> {
        let n = self
            .sql
            .execute(
                &format!(
                    "UPDATE {} SET email_verified = 1, updated_at = ? WHERE id = ? AND email_verified = 0",
                    self.t("users")
                ),
                &[json!(now() as i64), json!(user_id)],
            )
            .await?;
        if n > 0 {
            self.emit("auth.user.verified", json!({ "user_id": user_id })).await;
        }
        Ok(())
    }

    async fn login(&self, user_id: &str) -> Result<LoginOutcome, ApiError> {
        let sf = self.second_factor.read().expect("second factor lock").clone();
        if let Some(sf) = sf {
            if sf.required(user_id).await? {
                let token = self.new_token("mfa", user_id, MFA_CHALLENGE_SECS).await?;
                return Ok(LoginOutcome::MfaRequired {
                    mfa_required: true,
                    mfa_token: token,
                });
            }
        }
        Ok(LoginOutcome::Tokens(self.issue_tokens(user_id).await?))
    }

    async fn issue_tokens(&self, user_id: &str) -> Result<Tokens, ApiError> {
        self.jwt_key()?;
        let sid = uuid::Uuid::new_v4().to_string();
        let refresh = self.random_token();
        let t = now();
        self.sql
            .execute(
                &format!(
                    "INSERT INTO {} (id, user_id, refresh_hash, created_at, expires_at) VALUES (?, ?, ?, ?, ?)",
                    self.t("sessions")
                ),
                &[
                    json!(sid),
                    json!(user_id),
                    json!(sha256_hex(&refresh)),
                    json!(t as i64),
                    json!((t + self.settings.refresh_ttl) as i64),
                ],
            )
            .await?;
        self.emit("auth.login", json!({ "user_id": user_id, "session_id": sid })).await;
        self.tokens_for(user_id, &sid, refresh)
    }

    async fn take_mfa_challenge(&self, mfa_token: &str) -> Result<Option<String>, ApiError> {
        self.use_token("mfa", mfa_token).await
    }

    async fn verify_access_token(&self, token: &str) -> Result<Claims, ApiError> {
        let key = self.jwt_key()?;
        let claims = jwt::verify(token, &key, now()).map_err(|e| {
            let (code, msg) = match e {
                jwt::JwtError::Expired => ("token_expired", "the access token expired; use /auth/refresh"),
                _ => ("invalid_token", "the access token is not valid"),
            };
            ApiError::unauthorized(code, msg)
        })?;
        let rows = self
            .sql
            .query(
                &format!(
                    "SELECT id FROM {} WHERE id = ? AND revoked_at IS NULL",
                    self.t("sessions")
                ),
                &[json!(claims.sid)],
            )
            .await?;
        if rows.is_empty() {
            return Err(ApiError::unauthorized("session_revoked", "this session has ended; log in again"));
        }
        Ok(claims)
    }

    fn set_second_factor(&self, second_factor: Arc<dyn SecondFactor>) {
        *self.second_factor.write().expect("second factor lock") = Some(second_factor);
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_email;

    #[test]
    fn emails() {
        assert_eq!(normalize_email("  Ann@Example.COM ").unwrap(), "ann@example.com");
        for bad in ["", "ann", "@x.com", "ann@x", "a b@x.com", "ann@.com", "ann@x."] {
            assert!(normalize_email(bad).is_err(), "{bad}");
        }
    }
}
