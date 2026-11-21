//! Example Keel plugin. Shows the whole contract in one small file:
//! manifest, validated config, a declared route, an emitted event, the log
//! capability and a live config change.

use std::sync::{Arc, RwLock};

use keel_sdk::prelude::*;

#[derive(Default)]
pub struct Ping {
    message: Arc<RwLock<String>>,
}

impl Ping {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Plugin for Ping {
    fn manifest(&self) -> &Manifest {
        keel_sdk::manifest!()
    }

    async fn init(&self, ctx: &mut InitContext) -> Result<(), PluginError> {
        let message = ctx
            .config
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("pong")
            .to_string();
        *self.message.write().unwrap() = message;

        let log = ctx.capabilities.log()?;
        let emitter = ctx.emitter();
        let msg = self.message.clone();
        ctx.routes().get("/ping", move || {
            let msg = msg.read().unwrap().clone();
            let emitter = emitter.clone();
            let log = log.clone();
            async move {
                log.info("ping received");
                let _ = emitter.emit("ping.received", json!({})).await;
                Json(json!({ "message": msg }))
            }
        })?;
        Ok(())
    }

    async fn configure(&self, patch: Value) -> Result<(), PluginError> {
        match patch.get("message").and_then(Value::as_str) {
            Some(m) if !m.is_empty() => {
                *self.message.write().unwrap() = m.to_string();
                Ok(())
            }
            _ => Err(PluginError::other("`message` must be a non-empty string")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_valid() {
        let m = Ping::new().manifest().clone();
        assert_eq!(m.id, "ping");
        assert!(m.declares("GET", "/ping"));
        assert!(m.validate_config(&json!({ "message": "hi" })).is_empty());
        assert!(!m.validate_config(&json!({ "nope": 1 })).is_empty());
    }
}
