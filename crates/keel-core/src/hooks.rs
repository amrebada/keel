//! Event hook bus. Plugins publish events (`user.created`, ...) and other
//! plugins subscribe to them without depending on each other directly.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};

use serde_json::Value;

/// An event on the bus.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub name: String,
    pub payload: Value,
}

impl Event {
    pub fn new(name: impl Into<String>, payload: Value) -> Self {
        Self {
            name: name.into(),
            payload,
        }
    }
}

type BoxFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
type Handler = Arc<dyn Fn(Event) -> BoxFuture + Send + Sync>;

/// Delivers events to subscribers, in the order they subscribed.
#[derive(Default)]
pub struct HookBus {
    handlers: RwLock<HashMap<String, Vec<Handler>>>,
}

impl HookBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribes `handler` to events named `name`. Use `*` for every event.
    pub fn subscribe<F, Fut>(&self, name: impl Into<String>, handler: F)
    where
        F: Fn(Event) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let handler: Handler = Arc::new(move |e| Box::pin(handler(e)));
        self.handlers
            .write()
            .expect("hook bus lock")
            .entry(name.into())
            .or_default()
            .push(handler);
    }

    /// Delivers `event` to its subscribers and to `*` subscribers, one after
    /// another. Returns how many handlers ran.
    pub async fn emit(&self, event: Event) -> usize {
        let targets: Vec<Handler> = {
            let map = self.handlers.read().expect("hook bus lock");
            map.get(&event.name)
                .into_iter()
                .chain(map.get("*"))
                .flatten()
                .cloned()
                .collect()
        };
        for h in &targets {
            h(event.clone()).await;
        }
        targets.len()
    }

    /// Number of handlers subscribed to `name` (not counting `*`).
    pub fn subscriber_count(&self, name: &str) -> usize {
        self.handlers
            .read()
            .expect("hook bus lock")
            .get(name)
            .map_or(0, Vec::len)
    }
}

impl std::fmt::Debug for HookBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let map = self.handlers.read().expect("hook bus lock");
        let counts: HashMap<&String, usize> = map.iter().map(|(k, v)| (k, v.len())).collect();
        f.debug_struct("HookBus").field("subscribers", &counts).finish()
    }
}
