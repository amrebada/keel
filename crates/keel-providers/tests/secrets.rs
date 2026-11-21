use axum::http::HeaderMap;
use axum::routing::get;
use axum::{Json, Router};
use keel_core::{SecretSource, SecretsConfig};
use serde_json::{json, Value};

async fn mock_vault() -> String {
    async fn read(headers: HeaderMap) -> Result<Json<Value>, axum::http::StatusCode> {
        if headers.get("x-vault-token").and_then(|v| v.to_str().ok()) != Some("root-token") {
            return Err(axum::http::StatusCode::FORBIDDEN);
        }
        Ok(Json(json!({
            "data": { "data": { "AUTH_JWT_SECRET": "from-vault", "PORT_NUMBER": 5432 }, "metadata": {} }
        })))
    }
    let app = Router::new().route("/v1/secret/data/shop/prod", get(read));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn cfg(url: &str, token_env: &str, path: &str) -> SecretsConfig {
    SecretsConfig {
        kind: "vault".into(),
        url: Some(url.into()),
        token_env: Some(token_env.into()),
        mount: None,
        path: Some(path.into()),
        namespace: None,
    }
}

#[tokio::test]
async fn reads_vault_kv2_and_env_wins() {
    let url = mock_vault().await;
    std::env::set_var("KEEL_TEST_VAULT_TOKEN_OK", "root-token");
    std::env::set_var("PORT_NUMBER", "1");
    let s = keel_providers::secret_source(Some(&cfg(&url, "KEEL_TEST_VAULT_TOKEN_OK", "shop/prod")))
        .await
        .unwrap();
    assert_eq!(s.get("AUTH_JWT_SECRET").as_deref(), Some("from-vault"));
    assert_eq!(
        s.get("PORT_NUMBER").as_deref(),
        Some("1"),
        "environment overrides Vault"
    );
    assert_eq!(s.description, "env + vault (secret/shop/prod)");
}

#[tokio::test]
async fn vault_errors_say_what_to_fix() {
    let url = mock_vault().await;
    let e = keel_providers::secret_source(Some(&cfg(&url, "KEEL_TEST_VAULT_TOKEN_UNSET", "shop/prod")))
        .await
        .err()
        .unwrap();
    assert!(e.contains("KEEL_TEST_VAULT_TOKEN_UNSET"), "{e}");
    std::env::set_var("KEEL_TEST_VAULT_TOKEN_BAD", "nope");
    let e = keel_providers::secret_source(Some(&cfg(&url, "KEEL_TEST_VAULT_TOKEN_BAD", "shop/prod")))
        .await
        .err()
        .unwrap();
    assert!(e.contains("403") && e.contains("lacks read access"), "{e}");
    std::env::set_var("KEEL_TEST_VAULT_TOKEN_OK2", "root-token");
    let e = keel_providers::secret_source(Some(&cfg(&url, "KEEL_TEST_VAULT_TOKEN_OK2", "shop/none")))
        .await
        .err()
        .unwrap();
    assert!(e.contains("404"), "{e}");
}

#[tokio::test]
async fn env_is_the_default() {
    let s = keel_providers::secret_source(None).await.unwrap();
    assert_eq!(s.description, "env");
}
