//! Composer plugins: bundles with a manifest and a config schema but no code.
//! The host replaces a composer's `app.yaml` entry with the plugins listed
//! under `compose:` in its manifest before anything starts.

use crate::manifest::Manifest;
use crate::plugin::{InitContext, Plugin, PluginError};

/// A plugin made only of `plugin.yaml` (with a `compose:` block) and
/// `config.schema.json`. Build one with [`crate::composer!`].
pub struct Composer {
    manifest: Manifest,
}

impl Composer {
    /// Panics if the manifest is invalid or has no `compose:` block; both
    /// are build mistakes.
    pub fn from_sources(plugin_yaml: &str, config_schema: Option<&str>) -> Self {
        let manifest = Manifest::from_sources(plugin_yaml, config_schema)
            .unwrap_or_else(|e| panic!("invalid composer manifest: {e}"));
        assert!(
            manifest.is_composer(),
            "composer `{}` has no compose: block",
            manifest.id
        );
        Self { manifest }
    }
}

#[crate::async_trait]
impl Plugin for Composer {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    async fn init(&self, _ctx: &mut InitContext) -> Result<(), PluginError> {
        Err(PluginError::other(format!(
            "`{}` is a composer; the host expands it into its plugins and never starts it",
            self.manifest.id
        )))
    }
}

/// A [`Composer`] from the calling crate's `plugin.yaml` and `config.schema.json`.
#[macro_export]
macro_rules! composer {
    () => {
        $crate::composer::Composer::from_sources(
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/plugin.yaml")),
            Some(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/config.schema.json"
            ))),
        )
    };
}
