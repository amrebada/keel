//! `auth-oauth`: social login through any OAuth2 / OpenID Connect provider,
//! with presets for Google, Apple, Facebook, LinkedIn, GitHub, Microsoft and X.
//!
//! Flow: `GET /auth/oauth/<p>/start` redirects to the provider with a
//! one-time `state` (and PKCE where the provider supports it). The provider
//! sends the browser back to `/auth/oauth/<p>/callback`, which exchanges the
//! code, reads the profile, finds or creates the account through auth-core's
//! `identity-provider` interface, and logs the user in (MFA included).
//!
//! Linking: a known provider identity signs into its account. Otherwise, if
//! the provider vouches for the email (`email_verified`) and an account has
//! that email, the identity is linked to it. Otherwise a new account is made.

mod apple;
mod presets;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{Form, Path, Query};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine as _;
use keel_sdk::auth::{IdentityProvider, LoginOutcome, IDENTITY_PROVIDER};
use keel_sdk::capabilities::{Cache, Crypto, HttpClient, HttpRequest, Log, Secrets, Sql};
use keel_sdk::prelude::*;
use keel_sdk::Emitter;
use sha2::{Digest, Sha256};

use presets::{preset, secret_name, ClientAuth, Preset, Profile};

const STATE_TTL: Duration = Duration::from_secs(600);

/// One enabled provider: its preset with any endpoint overrides.
#[derive(Debug, Clone)]
struct Provider {
    preset: &'static Preset,
    auth_url: String,
    token_url: String,
    userinfo_url: String,
    emails_url: String,
    scopes: Vec<String>,
}

struct Settings {
    redirect_base_url: String,
    success_redirect: Option<String>,
    link_by_email: bool,
    providers: BTreeMap<String, Provider>,
}

impl Settings {
    fn from_config(c: &Value) -> Self {
        let mut providers = BTreeMap::new();
        if let Some(map) = c.get("providers").and_then(Value::as_object) {
            for (key, o) in map {
                let Some(p) = preset(key) else { continue };
                let s = |k: &str, d: &str| o.get(k).and_then(Value::as_str).unwrap_or(d).to_string();
                let scopes = o
                    .get("scopes")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                    .unwrap_or_else(|| p.scopes.iter().map(|s| s.to_string()).collect());
                providers.insert(
                    key.clone(),
                    Provider {
                        preset: p,
                        auth_url: s("auth_url", p.auth_url),
                        token_url: s("token_url", p.token_url),
                        userinfo_url: s("userinfo_url", p.userinfo_url),
                        emails_url: s("emails_url", p.emails_url),
                        scopes,
                    },
                );
            }
        }
        Self {
            redirect_base_url: c
                .get("redirect_base_url")
                .and_then(Value::as_str)
                .unwrap_or("http://localhost:8080")
                .trim_end_matches('/')
                .to_string(),
            success_redirect: c
                .get("success_redirect")
                .and_then(Value::as_str)
                .map(str::to_string),
            link_by_email: c.get("link_by_email").and_then(Value::as_bool).unwrap_or(true),
            providers,
        }
    }

    fn callback_url(&self, key: &str) -> String {
        format!("{}/auth/oauth/{key}/callback", self.redirect_base_url)
    }
}

struct Engine {
    settings: Settings,
    identity: Arc<dyn IdentityProvider>,
    sql: Arc<dyn Sql>,
    http: Arc<dyn HttpClient>,
    secrets: Arc<dyn Secrets>,
    cache: Arc<dyn Cache>,
    crypto: Arc<dyn Crypto>,
    log: Arc<dyn Log>,
    events: Emitter,
    table: String,
}

/// What a provider says about the user.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderProfile {
    subject: String,
    email: Option<String>,
    email_verified: bool,
    name: Option<String>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn form_encode(pairs: &[(&str, String)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn bad_gateway(code: &str, message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_GATEWAY, code, message)
}

/// `true`, `"true"` -> true.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s == "true",
        _ => false,
    }
}

