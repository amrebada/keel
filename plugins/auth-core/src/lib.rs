//! `auth-core`: email + password accounts, verification, password reset,
//! magic links, JWT access tokens with rotating refresh tokens, sessions and
//! logout everywhere. Offers the `identity-provider` interface that
//! `auth-oauth` and `auth-mfa` build on.
//!
//! Every route answers JSON. Errors use the standard body
//! `{"error": {"code", "message"}}`.

mod jwt;
mod migrations;
mod service;

use std::sync::{Arc, OnceLock, RwLock};

use axum::body::Bytes;
use axum::extract::Query;
use axum::http::{HeaderMap, StatusCode};
use keel_sdk::auth::{bearer, IdentityProvider, IDENTITY_PROVIDER};
use keel_sdk::prelude::*;
use serde::de::DeserializeOwned;
use serde::Deserialize;

pub use service::{Service, Settings, JWT_SECRET};

#[derive(Default)]
pub struct AuthCore {
    service: OnceLock<Arc<Service>>,
}

impl AuthCore {
    pub fn new() -> Self {
        Self::default()
    }

    /// The running service, after `init`.
    pub fn service(&self) -> Option<Arc<Service>> {
        self.service.get().cloned()
    }
}

/// Parses a JSON body into `T`, answering the standard 400 on failure.
fn body<T: DeserializeOwned>(bytes: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(bytes)
        .map_err(|e| ApiError::bad_request("invalid_body", format!("the JSON body is not valid: {e}")))
}

fn token_of(headers: &HeaderMap) -> Result<&str, ApiError> {
    bearer(headers).ok_or_else(|| {
        ApiError::unauthorized(
            "missing_token",
            "send the access token as `Authorization: Bearer <token>`",
        )
    })
}

#[derive(Deserialize)]
struct Signup {
    email: String,
    password: String,
    name: Option<String>,
}

#[derive(Deserialize)]
struct Login {
    email: String,
    password: String,
}

#[derive(Deserialize)]
struct Email {
    email: String,
}

#[derive(Deserialize)]
struct TokenBody {
    token: String,
}

#[derive(Deserialize)]
struct Refresh {
    refresh_token: String,
}

#[derive(Deserialize)]
struct Reset {
    token: String,
    password: String,
}

type Reply = Result<(StatusCode, Json<Value>), ApiError>;

fn ok(v: Value) -> Reply {
    Ok((StatusCode::OK, Json(v)))
}

fn accepted() -> Reply {
    // Same answer whether or not the account exists.
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "accepted": true, "message": "if an account exists for this address, an email is on its way" })),
    ))
}

macro_rules! handler {
    ($svc:expr, |$s:ident, $($arg:ident : $t:ty),*| $body:expr) => {{
        let svc = $svc.clone();
        move |$($arg: $t),*| {
            let $s = svc.clone();
            async move { $body }
        }
    }};
}

#[async_trait]
impl Plugin for AuthCore {
    fn manifest(&self) -> &Manifest {
        keel_sdk::manifest!()
    }

    fn migrations(&self, _dialect: &str) -> Vec<Migration> {
        migrations::all()
    }

    async fn check_setup(&self, step_id: &str) -> Result<(), String> {
        match (step_id, self.service.get()) {
            ("jwt-secret", Some(s)) => s.check_jwt_secret(),
            _ => Ok(()),
        }
    }

    async fn init(&self, ctx: &mut InitContext) -> Result<(), PluginError> {
        let caps = &ctx.capabilities;
        let svc = Arc::new(Service {
            settings: Settings::from_config(&ctx.config),
            sql: caps.sql()?,
            crypto: caps.crypto()?,
            email: caps.email()?,
            secrets: caps.secrets()?,
            log: caps.log()?,
            events: ctx.emitter(),
            ns: caps.namespace().to_string(),
            second_factor: RwLock::new(None),
        });
        let _ = self.service.set(svc.clone());
        ctx.provide::<dyn IdentityProvider>(IDENTITY_PROVIDER, svc.clone())?;

        let r = ctx.routes();
        r.post(
            "/auth/signup",
            handler!(svc, |s, b: Bytes| {
                let b: Signup = body(&b)?;
                let v = s.signup(&b.email, &b.password, b.name.as_deref()).await?;
                Ok::<_, ApiError>((StatusCode::CREATED, Json(v)))
            }),
        )?;
        r.post(
            "/auth/login",
            handler!(svc, |s, b: Bytes| {
                let b: Login = body(&b)?;
                ok(json!(s.password_login(&b.email, &b.password).await?))
            }),
        )?;
        r.post(
            "/auth/refresh",
            handler!(svc, |s, b: Bytes| {
                let b: Refresh = body(&b)?;
                ok(json!(s.refresh(&b.refresh_token).await?))
            }),
        )?;
        r.post(
            "/auth/logout",
            handler!(svc, |s, b: Bytes| {
                let b: Refresh = body(&b)?;
                s.logout(&b.refresh_token).await?;
                ok(json!({ "logged_out": true }))
            }),
        )?;
        r.post(
            "/auth/logout-all",
            handler!(svc, |s, h: HeaderMap| ok(s.logout_all(token_of(&h)?).await?)),
        )?;
        r.get(
            "/auth/me",
            handler!(svc, |s, h: HeaderMap| ok(s.me(token_of(&h)?).await?)),
        )?;
        r.get(
            "/auth/verify",
            handler!(svc, |s, q: Query<TokenBody>| ok(s.verify_email(&q.token).await?)),
        )?;
        r.post(
            "/auth/verify",
            handler!(svc, |s, b: Bytes| {
                let b: TokenBody = body(&b)?;
                ok(s.verify_email(&b.token).await?)
            }),
        )?;
        r.post(
            "/auth/verify/resend",
            handler!(svc, |s, b: Bytes| {
                let b: Email = body(&b)?;
                s.resend_verification(&b.email).await?;
                accepted()
            }),
        )?;
        r.post(
            "/auth/password/forgot",
            handler!(svc, |s, b: Bytes| {
                let b: Email = body(&b)?;
                s.forgot_password(&b.email).await?;
                accepted()
            }),
        )?;
        r.post(
            "/auth/password/reset",
            handler!(svc, |s, b: Bytes| {
                let b: Reset = body(&b)?;
                ok(s.reset_password(&b.token, &b.password).await?)
            }),
        )?;
        r.post(
            "/auth/magic-link",
            handler!(svc, |s, b: Bytes| {
                let b: Email = body(&b)?;
                s.request_magic_link(&b.email).await?;
                accepted()
            }),
        )?;
        r.get(
            "/auth/magic-link/verify",
            handler!(svc, |s, q: Query<TokenBody>| ok(json!(s.use_magic_link(&q.token).await?))),
        )?;
        r.post(
            "/auth/magic-link/verify",
            handler!(svc, |s, b: Bytes| {
                let b: TokenBody = body(&b)?;
                ok(json!(s.use_magic_link(&b.token).await?))
            }),
        )?;
        Ok(())
    }

    async fn health(&self) -> Health {
        match self.service.get().map(|s| s.check_jwt_secret()) {
            Some(Err(reason)) => Health::Degraded { reason },
            _ => Health::Ok,
        }
    }
}
