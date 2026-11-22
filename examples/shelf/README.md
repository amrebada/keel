# Shelf

The reading-list API built step by step in the [tutorial](../../docs/src/tutorial/index.md).

- `app.yaml`: the finished app (Postgres + Redis; see step 9 for the secrets it needs).
- `reading-stats/`: the custom plugin from step 7. To compile it in, copy it to
  `plugins/reading-stats`, then add
  `keel-plugin-reading-stats = { path = "../../plugins/reading-stats" }` to
  `crates/keel/Cargo.toml` and
  `.register(keel_plugin_reading_stats::ReadingStats::new())` to
  `crates/keel/src/plugins.rs`, between the `keel:plugins` markers.
  `keel new-plugin reading-stats` does both registration edits for you.
