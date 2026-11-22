# Migrations

Plugins that store data ship their own SQL migrations, one set per database dialect. Keel plans them, classifies each as additive or destructive, runs the safe ones, and refuses the risky ones until you allow them by name.

## Plan, then apply

```sh
keel migrate                 # dry run: what would run, and whether it's safe
keel migrate --apply         # run pending additive migrations
```

```text
migrations: 3 pending
  additive    auth-core/001_init  tables: auth_core_sessions, auth_core_tokens, auth_core_users
  additive    rbac/001_init  tables: rbac_assignments, rbac_grants
  additive    resource/books_00_table  tables: resource_books
dry run: nothing changed. Run with --apply to run the additive ones
```

`keel serve`, `keel dev` and live changes run additive migrations automatically. A pending destructive migration stops the server from starting, and a live change that needs one is refused, leaving the server as it was.

## Additive and destructive

| Additive (runs automatically) | Destructive (needs your permission) |
| --- | --- |
| `CREATE TABLE`, `CREATE INDEX` | `DROP` of anything |
| `ALTER TABLE … ADD` | column drop, rename or type change |
| `INSERT` | `TRUNCATE`, any `DELETE` |
| `UPDATE … WHERE …` | `UPDATE` without `WHERE`, anything not recognized |

To run a destructive migration:

```sh
keel migrate --allow-destructive resource/books_30_drop_isbn
keel migrate --apply --allow-destructive resource/books_30_drop_isbn
```

The plan shows how many rows each table has. With `--apply`, the tables it touches are copied to `keel_backup_<time>_<table>` first. `keel migrate --apply` exits non-zero while a destructive migration is still blocked, so CI notices.

## Safety rules

- A migration may only touch tables and indexes in its plugin's namespace. Plugin SQL writes `{ns}` before each table name (`CREATE TABLE {ns}users`), and Keel replaces it with the prefix (`auth_core_`). A migration that names another plugin's table is refused.
- Applied migrations are recorded in `keel_migrations` with a checksum. Editing a migration that already ran is an error: add a new one.
- Every run is written to the `keel_audit` table, which `keel audit` reads.

## Config-driven migrations

Some plugins generate migrations from their config. The `resource` plugin makes one migration per table and one per field (`books_00_table`, `books_10_title`, `books_20_isbn_unique`), so adding a field adds a column, and changing rules like `max_length` needs no migration. To change a field's type, add a field with a new name. Writing such a plugin uses `Plugin::migrations_for(dialect, config)`; ids must be stable for the same config.

## Writing migrations in a plugin

```rust
fn migrations(&self, _dialect: &str) -> Vec<Migration> {
    vec![
        Migration::new(
            "001_init",
            "CREATE TABLE {ns}entries (
                id VARCHAR(36) NOT NULL PRIMARY KEY,
                body VARCHAR(2000) NOT NULL,
                created_at BIGINT NOT NULL
            );
            CREATE INDEX {ns}entries_created ON {ns}entries (created_at)",
        )
        .with_down("DROP TABLE {ns}entries"),
    ]
}
```

One migration may hold several statements separated by `;`. Portable types (`VARCHAR`, `BIGINT`) let the same text run on all three databases, as auth-core does; return different SQL per `dialect` when you need to.

Declare `sql` under `requires.capabilities` and `migrations: built-in` in `plugin.yaml`. Ids sort the migrations; never rename one that shipped. `dialect` is `postgres`, `mysql` or `sqlite`.
