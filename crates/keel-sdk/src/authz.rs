//! Authorization: the `authorizer` interface (offered by `rbac`), guard
//! expressions used in `app.yaml` route rules, and the hook a plugin uses to
//! offer its own guard kind (for example `plan:pro` from `billing-stripe`).

use std::collections::BTreeMap;
use std::fmt;

use crate::async_trait;
use crate::auth::Claims;
use crate::http::ApiError;

pub const AUTHORIZER: &str = "authorizer";

/// Requests name their tenant (team, organization) in this header. Roles
/// granted for that tenant apply on top of global roles.
pub const TENANT_HEADER: &str = "x-tenant-id";

#[async_trait]
pub trait Authorizer: Send + Sync {
    /// Effective roles of a user (global plus the tenant's, with inherited roles).
    async fn roles(&self, user_id: &str, tenant: Option<&str>) -> Result<Vec<String>, ApiError>;
    async fn has_role(&self, user_id: &str, role: &str, tenant: Option<&str>) -> Result<bool, ApiError>;
    /// Through a role, an inherited role, or a direct grant. `*` grants all,
    /// and `posts.*` grants every `posts.` permission.
    async fn has_permission(
        &self,
        user_id: &str,
        permission: &str,
        tenant: Option<&str>,
    ) -> Result<bool, ApiError>;
    /// Gives a role, globally (`None`) or in one tenant. For plugins that
    /// create tenants, e.g. making the creator of a team its admin.
    async fn assign_role(&self, user_id: &str, role: &str, tenant: Option<&str>) -> Result<(), ApiError>;
}

/// What a guard sees about a request.
#[derive(Debug, Clone, Default)]
pub struct GuardContext {
    pub method: String,
    pub path: String,
    /// The signed-in user, when the request carried a valid access token.
    pub user: Option<Claims>,
    /// From the `X-Tenant-Id` header.
    pub tenant: Option<String>,
    /// `{name}` segments captured by the route rule's pattern.
    pub params: BTreeMap<String, String>,
    /// Client address, for rate limits on anonymous requests.
    pub client: Option<String>,
}

/// A guard kind offered by a plugin. The plugin provides it as the interface
/// `guard:<name>` (see [`guard_interface`]) and lists that name under
/// `provides.interfaces`; route rules then use `<name>:<arg>`.
#[async_trait]
pub trait GuardCheck: Send + Sync {
    async fn check(&self, ctx: &GuardContext, arg: &str) -> Result<(), ApiError>;
}

pub fn guard_interface(name: &str) -> String {
    format!("guard:{name}")
}

/// One parsed guard expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Guard {
    /// A valid access token.
    Authenticated,
    Role(String),
    Permission(String),
    /// The signed-in user's id equals the route's `{param}`.
    Owner(String),
    /// At most `max` requests per `period_secs`, per user (or client address).
    RateLimit {
        max: u64,
        period_secs: u64,
    },
    All(Vec<Guard>),
    Any(Vec<Guard>),
    /// Offered by a plugin, e.g. `plan:pro`.
    Custom {
        name: String,
        arg: String,
    },
}

