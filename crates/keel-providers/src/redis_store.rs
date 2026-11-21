use std::time::Duration;

use keel_sdk::async_trait;
use keel_sdk::capabilities::{Cache, CapResult, CapabilityError, Kv};
use redis::aio::ConnectionManager;
use redis::AsyncCommands;

/// Redis-backed cache and key-value store. Reconnects automatically.
#[derive(Clone)]
pub struct RedisStore {
    conn: ConnectionManager,
    prefix: String,
}

fn err(e: redis::RedisError) -> CapabilityError {
    CapabilityError::Failed(format!("redis: {e}"))
}

impl RedisStore {
    pub async fn connect(url: &str, prefix: impl Into<String>) -> Result<Self, String> {
        let client = redis::Client::open(url).map_err(|e| e.to_string())?;
        let conn = tokio::time::timeout(Duration::from_secs(5), ConnectionManager::new(client))
            .await
            .map_err(|_| "timed out after 5s".to_string())?
            .map_err(|e| e.to_string())?;
        Ok(Self {
            conn,
            prefix: prefix.into(),
        })
    }

    fn key(&self, k: &str) -> String {
        format!("{}{k}", self.prefix)
    }

    async fn read(&self, key: &str) -> CapResult<Option<Vec<u8>>> {
        self.conn.clone().get(self.key(key)).await.map_err(err)
    }

    async fn remove(&self, key: &str) -> CapResult<bool> {
        let n: u64 = self.conn.clone().del(self.key(key)).await.map_err(err)?;
        Ok(n > 0)
    }
}

#[async_trait]
impl Kv for RedisStore {
    async fn ping(&self) -> CapResult<()> {
        redis::cmd("PING")
            .query_async::<String>(&mut self.conn.clone())
            .await
            .map(|_| ())
            .map_err(err)
    }
    async fn get(&self, key: &str) -> CapResult<Option<Vec<u8>>> {
        self.read(key).await
    }
    async fn set(&self, key: &str, value: Vec<u8>) -> CapResult<()> {
        self.conn.clone().set(self.key(key), value).await.map_err(err)
    }
    async fn delete(&self, key: &str) -> CapResult<bool> {
        self.remove(key).await
    }
}

#[async_trait]
impl Cache for RedisStore {
    async fn ping(&self) -> CapResult<()> {
        Kv::ping(self).await
    }
    async fn get(&self, key: &str) -> CapResult<Option<Vec<u8>>> {
        self.read(key).await
    }
    async fn set(&self, key: &str, value: Vec<u8>, ttl: Duration) -> CapResult<()> {
        let ms = ttl.as_millis().max(1) as u64;
        redis::cmd("SET")
            .arg(self.key(key))
            .arg(value)
            .arg("PX")
            .arg(ms)
            .query_async::<()>(&mut self.conn.clone())
            .await
            .map_err(err)
    }
    async fn delete(&self, key: &str) -> CapResult<bool> {
        self.remove(key).await
    }
    async fn incr(&self, key: &str, ttl: Duration) -> CapResult<i64> {
        let key = self.key(key);
        let ms = ttl.as_millis().max(1) as u64;
        // INCR then set the expiry only if the key has none (first hit).
        let (n, _): (i64, i64) = redis::pipe()
            .atomic()
            .cmd("INCR")
            .arg(&key)
            .cmd("PEXPIRE")
            .arg(&key)
            .arg(ms)
            .arg("NX")
            .query_async(&mut self.conn.clone())
            .await
            .map_err(err)?;
        Ok(n)
    }
}
