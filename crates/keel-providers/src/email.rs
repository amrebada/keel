//! Email providers for the `email` slot: `console` (in builtin.rs), `smtp`,
//! `resend` and `ses`, plus templates from `app.yaml` that any of them use.
//!
//! ```yaml
//! providers:
//!   email:
//!     use: resend
//!     api_key: "${secret:RESEND_API_KEY}"
//!     from: "Shop <hello@shop.com>"
//!     templates:
//!       auth.verify:
//!         subject: "Welcome to Shop, confirm your email"
//!         text: "Hi {{name}}, confirm here: {{link}}"
//!         html: "<p>Hi {{name}}, <a href=\"{{link}}\">confirm your email</a></p>"
//! ```

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use keel_sdk::async_trait;
use keel_sdk::capabilities::{CapResult, CapabilityError, Email, EmailMessage, HttpClient, HttpRequest};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

fn fail(m: impl Into<String>) -> CapabilityError {
    CapabilityError::Failed(m.into())
}

/// One template: any field left out keeps the plugin's default.
#[derive(Debug, Clone, Default)]
pub struct Template {
    pub subject: Option<String>,
    pub text: Option<String>,
    pub html: Option<String>,
}

/// Fills `{{name}}` from vars; unknown names become empty. HTML templates
/// get HTML-escaped values.
fn render(t: &str, vars: &Map<String, Value>, escape: bool) -> String {
    let mut out = String::with_capacity(t.len());
    let mut rest = t;
    while let Some(i) = rest.find("{{") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        match after.find("}}") {
            Some(j) => {
                let key = after[..j].trim();
                let v = match vars.get(key) {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Null) | None => String::new(),
                    Some(v) => v.to_string(),
                };
                if escape {
                    out.push_str(
                        &v.replace('&', "&amp;")
                            .replace('<', "&lt;")
                            .replace('>', "&gt;")
                            .replace('"', "&quot;"),
                    );
                } else {
                    out.push_str(&v);
                }
                rest = &after[j + 2..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Applies `templates:` from the email slot, then hands off to the provider.
pub struct TemplatedEmail {
    inner: Arc<dyn Email>,
    templates: BTreeMap<String, Template>,
}

impl TemplatedEmail {
    pub fn new(inner: Arc<dyn Email>, settings: &Map<String, Value>) -> Self {
        let templates = settings
            .get("templates")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .map(|(k, v)| {
                        let f = |n: &str| v.get(n).and_then(Value::as_str).map(String::from);
                        (
                            k.clone(),
                            Template {
                                subject: f("subject"),
                                text: f("text"),
                                html: f("html"),
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self { inner, templates }
    }

    pub fn apply(&self, mut m: EmailMessage) -> EmailMessage {
        if let Some(t) = m.template.as_ref().and_then(|id| self.templates.get(id)) {
            if let Some(s) = &t.subject {
                m.subject = render(s, &m.vars, false);
            }
            if let Some(s) = &t.text {
                m.text = render(s, &m.vars, false);
            }
            if let Some(s) = &t.html {
                m.html = Some(render(s, &m.vars, true));
            }
        }
        m
    }
}

#[async_trait]
impl Email for TemplatedEmail {
    async fn send(&self, message: EmailMessage) -> CapResult<()> {
        self.inner.send(self.apply(message)).await
    }
}

/// [Resend](https://resend.com) over its HTTP API.
pub struct ResendEmail {
    pub http: Arc<dyn HttpClient>,
    pub api_key: String,
    pub from: String,
    /// `https://api.resend.com`; tests point it at a mock.
    pub base_url: String,
}

#[async_trait]
impl Email for ResendEmail {
    async fn send(&self, m: EmailMessage) -> CapResult<()> {
        let mut body = json!({ "from": self.from, "to": m.to, "subject": m.subject, "text": m.text });
        if let Some(h) = &m.html {
            body["html"] = json!(h);
        }
        let res = self
            .http
            .send(HttpRequest {
                method: "POST".into(),
                url: format!("{}/emails", self.base_url.trim_end_matches('/')),
                headers: vec![
                    ("authorization".into(), format!("Bearer {}", self.api_key)),
                    ("content-type".into(), "application/json".into()),
                ],
                body: body.to_string().into_bytes(),
            })
            .await?;
        if !(200..300).contains(&res.status) {
            return Err(fail(format!(
                "resend: HTTP {}: {}",
                res.status,
                String::from_utf8_lossy(&res.body)
            )));
        }
        Ok(())
    }
}

/// Amazon SES v2 `SendEmail`, signed with AWS Signature Version 4.
pub struct SesEmail {
    pub http: Arc<dyn HttpClient>,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
    pub from: String,
    /// `https://email.<region>.amazonaws.com`; tests point it at a mock.
    pub endpoint: String,
}

type HmacSha256 = Hmac<Sha256>;

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    let mut m = HmacSha256::new_from_slice(key).expect("any key length");
    m.update(data.as_bytes());
    m.finalize().into_bytes().to_vec()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Days since 1970 to (year, month, day), for the SigV4 date.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// `(amz_date, date)` like `20260115T093000Z`, `20260115`.
fn amz_dates(unix: u64) -> (String, String) {
    let (y, mo, d) = civil((unix / 86_400) as i64);
    let s = unix % 86_400;
    let date = format!("{y:04}{mo:02}{d:02}");
    (
        format!("{date}T{:02}{:02}{:02}Z", s / 3600, (s / 60) % 60, s % 60),
        date,
    )
}

/// SigV4 headers for a JSON POST to `path` on `host`.
#[allow(clippy::too_many_arguments)]
pub fn sigv4_headers(
    method: &str,
    host: &str,
    path: &str,
    body: &[u8],
    region: &str,
    service: &str,
    key_id: &str,
    secret: &str,
    session_token: Option<&str>,
    unix: u64,
) -> Vec<(String, String)> {
    let (amz_date, date) = amz_dates(unix);
    let payload_hash = hex(&Sha256::digest(body));
    let mut headers: Vec<(String, String)> = vec![
        ("content-type".into(), "application/json".into()),
        ("host".into(), host.into()),
        ("x-amz-content-sha256".into(), payload_hash.clone()),
        ("x-amz-date".into(), amz_date.clone()),
    ];
    if let Some(t) = session_token {
        headers.push(("x-amz-security-token".into(), t.into()));
    }
    headers.sort();
    let (signed, sig, scope) = signature(
        method,
        path,
        "",
        &headers,
        &payload_hash,
        region,
        service,
        secret,
        &amz_date,
        &date,
    );
    headers.retain(|(k, _)| k != "host");
    headers.push((
        "authorization".into(),
        format!("AWS4-HMAC-SHA256 Credential={key_id}/{scope}, SignedHeaders={signed}, Signature={sig}"),
    ));
    headers
}

/// SigV4 core: headers sorted by lowercase name. Returns (signed header
/// names, signature, credential scope).
#[allow(clippy::too_many_arguments)]
fn signature(
    method: &str,
    path: &str,
    query: &str,
    headers: &[(String, String)],
    payload_hash: &str,
    region: &str,
    service: &str,
    secret: &str,
    amz_date: &str,
    date: &str,
) -> (String, String, String) {
    let signed = headers
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let canonical_headers: String = headers
        .iter()
        .map(|(k, v)| format!("{k}:{}\n", v.trim()))
        .collect();
    let canonical = format!("{method}\n{path}\n{query}\n{canonical_headers}\n{signed}\n{payload_hash}");
    let scope = format!("{date}/{region}/{service}/aws4_request");
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        hex(&Sha256::digest(canonical.as_bytes()))
    );
    let k = hmac(format!("AWS4{secret}").as_bytes(), date);
    let k = hmac(&k, region);
    let k = hmac(&k, service);
    let k = hmac(&k, "aws4_request");
    (signed, hex(&hmac(&k, &to_sign)), scope)
}

#[async_trait]
impl Email for SesEmail {
    async fn send(&self, m: EmailMessage) -> CapResult<()> {
        let mut content = json!({ "Text": { "Data": m.text, "Charset": "UTF-8" } });
        if let Some(h) = &m.html {
            content["Html"] = json!({ "Data": h, "Charset": "UTF-8" });
        }
        let body = json!({
            "FromEmailAddress": self.from,
            "Destination": { "ToAddresses": m.to },
            "Content": { "Simple": { "Subject": { "Data": m.subject, "Charset": "UTF-8" }, "Body": content } }
        })
        .to_string()
        .into_bytes();
        let path = "/v2/email/outbound-emails";
        let host = self
            .endpoint
            .split("://")
            .nth(1)
            .unwrap_or(&self.endpoint)
            .trim_end_matches('/')
            .to_string();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let headers = sigv4_headers(
            "POST",
            &host,
            path,
            &body,
            &self.region,
            "ses",
            &self.access_key_id,
            &self.secret_access_key,
            self.session_token.as_deref(),
            now,
        );
        let res = self
            .http
            .send(HttpRequest {
                method: "POST".into(),
                url: format!("{}{path}", self.endpoint.trim_end_matches('/')),
                headers,
                body,
            })
            .await?;
        if !(200..300).contains(&res.status) {
            return Err(fail(format!(
                "ses: HTTP {}: {}",
                res.status,
                String::from_utf8_lossy(&res.body)
            )));
        }
        Ok(())
    }
}

/// SMTP with STARTTLS (port 587), implicit TLS (465) or plain (local relays).
pub struct SmtpEmail {
    transport: lettre::AsyncSmtpTransport<lettre::Tokio1Executor>,
    from: lettre::message::Mailbox,
}

impl SmtpEmail {
    /// `tls`: `starttls` (default), `tls` or `none`.
    pub fn new(
        host: &str,
        port: Option<u16>,
        username: Option<&str>,
        password: Option<&str>,
        tls: &str,
        from: &str,
    ) -> Result<Self, String> {
        use lettre::transport::smtp::authentication::Credentials;
        use lettre::AsyncSmtpTransport;
        let mut b = match tls {
            "starttls" => AsyncSmtpTransport::<lettre::Tokio1Executor>::starttls_relay(host)
                .map_err(|e| format!("smtp: {e}"))?,
            "tls" => {
                AsyncSmtpTransport::<lettre::Tokio1Executor>::relay(host).map_err(|e| format!("smtp: {e}"))?
            }
            "none" => AsyncSmtpTransport::<lettre::Tokio1Executor>::builder_dangerous(host),
            other => return Err(format!("smtp: tls must be starttls, tls or none, not `{other}`")),
        };
        if let Some(p) = port {
            b = b.port(p);
        }
        if let (Some(u), Some(p)) = (username, password) {
            b = b.credentials(Credentials::new(u.into(), p.into()));
        }
        let from = from.parse().map_err(|e| format!("smtp: from `{from}`: {e}"))?;
        Ok(Self {
            transport: b.build(),
            from,
        })
    }
}

#[async_trait]
impl Email for SmtpEmail {
    async fn send(&self, m: EmailMessage) -> CapResult<()> {
        use lettre::message::{MultiPart, SinglePart};
        use lettre::AsyncTransport;
        let mut b = lettre::Message::builder()
            .from(self.from.clone())
            .subject(m.subject);
        for to in &m.to {
            b = b.to(to.parse().map_err(|e| fail(format!("smtp: to `{to}`: {e}")))?);
        }
        let msg = match m.html {
            Some(h) => b.multipart(MultiPart::alternative_plain_html(m.text, h)),
            None => b.singlepart(SinglePart::plain(m.text)),
        }
        .map_err(|e| fail(format!("smtp: {e}")))?;
        self.transport
            .send(msg)
            .await
            .map_err(|e| fail(format!("smtp: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_templates() {
        let vars: Map<String, Value> = serde_json::from_value(json!({ "name": "<Mia>", "n": 3 })).unwrap();
        assert_eq!(
            render("Hi {{ name }}, {{n}} {{missing}}!", &vars, false),
            "Hi <Mia>, 3 !"
        );
        assert_eq!(render("<b>{{name}}</b> {{", &vars, true), "<b>&lt;Mia&gt;</b> {{");
    }

    #[test]
    fn sigv4_matches_aws_example() {
        // The worked example from the AWS SigV4 documentation (IAM ListUsers).
        let headers = vec![
            (
                "content-type".to_string(),
                "application/x-www-form-urlencoded; charset=utf-8".to_string(),
            ),
            ("host".to_string(), "iam.amazonaws.com".to_string()),
            ("x-amz-date".to_string(), "20150830T123600Z".to_string()),
        ];
        let (signed, sig, scope) = signature(
            "GET",
            "/",
            "Action=ListUsers&Version=2010-05-08",
            &headers,
            &hex(&Sha256::digest(b"")),
            "us-east-1",
            "iam",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "20150830T123600Z",
            "20150830",
        );
        assert_eq!(signed, "content-type;host;x-amz-date");
        assert_eq!(scope, "20150830/us-east-1/iam/aws4_request");
        assert_eq!(
            sig,
            "5d672d79c15b13162d9279b0855cfba6789a8edb4c82c400e06b5924a6f2b5d7"
        );
        assert_eq!(
            amz_dates(1_440_938_160),
            ("20150830T123600Z".into(), "20150830".into())
        );
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(19_723), (2024, 1, 1));
        let h = sigv4_headers(
            "POST",
            "email.us-east-1.amazonaws.com",
            "/v2/email/outbound-emails",
            b"{}",
            "us-east-1",
            "ses",
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            None,
            1_440_938_160,
        );
        let auth = &h.iter().find(|(k, _)| k == "authorization").unwrap().1;
        assert!(auth.starts_with(
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/ses/aws4_request, SignedHeaders=content-type;host;x-amz-content-sha256;x-amz-date, Signature="
        ));
        assert_eq!(auth.rsplit('=').next().unwrap().len(), 64);
    }
}
