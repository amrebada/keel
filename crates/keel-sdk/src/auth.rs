//! The `identity-provider` interface (offered by `auth-core`) and the
//! `second-factor` hook (offered by `auth-mfa`). Social login, MFA and, from
//! phase 5, route guards use these instead of touching auth tables.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::async_trait;
use crate::http::ApiError;

pub const IDENTITY_PROVIDER: &str = "identity-provider";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthUser {
    pub id: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub name: Option<String>,
}

/// Returned by every login route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    /// Seconds until the access token expires.
    pub expires_in: u64,
}

/// A login either finishes or needs a second factor first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LoginOutcome {
    Tokens(Tokens),
    MfaRequired { mfa_required: bool, mfa_token: String },
}

/// Claims of a verified access token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    /// User id.
    pub sub: String,
    /// Session id; revoked sessions fail verification.
    pub sid: String,
    pub iat: u64,
    pub exp: u64,
}

#[async_trait]
pub trait IdentityProvider: Send + Sync {
    async fn user(&self, id: &str) -> Result<Option<AuthUser>, ApiError>;
    async fn user_by_email(&self, email: &str) -> Result<Option<AuthUser>, ApiError>;
    /// Creates a user without a password (social login).
    async fn create_user(
        &self,
        email: Option<&str>,
        email_verified: bool,
        name: Option<&str>,
    ) -> Result<AuthUser, ApiError>;
    async fn mark_email_verified(&self, user_id: &str) -> Result<(), ApiError>;
    /// Starts a session, asking for a second factor when one is enabled.
    async fn login(&self, user_id: &str) -> Result<LoginOutcome, ApiError>;
    /// Starts a session without the second-factor step. Only for a plugin
    /// that just checked the second factor itself.
    async fn issue_tokens(&self, user_id: &str) -> Result<Tokens, ApiError>;
    /// The user waiting on `mfa_token`, consuming it.
    async fn take_mfa_challenge(&self, mfa_token: &str) -> Result<Option<String>, ApiError>;
    /// Checks signature, expiry and that the session is still active.
    async fn verify_access_token(&self, token: &str) -> Result<Claims, ApiError>;
    /// Called by `auth-mfa` during its init.
    fn set_second_factor(&self, second_factor: Arc<dyn SecondFactor>);
}

#[async_trait]
pub trait SecondFactor: Send + Sync {
    /// Whether `user_id` must pass a second factor to log in.
    async fn required(&self, user_id: &str) -> Result<bool, ApiError>;
}

/// The bearer token from an `Authorization: Bearer ...` header.
pub fn bearer(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}
