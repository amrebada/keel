//! Schema migrations a plugin ships. The host runs them in order, records
//! them, and refuses destructive ones unless the user allows them with
//! `keel migrate --allow-destructive <plugin>/<id>`.

/// One migration. `up` may hold several statements separated by `;`.
/// Write `{ns}` before every table name; the host replaces it with the
/// plugin's prefix (e.g. `auth_core_`), and refuses tables outside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migration {
    /// Sorts the migrations, e.g. `001_users`. Never rename one that shipped.
    pub id: String,
    pub up: String,
    /// Optional undo, run by `keel migrate --down`.
    pub down: Option<String>,
}

impl Migration {
    pub fn new(id: impl Into<String>, up: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            up: up.into(),
            down: None,
        }
    }

    pub fn with_down(mut self, down: impl Into<String>) -> Self {
        self.down = Some(down.into());
        self
    }
}