fn str_of(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

impl Engine {
    fn provider(&self, key: &str) -> Result<&Provider, ApiError> {
        self.settings.providers.get(key).ok_or_else(|| {
            let on: Vec<&str> = self.settings.providers.keys().map(String::as_str).collect();
            ApiError::not_found(
                "unknown_provider",
                format!("`{key}` sign-in is not enabled. Enabled: {}", on.join(", ")),
            )
        })
    }

    fn secret(&self, key: &str, what: &str) -> Result<String, ApiError> {
        let name = secret_name(key, what);
        self.secrets.get(&name).filter(|v| !v.is_empty()).ok_or_else(|| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "setup_incomplete",
                format!("{name} is not set"),
            )
        })
    }

    fn client_secret(&self, key: &str) -> Result<String, ApiError> {
        if key == "apple" {
            let client_id = self.secret(key, "CLIENT_ID")?;
            return apple::client_secret(
                &self.secret(key, "TEAM_ID")?,
                &self.secret(key, "KEY_ID")?,
                &client_id,
                &self.secret(key, "PRIVATE_KEY")?,
                now(),
            )
            .map_err(|e| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "setup_incomplete", e));
        }
        self.secret(key, "CLIENT_SECRET")
    }

    fn random(&self) -> String {
        B64.encode(self.crypto.random_bytes(32))
    }

    async fn start(&self, key: &str) -> Result<String, ApiError> {
        let p = self.provider(key)?;
        let client_id = self.secret(key, "CLIENT_ID")?;
        let state = self.random();
        let verifier = self.random();
        self.cache
            .set(
                &format!("state:{state}"),
                json!({ "p": key, "v": verifier }).to_string().into_bytes(),
                STATE_TTL,
            )
            .await?;
        let mut q = vec![
            ("response_type", "code".to_string()),
            ("client_id", client_id),
            ("redirect_uri", self.settings.callback_url(key)),
            ("scope", p.scopes.join(" ")),
            ("state", state),
        ];
        if p.preset.pkce {
            let challenge = B64.encode(Sha256::digest(verifier.as_bytes()));
            q.push(("code_challenge", challenge));
            q.push(("code_challenge_method", "S256".into()));
        }
        for (k, v) in p.preset.extra_params {
            q.push((k, v.to_string()));
        }
        let sep = if p.auth_url.contains('?') { '&' } else { '?' };
        Ok(format!("{}{sep}{}", p.auth_url, form_encode(&q)))
    }

    async fn take_state(&self, key: &str, state: &str) -> Result<String, ApiError> {
        let invalid = || {
            ApiError::bad_request(
                "invalid_state",
                "this sign-in link expired or was already used; start again",
            )
        };
        let k = format!("state:{state}");
        let raw = self.cache.get(&k).await?.ok_or_else(invalid)?;
        // One use only: a second callback with the same state fails.
        if !self.cache.delete(&k).await? {
            return Err(invalid());
        }
        let v: Value = serde_json::from_slice(&raw).map_err(|_| invalid())?;
        if v["p"] != key {
            return Err(invalid());
        }
        Ok(v["v"].as_str().unwrap_or_default().to_string())
    }

    async fn http_json(&self, req: HttpRequest, what: &str) -> Result<Value, ApiError> {
        let res = self
            .http
            .send(req)
            .await
            .map_err(|e| bad_gateway("provider_unreachable", format!("{what}: {e}")))?;
        let body: Value = serde_json::from_slice(&res.body).unwrap_or(Value::Null);
        if res.status >= 400 {
            self.log
                .warn(&format!("{what} failed with {}: {body}", res.status));
            return Err(bad_gateway(
                "provider_error",
                format!("{what} failed ({}); try again", res.status),
            ));
        }
        Ok(body)
    }

    async fn exchange(&self, key: &str, code: &str, verifier: &str) -> Result<Value, ApiError> {
        let p = self.provider(key)?;
        let client_id = self.secret(key, "CLIENT_ID")?;
        let client_secret = self.client_secret(key)?;
        let mut form = vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", code.to_string()),
            ("redirect_uri", self.settings.callback_url(key)),
        ];
        let mut headers = vec![
            ("Accept".to_string(), "application/json".to_string()),
            (
                "Content-Type".to_string(),
                "application/x-www-form-urlencoded".to_string(),
            ),
        ];
        match p.preset.client_auth {
            ClientAuth::Post => {
                form.push(("client_id", client_id.clone()));
                form.push(("client_secret", client_secret));
            }
            ClientAuth::Basic => {
                let basic =
                    base64::engine::general_purpose::STANDARD.encode(format!("{client_id}:{client_secret}"));
                headers.push(("Authorization".into(), format!("Basic {basic}")));
                form.push(("client_id", client_id.clone()));
            }
        }
        if p.preset.pkce {
            form.push(("code_verifier", verifier.to_string()));
        }
        let body = self
            .http_json(
                HttpRequest {
                    method: "POST".into(),
                    url: p.token_url.clone(),
                    headers,
                    body: form_encode(&form).into_bytes(),
                },
                "token exchange",
            )
            .await?;
        if body.get("access_token").is_none() && body.get("id_token").is_none() {
            return Err(bad_gateway("provider_error", "the provider returned no token"));
        }
        Ok(body)
    }

    async fn get(&self, url: &str, access_token: &str, what: &str) -> Result<Value, ApiError> {
        self.http_json(
            HttpRequest {
                method: "GET".into(),
                url: url.to_string(),
                headers: vec![
                    ("Authorization".into(), format!("Bearer {access_token}")),
                    ("Accept".into(), "application/json".into()),
                ],
                body: Vec::new(),
            },
            what,
        )
        .await
    }

    async fn profile(
        &self,
        key: &str,
        tokens: &Value,
        apple_user: Option<&str>,
    ) -> Result<ProviderProfile, ApiError> {
        let p = self.provider(key)?;
        let at = tokens
            .get("access_token")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mut prof = match p.preset.profile {
            Profile::Oidc => {
                let u = self.get(&p.userinfo_url, at, "userinfo").await?;
                ProviderProfile {
                    subject: str_of(u.get("sub")).unwrap_or_default(),
                    email: str_of(u.get("email")),
                    email_verified: truthy(u.get("email_verified")),
                    name: str_of(u.get("name")),
                }
            }
            Profile::Github => {
                let u = self.get(&p.userinfo_url, at, "user profile").await?;
                let emails = self.get(&p.emails_url, at, "user emails").await?;
                let primary = emails.as_array().and_then(|a| {
                    a.iter()
                        .find(|e| truthy(e.get("primary")) && truthy(e.get("verified")))
                        .and_then(|e| str_of(e.get("email")))
                });
                ProviderProfile {
                    subject: str_of(u.get("id")).unwrap_or_default(),
                    email_verified: primary.is_some(),
                    email: primary.or_else(|| str_of(u.get("email"))),
                    name: str_of(u.get("name")).or_else(|| str_of(u.get("login"))),
                }
            }
            Profile::Facebook => {
                let sep = if p.userinfo_url.contains('?') { '&' } else { '?' };
                let url = format!("{}{sep}fields=id,name,email", p.userinfo_url);
                let u = self.get(&url, at, "user profile").await?;
                ProviderProfile {
                    subject: str_of(u.get("id")).unwrap_or_default(),
                    email: str_of(u.get("email")),
                    email_verified: false,
                    name: str_of(u.get("name")),
                }
            }
            Profile::X => {
                let u = self.get(&p.userinfo_url, at, "user profile").await?;
                let d = u.get("data").cloned().unwrap_or(Value::Null);
                ProviderProfile {
                    subject: str_of(d.get("id")).unwrap_or_default(),
                    email: None,
                    email_verified: false,
                    name: str_of(d.get("name")).or_else(|| str_of(d.get("username"))),
                }
            }
            Profile::AppleIdToken => {
                // Received straight from Apple's token endpoint over TLS, so
                // its claims are trusted without checking the signature
                // (OpenID Connect Core 3.1.3.7).
                let id_token = tokens.get("id_token").and_then(Value::as_str).unwrap_or_default();
                let claims: Value = id_token
                    .split('.')
                    .nth(1)
                    .and_then(|b| B64.decode(b).ok())
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .ok_or_else(|| bad_gateway("provider_error", "Apple returned no readable id_token"))?;
                // Apple sends the name only once, as JSON in the `user` form field.
                let name = apple_user
                    .and_then(|u| serde_json::from_str::<Value>(u).ok())
                    .and_then(|u| {
                        let n = &u["name"];
                        let full = format!(
                            "{} {}",
                            n["firstName"].as_str().unwrap_or(""),
                            n["lastName"].as_str().unwrap_or("")
                        );
                        Some(full.trim().to_string()).filter(|s| !s.is_empty())
                    });
                ProviderProfile {
                    subject: str_of(claims.get("sub")).unwrap_or_default(),
                    email: str_of(claims.get("email")),
                    email_verified: truthy(claims.get("email_verified")),
                    name,
                }
            }
        };
        if prof.subject.is_empty() {
            return Err(bad_gateway("provider_error", "the provider returned no user id"));
        }
        if !p.preset.trust_email {
            prof.email_verified = false;
        }
        Ok(prof)
    }

    async fn linked_user(&self, key: &str, subject: &str) -> Result<Option<String>, ApiError> {
        let rows = self
            .sql
            .query(
                &format!(
                    "SELECT user_id FROM {} WHERE provider = ? AND subject = ?",
                    self.table
                ),
                &[json!(key), json!(subject)],
            )
            .await?;
        Ok(rows
            .first()
            .and_then(|r| r.get("user_id"))
            .and_then(Value::as_str)
            .map(str::to_string))
    }

    async fn link(&self, key: &str, prof: &ProviderProfile, user_id: &str) -> Result<(), ApiError> {
        self.sql
            .execute(
                &format!(
                    "INSERT INTO {} (provider, subject, user_id, email, created_at) VALUES (?, ?, ?, ?, ?)",
                    self.table
                ),
                &[
                    json!(key),
                    json!(prof.subject),
                    json!(user_id),
                    json!(prof.email),
                    json!(now() as i64),
                ],
            )
            .await?;
        if let Err(e) = self
            .events
            .emit(
                "auth.oauth.linked",
                json!({ "provider": key, "user_id": user_id }),
            )
            .await
        {
            self.log.warn(&e.to_string());
        }
        Ok(())
    }

    /// Finds or creates the account for a provider profile.
    async fn account(&self, key: &str, prof: &ProviderProfile) -> Result<String, ApiError> {
        if let Some(uid) = self.linked_user(key, &prof.subject).await? {
            return Ok(uid);
        }
        let existing = match &prof.email {
            Some(e) => self.identity.user_by_email(e).await.ok().flatten(),
            None => None,
        };
        let user_id = match existing {
            Some(u) if prof.email_verified && self.settings.link_by_email => {
                if !u.email_verified {
                    self.identity.mark_email_verified(&u.id).await?;
                }
                u.id
            }
            Some(_) => {
                return Err(ApiError::conflict(
                    "email_in_use",
                    format!(
                        "an account already uses this email and {} did not confirm it belongs to you; sign in another way",
                        self.provider(key)?.preset.label
                    ),
                ))
            }
            None => {
                self.identity
                    .create_user(prof.email.as_deref(), prof.email_verified, prof.name.as_deref())
                    .await?
                    .id
            }
        };
        self.link(key, prof, &user_id).await?;
        Ok(user_id)
    }

    async fn callback(&self, key: &str, params: &HashMap<String, String>) -> Result<LoginOutcome, ApiError> {
        self.provider(key)?;
        if let Some(err) = params.get("error") {
            let detail = params.get("error_description").cloned().unwrap_or_default();
            return Err(ApiError::bad_request(
                "provider_denied",
                format!("sign-in was cancelled or refused: {err} {detail}")
                    .trim()
                    .to_string(),
            ));
        }
        let state = params
            .get("state")
            .ok_or_else(|| ApiError::bad_request("invalid_state", "missing state"))?;
        let verifier = self.take_state(key, state).await?;
        let code = params
            .get("code")
            .ok_or_else(|| ApiError::bad_request("missing_code", "the provider sent no code"))?;
        let tokens = self.exchange(key, code, &verifier).await?;
        let prof = self
            .profile(key, &tokens, params.get("user").map(String::as_str))
            .await?;
        let user_id = self.account(key, &prof).await?;
        self.identity.login(&user_id).await
    }

    /// JSON, or a redirect to `success_redirect` with the result in the fragment.
    fn respond(&self, result: Result<LoginOutcome, ApiError>) -> Response {
        let Some(base) = &self.settings.success_redirect else {
            return match result {
                Ok(o) => Json(json!(o)).into_response(),
                Err(e) => e.into_response(),
            };
        };
        let fragment = match result {
            Ok(LoginOutcome::Tokens(t)) => form_encode(&[
                ("access_token", t.access_token),
                ("refresh_token", t.refresh_token),
                ("expires_in", t.expires_in.to_string()),
                ("token_type", t.token_type),
            ]),
            Ok(LoginOutcome::MfaRequired { mfa_token, .. }) => {
                form_encode(&[("mfa_required", "true".into()), ("mfa_token", mfa_token)])
            }
            Err(e) => form_encode(&[("error", e.code), ("error_description", e.message)]),
        };
        (
            StatusCode::FOUND,
            [(header::LOCATION, format!("{base}#{fragment}"))],
        )
            .into_response()
    }
}

