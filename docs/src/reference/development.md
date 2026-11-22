# Repository layout and development

## Layout

| Path | What it is |
| --- | --- |
| `crates/keel-sdk` | What plugins depend on: `Plugin` trait, manifest format, capability traits, hook bus, auth and authz interfaces |
| `crates/keel-sdk/schemas/plugin.schema.json` | JSON Schema for `plugin.yaml` |
| `crates/keel-core` | Host library: config loader, secrets, plugin loading and bundle expansion, HTTP host, route guards and validation, setup tracking, migration planner, live changes and admin API |
| `crates/keel-core/schemas/app.schema.json` | JSON Schema for `app.yaml` |
| `crates/keel-providers` | SQL (sqlx), Redis, in-memory, crypto, HTTP, email (console, SMTP, Resend, SES) and Vault providers |
| `crates/keel` | The `keel` binary: CLI, `dev` loop, MCP server, supervisor, scaffolding |
| `crates/keel/src/plugins.rs` | The list of compiled-in plugins (`keel new-plugin` edits it) |
| `plugins/` | Compiled-in plugins (`saas-auth` is a bundle with no code) |
| `examples/team-tasks` | The v1 success test: spec, acceptance tests, results and benchmark transcripts |
| `examples/shelf` | The app from the tutorial, with its custom plugin |
| `docs/` | This documentation (mdBook) |
| `site/` | The landing page; the docs build into `site/docs/` |

## Build and test

```sh
cargo build -p keel
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Postgres, MySQL and Redis tests run when these are set (CI sets them with service containers):

```sh
export KEEL_TEST_POSTGRES_URL=postgres://postgres:keel@127.0.0.1:5432/postgres
export KEEL_TEST_MYSQL_URL=mysql://root:keel@127.0.0.1:3306/keel
export KEEL_TEST_REDIS_URL=redis://127.0.0.1:6379
```

CI also scaffolds a plugin with `keel new-plugin ci-probe` and checks that it passes fmt, clippy and its tests, so the template never rots.

## Build the docs

The docs use [mdBook](https://rust-lang.github.io/mdBook/):

```sh
cargo install mdbook          # or download a release binary
mdbook serve docs --open      # live preview at localhost:3000
mdbook build docs             # writes site/docs/
```

The GitHub Pages workflow (`.github/workflows/pages.yml`) builds the book into `site/docs/` and publishes `site/` on every push to `main`.