impl Guard {
    /// Parses `authenticated`, `role:admin`, `permission:posts.write`,
    /// `owner:user_id`, `rate-limit:100/min`, `any(role:admin, owner:id)`,
    /// `all(...)`, or `<plugin guard>:<arg>`.
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("empty guard".into());
        }
        for (kw, any) in [("any(", true), ("all(", false)] {
            if let Some(rest) = s.strip_prefix(kw) {
                let inner = rest
                    .strip_suffix(')')
                    .ok_or_else(|| format!("`{s}`: missing closing `)`"))?;
                let parts = split_top(inner)?;
                if parts.is_empty() {
                    return Err(format!("`{s}`: needs at least one guard"));
                }
                let list = parts
                    .iter()
                    .map(|p| Guard::parse(p))
                    .collect::<Result<Vec<_>, _>>()?;
                return Ok(if any { Guard::Any(list) } else { Guard::All(list) });
            }
        }
        if s == "authenticated" {
            return Ok(Guard::Authenticated);
        }
        let (name, arg) = s
            .split_once(':')
            .ok_or_else(|| format!("`{s}`: unknown guard; use authenticated, role:x, permission:x, owner:param, rate-limit:n/period, any(...), all(...) or a plugin guard such as plan:x"))?;
        let (name, arg) = (name.trim(), arg.trim());
        if arg.is_empty() {
            return Err(format!("`{s}`: `{name}` needs a value after `:`"));
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            || name.is_empty()
        {
            return Err(format!("`{s}`: `{name}` is not a guard name"));
        }
        Ok(match name {
            "role" => Guard::Role(arg.into()),
            "permission" => Guard::Permission(arg.into()),
            "owner" => Guard::Owner(arg.into()),
            "rate-limit" => {
                let (max, period) = arg.split_once('/').ok_or_else(|| {
                    format!("`{s}`: write it as rate-limit:<n>/<period>, e.g. rate-limit:100/min")
                })?;
                let max: u64 = max
                    .trim()
                    .parse()
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or_else(|| format!("`{s}`: `{max}` is not a positive number"))?;
                let period_secs = period_secs(period.trim())
                    .ok_or_else(|| format!("`{s}`: period `{period}` must be s, min, hour or day (optionally with a count, e.g. 10min)"))?;
                Guard::RateLimit { max, period_secs }
            }
            _ => Guard::Custom {
                name: name.into(),
                arg: arg.into(),
            },
        })
    }

    /// Every guard in this expression, depth first.
    pub fn walk(&self, f: &mut dyn FnMut(&Guard)) {
        f(self);
        if let Guard::All(v) | Guard::Any(v) = self {
            for g in v {
                g.walk(f);
            }
        }
    }
}

impl fmt::Display for Guard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let list = |v: &[Guard]| v.iter().map(|g| g.to_string()).collect::<Vec<_>>().join(", ");
        match self {
            Guard::Authenticated => f.write_str("authenticated"),
            Guard::Role(r) => write!(f, "role:{r}"),
            Guard::Permission(p) => write!(f, "permission:{p}"),
            Guard::Owner(p) => write!(f, "owner:{p}"),
            Guard::RateLimit { max, period_secs } => write!(f, "rate-limit:{max}/{period_secs}s"),
            Guard::All(v) => write!(f, "all({})", list(v)),
            Guard::Any(v) => write!(f, "any({})", list(v)),
            Guard::Custom { name, arg } => write!(f, "{name}:{arg}"),
        }
    }
}

fn period_secs(p: &str) -> Option<u64> {
    let split = p.find(|c: char| !c.is_ascii_digit()).unwrap_or(p.len());
    let (count, unit) = p.split_at(split);
    let count: u64 = if count.is_empty() { 1 } else { count.parse().ok()? };
    let unit = match unit {
        "s" | "sec" | "second" | "seconds" => 1,
        "m" | "min" | "minute" | "minutes" => 60,
        "h" | "hour" | "hours" => 3600,
        "d" | "day" | "days" => 86_400,
        _ => return None,
    };
    (count > 0).then_some(count * unit)
}

/// Splits on commas that are not inside parentheses.
fn split_top(s: &str) -> Result<Vec<String>, String> {
    let (mut depth, mut cur, mut out) = (0i32, String::new(), Vec::new());
    for c in s.chars() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth < 0 {
                    return Err(format!("`{s}`: unbalanced `)`"));
                }
            }
            ',' if depth == 0 => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    if depth != 0 {
        return Err(format!("`{s}`: unbalanced `(`"));
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    Ok(out.into_iter().map(|p| p.trim().to_string()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_guards() {
        assert_eq!(Guard::parse("authenticated").unwrap(), Guard::Authenticated);
        assert_eq!(Guard::parse("role:admin").unwrap(), Guard::Role("admin".into()));
        assert_eq!(
            Guard::parse("rate-limit:100/min").unwrap(),
            Guard::RateLimit {
                max: 100,
                period_secs: 60
            }
        );
        assert_eq!(
            Guard::parse("rate-limit:5/10s").unwrap(),
            Guard::RateLimit {
                max: 5,
                period_secs: 10
            }
        );
        assert_eq!(
            Guard::parse("any(role:admin, all(owner:id, plan:pro))").unwrap(),
            Guard::Any(vec![
                Guard::Role("admin".into()),
                Guard::All(vec![
                    Guard::Owner("id".into()),
                    Guard::Custom {
                        name: "plan".into(),
                        arg: "pro".into()
                    }
                ])
            ])
        );
        for bad in [
            "",
            "admin",
            "role:",
            "any(role:a",
            "rate-limit:x/min",
            "rate-limit:5/week",
            "any()",
        ] {
            assert!(Guard::parse(bad).is_err(), "{bad}");
        }
    }
}
