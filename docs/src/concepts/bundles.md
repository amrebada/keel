# Bundles (composers)

A bundle is a plugin with no code. Its manifest lists other plugins and how to configure them from the bundle's own, smaller config. Before anything starts, Keel replaces the bundle's `app.yaml` entry with the plugins it expands into. The built-in bundle is [saas-auth](../plugins/saas-auth/index.md):

```yaml
plugins:
  - use: saas-auth@^1
    config:
      social: [google, apple]
      roles: [admin, member]
      admins: [you@example.com]
      with: { auth-core: { password_min_length: 12 } }
```

becomes auth-core, auth-oauth (with Google and Apple), auth-mfa and rbac. See the result with:

```sh
keel config expand
```

## Writing a bundle

A bundle crate has `plugin.yaml` with a `compose:` block, `config.schema.json`, and a one-line `src/lib.rs`:

```rust
pub struct SaasAuth;

impl SaasAuth {
    pub fn new() -> keel_sdk::composer::Composer {
        keel_sdk::composer!()
    }
}
```

The `compose:` block from saas-auth:

```yaml
id: saas-auth
version: 1.0.0
summary: Everything a SaaS app needs to sign users in …
compose:
  - use: auth-core@^0.1
    config:
      app_url: "${config.app_url}"
      signup: "${config.signup}"
  - use: auth-oauth@^0.1
    when: config.social
    config:
      redirect_base_url: "${config.app_url}"
      providers: { $keys: "${config.social}" }
  - use: auth-mfa@^0.1
    when: config.mfa
    config:
      issuer: "${config.app_name}"
  - use: rbac@^0.1
    config:
      roles:
        $keys: "${config.roles}"
        value: { permissions: [] }
        overrides: { "${config.admin_role}": { permissions: ["*"] } }
      admins: "${config.admins}"
config: config.schema.json
```

Template rules:

| Syntax | Meaning |
| --- | --- |
| `"${config.a.b}"` as a whole string | replaced by that value of the bundle's config, defaults filled in (keeps its type: list, bool, object) |
| `"…${config.x}…"` inside a longer string | interpolated as text |
| a key whose value ends up null | dropped, so the included plugin's default applies |
| `when: config.x` | keep the entry only when the value is set and not false, empty or zero |
| `{ $keys: "${config.list}", value: {...}, overrides: { name: {...} } }` | turn a list of names into a map, each name getting `value` merged with its `overrides` entry; a map passes through unchanged |

Every bundle also accepts `with: { <plugin id>: {...} }` in its config, deep-merged into that plugin's generated config, for settings the bundle doesn't expose.

A composer may not declare routes, capabilities, interfaces or migrations, and may not include itself.
