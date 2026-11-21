//! `keel new-plugin`: creates a plugin crate from a template and compiles it
//! into this binary.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

const DEPS_END: &str = "# keel:plugins:end";
const REG_END: &str = "    // keel:plugins:end";

/// What `new_plugin` created or changed, for the summary it prints.
pub struct Scaffolded {
    pub dir: PathBuf,
    pub files: Vec<PathBuf>,
    pub crate_name: String,
}

/// Finds the Keel workspace root at or above `start`.
pub fn find_workspace(start: &Path) -> Result<PathBuf> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        if d.join("crates/keel/src/plugins.rs").is_file() && d.join("Cargo.toml").is_file() {
            return Ok(d.to_path_buf());
        }
        dir = d.parent();
    }
    bail!(
        "run this inside the keel repository (no crates/keel/src/plugins.rs found above {}). \
         Scaffolding plugins in your own project comes with `keel init` in phase 3",
        start.display()
    )
}

pub fn new_plugin(root: &Path, id: &str) -> Result<Scaffolded> {
    if !valid_id(id) {
        bail!("`{id}` is not a valid plugin id: use lowercase letters, digits and dashes, starting with a letter");
    }
    let dir = root.join("plugins").join(id);
    if dir.exists() {
        bail!("{} already exists", dir.display());
    }
    let crate_name = format!("keel-plugin-{id}");
    let type_name = type_name(id);
    let lib_name = crate_name.replace('-', "_");

    let cargo_toml = root.join("crates/keel/Cargo.toml");
    let registry_rs = root.join("crates/keel/src/plugins.rs");
    let cargo_text = fs::read_to_string(&cargo_toml)?;
    let registry_text = fs::read_to_string(&registry_rs)?;
    if !cargo_text.contains(DEPS_END) || !registry_text.contains(REG_END) {
        bail!("the `keel:plugins` markers are missing from crates/keel; add the plugin by hand");
    }

    let files = [
        ("Cargo.toml", template_cargo(&crate_name, id)),
        ("plugin.yaml", template_manifest(id)),
        ("config.schema.json", TEMPLATE_SCHEMA.to_string()),
        ("src/lib.rs", template_lib(id, &type_name)),
    ];
    fs::create_dir_all(dir.join("src"))?;
    let mut written = Vec::new();
    for (name, body) in files {
        let path = dir.join(name);
        fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
        written.push(path);
    }

    let dep = format!("{crate_name} = {{ path = \"../../plugins/{id}\" }}\n{DEPS_END}");
    fs::write(&cargo_toml, cargo_text.replacen(DEPS_END, &dep, 1))?;
    let reg = format!("        .register({lib_name}::{type_name}::new())\n{REG_END}");
    fs::write(&registry_rs, registry_text.replacen(REG_END, &reg, 1))?;
    written.push(cargo_toml);
    written.push(registry_rs);

    Ok(Scaffolded {
        dir,
        files: written,
        crate_name,
    })
}

fn valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !id.ends_with('-')
        && !id.contains("--")
}

/// `stripe-billing` -> `StripeBilling`
fn type_name(id: &str) -> String {
    id.split('-')
        .map(|p| {
            let mut c = p.chars();
            c.next()
                .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
                .unwrap_or_default()
        })
        .collect()
}

fn template_cargo(crate_name: &str, id: &str) -> String {
    format!(
        r#"[package]
name = "{crate_name}"
description = "Keel plugin `{id}`"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
publish = false

[dependencies]
keel-sdk.workspace = true
"#
    )
}

fn template_manifest(id: &str) -> String {
    format!(
        r#"# What this plugin is and what it needs. An AI reads this file, not the code.
id: {id}
version: 0.1.0
summary: Says hello. Replace this with one line on what the plugin does.
requires:
  host: "^0.1"
  capabilities: [log]
provides:
  routes:
    - GET /{id}/hello
  events: []
config: config.schema.json
"#
    )
}

const TEMPLATE_SCHEMA: &str = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "greeting": { "type": "string", "minLength": 1, "default": "Hello" }
  }
}
"#;

fn template_lib(id: &str, type_name: &str) -> String {
    format!(
        r#"//! Keel plugin `{id}`.

use keel_sdk::prelude::*;

#[derive(Default)]
pub struct {type_name};

impl {type_name} {{
    pub fn new() -> Self {{
        Self
    }}
}}

#[async_trait]
impl Plugin for {type_name} {{
    fn manifest(&self) -> &Manifest {{
        keel_sdk::manifest!()
    }}

    async fn init(&self, ctx: &mut InitContext) -> Result<(), PluginError> {{
        let greeting = ctx
            .config
            .get("greeting")
            .and_then(Value::as_str)
            .unwrap_or("Hello")
            .to_string();
        let log = ctx.capabilities.log()?;
        ctx.routes().get("/{id}/hello", move || {{
            let greeting = greeting.clone();
            let log = log.clone();
            async move {{
                log.info("said hello");
                Json(json!({{ "message": format!("{{greeting}} from {id}") }}))
            }}
        }})?;
        Ok(())
    }}
}}

#[cfg(test)]
mod tests {{
    use super::*;

    #[test]
    fn manifest_is_valid() {{
        let m = {type_name}::new().manifest().clone();
        assert_eq!(m.id, "{id}");
        assert!(m.declares("GET", "/{id}/hello"));
    }}
}}
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_type_names() {
        assert!(valid_id("hello"));
        assert!(valid_id("stripe-billing2"));
        assert!(!valid_id("Hello"));
        assert!(!valid_id("2fa"));
        assert!(!valid_id("a--b"));
        assert!(!valid_id("a-"));
        assert_eq!(type_name("stripe-billing"), "StripeBilling");
        assert_eq!(type_name("hello"), "Hello");
    }

    #[test]
    fn scaffolds_into_a_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("crates/keel/src")).unwrap();
        fs::write(root.join("Cargo.toml"), "[workspace]\n").unwrap();
        fs::write(
            root.join("crates/keel/Cargo.toml"),
            format!("[dependencies]\n{DEPS_END}\n"),
        )
        .unwrap();
        fs::write(root.join("crates/keel/src/plugins.rs"), format!("x\n{REG_END}\n")).unwrap();

        assert_eq!(find_workspace(&root.join("crates/keel")).unwrap(), root);
        let out = new_plugin(root, "hello-world").unwrap();
        assert_eq!(out.crate_name, "keel-plugin-hello-world");

        let manifest = fs::read_to_string(root.join("plugins/hello-world/plugin.yaml")).unwrap();
        let schema = fs::read_to_string(root.join("plugins/hello-world/config.schema.json")).unwrap();
        let m = keel_sdk::Manifest::from_sources(&manifest, Some(&schema)).unwrap();
        assert!(m.declares("GET", "/hello-world/hello"));

        let cargo = fs::read_to_string(root.join("crates/keel/Cargo.toml")).unwrap();
        assert!(cargo.contains(
            "keel-plugin-hello-world = { path = \"../../plugins/hello-world\" }\n# keel:plugins:end"
        ));
        let reg = fs::read_to_string(root.join("crates/keel/src/plugins.rs")).unwrap();
        assert!(reg.contains(".register(keel_plugin_hello_world::HelloWorld::new())"));

        assert!(new_plugin(root, "hello-world").is_err(), "refuses to overwrite");
    }
}
