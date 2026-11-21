use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use keel_sdk::async_trait;
use keel_sdk::capabilities::{Cache, CapResult, Kv};

/// Value bytes and optional expiry, by full key.
type Entries = HashMap<String, (Vec<u8>, Option<Instant>)>;

/// In-process store for development and tests. Data is lost on restart.
#[derive(Default)]
pub struct MemoryStore {
    prefix: String,
    map: Mutex<Entries>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_prefix(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            map: Mutex::default(),
        }
    }

    fn key(&self, k: &str) -> String {
        format!("{}{k}", self.prefix)
    }

    fn read(&self, key: &str) -> Option<Vec<u8>> {
        let mut map = self.map.lock().unwrap();
        let key = self.key(key);
        match map.get(&key) {
            Some((_, Some(exp))) if *exp <= Instant::now() => {
                map.remove(&key);
                None
            }
            Some((v, _)) => Some(v.clone()),
            None => None,
        }
    }

    fn write(&self, key: &str, value: Vec<u8>, ttl: Option<Duration>) {
        let exp = ttl.map(|t| Instant::now() + t);
        self.map.lock().unwrap().insert(self.key(key), (value, exp));
    }

    fn remove(&self, key: &str) -> bool {
        self.map.lock().unwrap().remove(&self.key(key)).is_some()
    }
}

#[async_trait]
impl Kv for MemoryStore {
    async fn get(&self, key: &str) -> CapResult<Option<Vec<u8>>> {
        Ok(self.read(key))
    }
    async fn set(&self, key: &str, value: Vec<u8>) -> CapResult<()> {
        self.write(key, value, None);
        Ok(())
    }
    async fn delete(&self, key: &str) -> CapResult<bool> {
        Ok(self.remove(key))
    }
}

#[async_trait]
impl Cache for MemoryStore {
    async fn get(&self, key: &str) -> CapResult<Option<Vec<u8>>> {
        Ok(self.read(key))
    }
    async fn set(&self, key: &str, value: Vec<u8>, ttl: Duration) -> CapResult<()> {
        self.write(key, value, Some(ttl));
        Ok(())
    }
    async fn delete(&self, key: &str) -> CapResult<bool> {
        Ok(self.remove(key))
    }
}
