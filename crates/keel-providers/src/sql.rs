use std::time::Duration;

use keel_sdk::async_trait;
use keel_sdk::capabilities::{CapResult, CapabilityError, Sql};
use serde_json::{Map, Number, Value};
use sqlx::any::{AnyArguments, AnyPoolOptions, AnyRow};
use sqlx::query::Query;
use sqlx::{Any, AnyPool, Column, Row};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dialect {
    Postgres,
    Mysql,
    Sqlite,
}

/// SQL provider for Postgres, MySQL and SQLite.
///
/// Plugins write `?` placeholders; they are rewritten to `$1, $2, ...` for
/// Postgres. Values bind as JSON: null, bool, integer, float or string
/// (arrays and objects are stored as JSON text). Rows come back as JSON
/// objects; columns of other types (timestamps, numeric) should be cast to
/// text in the query.
pub struct SqlProvider {
    pool: AnyPool,
    dialect: Dialect,
}

impl SqlProvider {
    pub async fn connect(url: &str, max_connections: Option<u32>) -> Result<Self, String> {
        sqlx::any::install_default_drivers();
        let (dialect, url) = if url.starts_with("postgres://") || url.starts_with("postgresql://") {
            (Dialect::Postgres, url.to_string())
        } else if url.starts_with("mysql://") {
            (Dialect::Mysql, url.to_string())
        } else if url.starts_with("sqlite:") {
            // Create the database file on first use.
            let url = if url.contains(":memory:") || url.contains('?') {
                url.to_string()
            } else {
                format!("{url}?mode=rwc")
            };
            (Dialect::Sqlite, url)
        } else {
            return Err(format!(
                "unsupported database url `{url}`; use postgres://, mysql:// or sqlite:"
            ));
        };
        // Every connection to `sqlite::memory:` is a separate empty database,
        // so keep one connection.
        let max = if url.contains(":memory:") {
            1
        } else {
            max_connections.unwrap_or(10)
        };
        let pool = AnyPoolOptions::new()
            .max_connections(max)
            .acquire_timeout(Duration::from_secs(5))
            .connect(&url)
            .await
            .map_err(|e| e.to_string())?;
        Ok(Self { pool, dialect })
    }

    pub fn dialect_name(&self) -> &'static str {
        match self.dialect {
            Dialect::Postgres => "postgres",
            Dialect::Mysql => "mysql",
            Dialect::Sqlite => "sqlite",
        }
    }

    fn prepare(&self, sql: &str) -> String {
        match self.dialect {
            Dialect::Postgres => rewrite_placeholders(sql),
            _ => sql.to_string(),
        }
    }
}

fn err(e: sqlx::Error) -> CapabilityError {
    CapabilityError::Failed(format!("sql: {e}"))
}

/// `?` -> `$n`, skipping quoted strings and identifiers.
fn rewrite_placeholders(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len() + 8);
    let mut n = 0;
    let mut quote: Option<char> = None;
    for c in sql.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => {
                quote = None;
                out.push(c);
            }
            (Some(_), c) => out.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                out.push(c);
            }
            (None, '?') => {
                n += 1;
                out.push('$');
                out.push_str(&n.to_string());
            }
            (None, c) => out.push(c),
        }
    }
    out
}

fn bind<'q>(mut q: Query<'q, Any, AnyArguments<'q>>, params: &[Value]) -> Query<'q, Any, AnyArguments<'q>> {
    for p in params {
        q = match p {
            Value::Null => q.bind(None::<String>),
            Value::Bool(b) => q.bind(*b),
            Value::Number(n) => match n.as_i64() {
                Some(i) => q.bind(i),
                None => q.bind(n.as_f64().unwrap_or_default()),
            },
            Value::String(s) => q.bind(s.clone()),
            other => q.bind(other.to_string()),
        };
    }
    q
}

fn row_to_json(row: &AnyRow) -> Map<String, Value> {
    let mut obj = Map::new();
    for (i, col) in row.columns().iter().enumerate() {
        let v = if let Ok(v) = row.try_get::<Option<i64>, _>(i) {
            v.map(Value::from).unwrap_or(Value::Null)
        } else if let Ok(v) = row.try_get::<Option<f64>, _>(i) {
            v.and_then(Number::from_f64)
                .map(Value::Number)
                .unwrap_or(Value::Null)
        } else if let Ok(v) = row.try_get::<Option<bool>, _>(i) {
            v.map(Value::Bool).unwrap_or(Value::Null)
        } else if let Ok(v) = row.try_get::<Option<String>, _>(i) {
            v.map(Value::String).unwrap_or(Value::Null)
        } else if let Ok(v) = row.try_get::<Option<Vec<u8>>, _>(i) {
            v.map(|b| Value::String(String::from_utf8_lossy(&b).into_owned()))
                .unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        obj.insert(col.name().to_string(), v);
    }
    obj
}

#[async_trait]
impl Sql for SqlProvider {
    fn dialect(&self) -> &'static str {
        self.dialect_name()
    }

    async fn execute(&self, sql: &str, params: &[Value]) -> CapResult<u64> {
        let sql = self.prepare(sql);
        let r = bind(sqlx::query(&sql), params)
            .execute(&self.pool)
            .await
            .map_err(err)?;
        Ok(r.rows_affected())
    }

    async fn query(&self, sql: &str, params: &[Value]) -> CapResult<Vec<Map<String, Value>>> {
        let sql = self.prepare(sql);
        let rows = bind(sqlx::query(&sql), params)
            .fetch_all(&self.pool)
            .await
            .map_err(err)?;
        Ok(rows.iter().map(row_to_json).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::rewrite_placeholders;

    #[test]
    fn rewrites_placeholders_outside_quotes() {
        assert_eq!(
            rewrite_placeholders("SELECT * FROM t WHERE a = ? AND b = '?' AND \"c?\" = ?"),
            "SELECT * FROM t WHERE a = $1 AND b = '?' AND \"c?\" = $2"
        );
    }
}
