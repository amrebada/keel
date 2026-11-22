# ping

A tiny example plugin. `GET /ping` answers with a configurable message and emits `ping.received`. New apps from `keel init` include it so there's something to call, and its source (`plugins/ping/src/lib.rs`, about 70 lines) shows the whole plugin contract: manifest, validated config, a declared route, an emitted event, the log capability and a live config change.

```yaml
plugins:
  - use: ping@^0.1
    config:
      message: pong from shelf
```

| | |
| --- | --- |
| Version | 0.1.0 |
| Capabilities | `log` |
| Routes | `GET /ping` → `{"message": "pong"}` |
| Events | emits `ping.received` |
| Runtime config | `message` |

## Config

| Key | Default | Meaning |
| --- | --- | --- |
| `message` | `pong` | What `GET /ping` answers. Changes live with `keel plugins set ping message=hello`. |

Remove it from `app.yaml` once your app has real routes.
