//! Email providers against local mocks: Resend and SES HTTP APIs and a tiny
//! SMTP server, with templates from the slot config.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::http::HeaderMap;
use axum::routing::post;
use axum::{Json, Router};
use keel_core::ProviderConfig;
use keel_sdk::capabilities::EmailMessage;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

type Seen = Arc<Mutex<Vec<(HeaderMap, Value)>>>;

async fn mock(path: &'static str) -> (String, Seen) {
    let seen: Seen = Arc::default();
    let s = seen.clone();
    let app = Router::new().route(
        path,
        post(move |h: HeaderMap, Json(b): Json<Value>| {
            let s = s.clone();
            async move {
                s.lock().unwrap().push((h, b));
                Json(json!({ "id": "msg_1" }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), seen)
}

fn slot(v: Value) -> BTreeMap<String, ProviderConfig> {
    BTreeMap::from([("email".to_string(), serde_json::from_value(v).unwrap())])
}

fn verify_message() -> EmailMessage {
    let mut vars = serde_json::Map::new();
    vars.insert("link".into(), json!("https://shop.test/verify?t=1"));
    vars.insert("name".into(), json!("Mia"));
    EmailMessage {
        to: vec!["mia@example.com".into()],
        subject: "Confirm your email".into(),
        text: "default text".into(),
        html: None,
        template: Some("auth.verify".into()),
        vars,
    }
}

#[tokio::test]
async fn resend_with_templates() {
    let (url, seen) = mock("/emails").await;
    let p = keel_providers::connect(&slot(json!({
        "use": "resend", "api_key": "re_test", "from": "Shop <hi@shop.test>", "base_url": url,
        "templates": { "auth.verify": { "subject": "Welcome {{name}}", "html": "<a href=\"{{link}}\">Confirm</a>" } }
    })))
    .await
    .unwrap();
    p.email.unwrap().send(verify_message()).await.unwrap();
    let seen = seen.lock().unwrap();
    let (h, b) = &seen[0];
    assert_eq!(h["authorization"], "Bearer re_test");
    assert_eq!(b["subject"], "Welcome Mia");
    assert_eq!(
        b["text"], "default text",
        "fields the template leaves out keep the default"
    );
    assert_eq!(b["html"], "<a href=\"https://shop.test/verify?t=1\">Confirm</a>");
    assert_eq!(b["to"], json!(["mia@example.com"]));
    assert_eq!(b["from"], "Shop <hi@shop.test>");
}

#[tokio::test]
async fn ses_signs_requests() {
    let (url, seen) = mock("/v2/email/outbound-emails").await;
    let p = keel_providers::connect(&slot(json!({
        "use": "ses", "region": "eu-west-1", "access_key_id": "AKIDTEST", "secret_access_key": "secret",
        "from": "hi@shop.test", "endpoint": url
    })))
    .await
    .unwrap();
    p.email.unwrap().send(verify_message()).await.unwrap();
    let seen = seen.lock().unwrap();
    let (h, b) = &seen[0];
    let auth = h["authorization"].to_str().unwrap();
    assert!(
        auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIDTEST/"),
        "{auth}"
    );
    assert!(auth.contains("/eu-west-1/ses/aws4_request"), "{auth}");
    assert!(h.contains_key("x-amz-date"));
    assert_eq!(b["Destination"]["ToAddresses"], json!(["mia@example.com"]));
    assert_eq!(b["Content"]["Simple"]["Subject"]["Data"], "Confirm your email");
}

/// Accepts one message and returns its DATA section.
async fn mock_smtp() -> (u16, tokio::sync::oneshot::Receiver<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (sock, _) = listener.accept().await.unwrap();
        let (r, mut w) = sock.into_split();
        let mut lines = BufReader::new(r).lines();
        w.write_all(b"220 mock ESMTP\r\n").await.unwrap();
        let mut data = String::new();
        let mut in_data = false;
        let mut tx = Some(tx);
        while let Ok(Some(line)) = lines.next_line().await {
            if in_data {
                if line == "." {
                    in_data = false;
                    w.write_all(b"250 queued\r\n").await.unwrap();
                    if let Some(tx) = tx.take() {
                        let _ = tx.send(data.clone());
                    }
                } else {
                    data.push_str(&line);
                    data.push('\n');
                }
                continue;
            }
            let cmd = line.to_ascii_uppercase();
            let reply: &[u8] = if cmd.starts_with("EHLO") {
                b"250-mock\r\n250 8BITMIME\r\n"
            } else if cmd.starts_with("DATA") {
                in_data = true;
                b"354 go ahead\r\n"
            } else if cmd.starts_with("QUIT") {
                w.write_all(b"221 bye\r\n").await.unwrap();
                break;
            } else {
                b"250 ok\r\n"
            };
            w.write_all(reply).await.unwrap();
        }
    });
    (port, rx)
}

#[tokio::test]
async fn smtp_sends_multipart() {
    let (port, rx) = mock_smtp().await;
    let p = keel_providers::connect(&slot(json!({
        "use": "smtp", "host": "127.0.0.1", "port": port, "tls": "none", "from": "Shop <hi@shop.test>",
        "templates": { "auth.verify": { "html": "<b>Hi {{name}}</b>" } }
    })))
    .await
    .unwrap();
    p.email.unwrap().send(verify_message()).await.unwrap();
    let data = rx.await.unwrap();
    assert!(data.contains("Subject: Confirm your email"), "{data}");
    assert!(data.contains("To: mia@example.com"), "{data}");
    assert!(data.contains("multipart/alternative"), "{data}");
    assert!(data.contains("<b>Hi Mia</b>"), "{data}");
}

#[tokio::test]
async fn missing_settings_are_reported() {
    let err = match keel_providers::connect(&slot(json!({ "use": "resend", "from": "a@b.c" }))).await {
        Err(e) => e.to_string(),
        Ok(_) => panic!("should fail"),
    };
    assert!(err.contains("`api_key` is required"), "{err}");
}
