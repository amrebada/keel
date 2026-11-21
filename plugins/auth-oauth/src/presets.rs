//! Provider presets. Each one is plain data; the engine in `lib.rs` runs the
//! same authorization-code flow for all of them.

/// How the user's profile is read after the code exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// OpenID Connect userinfo: `sub`, `email`, `email_verified`, `name`.
    Oidc,
    /// `GET /user` plus `GET /user/emails` for the verified primary email.
    Github,
    /// Graph API `me?fields=id,name,email`.
    Facebook,
    /// `GET /2/users/me`; X shares no email.
    X,
    /// Claims of the `id_token` from the token endpoint (no userinfo).
    AppleIdToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientAuth {
    /// client_id and client_secret in the form body.
    Post,
    /// HTTP Basic with client_id:client_secret.
    Basic,
}

#[derive(Debug, Clone)]
pub struct Preset {
    pub key: &'static str,
    pub label: &'static str,
    pub auth_url: &'static str,
    pub token_url: &'static str,
    pub userinfo_url: &'static str,
    /// GitHub only.
    pub emails_url: &'static str,
    pub scopes: &'static [&'static str],
    pub pkce: bool,
    pub client_auth: ClientAuth,
    pub profile: Profile,
    /// Whether an `email_verified` claim (or the provider's policy) can be
    /// trusted for linking to an existing account.
    pub trust_email: bool,
    pub extra_params: &'static [(&'static str, &'static str)],
}

pub const PRESETS: &[Preset] = &[
    Preset {
        key: "google",
        label: "Google",
        auth_url: "https://accounts.google.com/o/oauth2/v2/auth",
        token_url: "https://oauth2.googleapis.com/token",
        userinfo_url: "https://openidconnect.googleapis.com/v1/userinfo",
        emails_url: "",
        scopes: &["openid", "email", "profile"],
        pkce: true,
        client_auth: ClientAuth::Post,
        profile: Profile::Oidc,
        trust_email: true,
        extra_params: &[("prompt", "select_account")],
    },
    Preset {
        key: "apple",
        label: "Apple",
        auth_url: "https://appleid.apple.com/auth/authorize",
        token_url: "https://appleid.apple.com/auth/token",
        userinfo_url: "",
        emails_url: "",
        scopes: &["name", "email"],
        pkce: false,
        client_auth: ClientAuth::Post,
        profile: Profile::AppleIdToken,
        trust_email: true,
        // Apple posts the result back when name or email is requested.
        extra_params: &[("response_mode", "form_post")],
    },
    Preset {
        key: "facebook",
        label: "Facebook",
        auth_url: "https://www.facebook.com/v19.0/dialog/oauth",
        token_url: "https://graph.facebook.com/v19.0/oauth/access_token",
        userinfo_url: "https://graph.facebook.com/v19.0/me",
        emails_url: "",
        scopes: &["email", "public_profile"],
        pkce: false,
        client_auth: ClientAuth::Post,
        profile: Profile::Facebook,
        trust_email: false,
        extra_params: &[],
    },
    Preset {
        key: "linkedin",
        label: "LinkedIn",
        auth_url: "https://www.linkedin.com/oauth/v2/authorization",
        token_url: "https://www.linkedin.com/oauth/v2/accessToken",
        userinfo_url: "https://api.linkedin.com/v2/userinfo",
        emails_url: "",
        scopes: &["openid", "profile", "email"],
        pkce: false,
        client_auth: ClientAuth::Post,
        profile: Profile::Oidc,
        trust_email: true,
        extra_params: &[],
    },
    Preset {
        key: "github",
        label: "GitHub",
        auth_url: "https://github.com/login/oauth/authorize",
        token_url: "https://github.com/login/oauth/access_token",
        userinfo_url: "https://api.github.com/user",
        emails_url: "https://api.github.com/user/emails",
        scopes: &["read:user", "user:email"],
        pkce: false,
        client_auth: ClientAuth::Post,
        profile: Profile::Github,
        trust_email: true,
        extra_params: &[],
    },
    Preset {
        key: "microsoft",
        label: "Microsoft",
        auth_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
        token_url: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
        userinfo_url: "https://graph.microsoft.com/oidc/userinfo",
        emails_url: "",
        scopes: &["openid", "email", "profile"],
        pkce: true,
        client_auth: ClientAuth::Post,
        profile: Profile::Oidc,
        // Entra lets tenants set unverified addresses; never link on them.
        trust_email: false,
        extra_params: &[],
    },
    Preset {
        key: "x",
        label: "X",
        auth_url: "https://twitter.com/i/oauth2/authorize",
        token_url: "https://api.twitter.com/2/oauth2/token",
        userinfo_url: "https://api.twitter.com/2/users/me",
        emails_url: "",
        scopes: &["users.read", "tweet.read"],
        pkce: true,
        client_auth: ClientAuth::Basic,
        profile: Profile::X,
        trust_email: false,
        extra_params: &[],
    },
];

pub fn preset(key: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.key == key)
}

/// `OAUTH_GOOGLE_CLIENT_ID` style names.
pub fn secret_name(key: &str, what: &str) -> String {
    format!("OAUTH_{}_{what}", key.to_ascii_uppercase())
}