#[derive(Default)]
pub struct AuthOauth {
    engine: OnceLock<Arc<Engine>>,
}

impl AuthOauth {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Plugin for AuthOauth {
    fn manifest(&self) -> &Manifest {
        keel_sdk::manifest!()
    }

    fn migrations(&self, _dialect: &str) -> Vec<Migration> {
        vec![Migration::new(
            "001_identities",
            "CREATE TABLE {ns}identities (
                provider VARCHAR(30) NOT NULL,
                subject VARCHAR(255) NOT NULL,
                user_id VARCHAR(36) NOT NULL,
                email VARCHAR(255),
                created_at BIGINT NOT NULL,
                PRIMARY KEY (provider, subject)
            );
            CREATE INDEX {ns}identities_user ON {ns}identities (user_id)",
        )]
    }

    async fn check_setup(&self, step_id: &str) -> Result<(), String> {
        let Some(e) = self.engine.get() else { return Ok(()) };
        if step_id == "apple-key" {
            return e.client_secret("apple").map(|_| ()).map_err(|e| e.message);
        }
        Ok(())
    }

    async fn init(&self, ctx: &mut InitContext) -> Result<(), PluginError> {
        let caps = &ctx.capabilities;
        let engine = Arc::new(Engine {
            settings: Settings::from_config(&ctx.config),
            identity: ctx.interface::<dyn IdentityProvider>(IDENTITY_PROVIDER)?,
            sql: caps.sql()?,
            http: caps.http_client()?,
            secrets: caps.secrets()?,
            cache: caps.cache()?,
            crypto: caps.crypto()?,
            log: caps.log()?,
            events: ctx.emitter(),
            table: caps.table("identities"),
        });
        let _ = self.engine.set(engine.clone());

        let r = ctx.routes();
        let e = engine.clone();
        r.get("/auth/oauth/providers", move || {
            let e = e.clone();
            async move {
                let list: Vec<Value> = e
                    .settings
                    .providers
                    .iter()
                    .map(|(k, p)| {
                        json!({
                            "id": k,
                            "label": p.preset.label,
                            "start": format!("{}/auth/oauth/{k}/start", e.settings.redirect_base_url),
                            "ready": e.secret(k, "CLIENT_ID").is_ok(),
                        })
                    })
                    .collect();
                Json(json!({ "providers": list }))
            }
        })?;
        let e = engine.clone();
        r.get("/auth/oauth/{provider}/start", move |Path(key): Path<String>| {
            let e = e.clone();
            async move {
                match e.start(&key).await {
                    Ok(url) => (StatusCode::FOUND, [(header::LOCATION, url)]).into_response(),
                    Err(err) => err.into_response(),
                }
            }
        })?;
        let e = engine.clone();
        r.get(
            "/auth/oauth/{provider}/callback",
            move |Path(key): Path<String>, Query(q): Query<HashMap<String, String>>| {
                let e = e.clone();
                async move {
                    let result = e.callback(&key, &q).await;
                    e.respond(result)
                }
            },
        )?;
        let e = engine.clone();
        r.post(
            "/auth/oauth/{provider}/callback",
            move |Path(key): Path<String>, Form(f): Form<HashMap<String, String>>| {
                let e = e.clone();
                async move {
                    let result = e.callback(&key, &f).await;
                    e.respond(result)
                }
            },
        )?;
        Ok(())
    }
}
