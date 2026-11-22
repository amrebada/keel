# Events and interfaces

Plugins don't call each other directly. They have two ways to work together, both declared in `plugin.yaml` so the host can check them.

## Events: fire and forget

A plugin **emits** events it lists under `provides.events`; another plugin **consumes** events it lists under `consumes.events`, and receives them in `on_event`. Neither needs the other to be present.

```yaml
# rbac/plugin.yaml
consumes:
  events: [auth.user.created]      # give new users the default role
```

```rust
async fn on_event(&self, event: &Event) {
    if event.name == "auth.user.created" {
        let user_id = event.payload["user_id"].as_str();
        // …
    }
}
```

Emitting:

```rust
let emitter = ctx.emitter();                 // in init()
emitter.emit("stats.goal.reached", json!({ "goal": 12 })).await?;
```

Delivery is in-process, in subscription order, and `emit` returns when every handler has run. Emitting an event that isn't declared fails with `UndeclaredEvent`.

### Events the built-in plugins emit

| Event | Plugin | Payload |
| --- | --- | --- |
| `auth.user.created` | auth-core | `user_id`, `email` |
| `auth.user.verified` | auth-core | `user_id` |
| `auth.login` | auth-core | `user_id`, `session_id` |
| `auth.logout` | auth-core | `user_id`, `all` (true for logout-all) |
| `auth.password.reset` | auth-core | `user_id` |
| `auth.oauth.linked` | auth-oauth | `provider`, `user_id` |
| `auth.mfa.enabled`, `auth.mfa.disabled` | auth-mfa | `user_id` |
| `rbac.role.assigned`, `rbac.role.removed` | rbac | `user_id`, `role`, `tenant` |
| `resource.created` | resource | `resource`, `id`, `item` |
| `resource.updated` | resource | `resource`, `id`, `item`, `changed` (field names) |
| `resource.deleted` | resource | `resource`, `id` |
| `billing.subscription.updated` | billing-stripe | `tenant`, `plan`, `status` |
| `ping.received` | ping | `{}` |

## Interfaces: typed services

When one plugin needs to call another and get an answer, the provider offers a typed **interface** by name, and the consumer requires the provider plugin so it starts first.

```yaml
# auth-core/plugin.yaml
provides:
  interfaces: [identity-provider]

# auth-oauth/plugin.yaml
requires:
  plugins: { auth-core: "^0.1" }
```

```rust
// auth-core, in init():
ctx.provide::<dyn IdentityProvider>(IDENTITY_PROVIDER, service.clone())?;

// auth-oauth, in init():
let idp = ctx.interface::<dyn IdentityProvider>(IDENTITY_PROVIDER)?;
```

For optional dependencies, keep `ctx.interfaces()` (the shared registry) and look the interface up at request time; the `resource` plugin does this so it works with or without `rbac`.

### Interfaces in v1

| Name | Offered by | Trait | Used for |
| --- | --- | --- | --- |
| `identity-provider` | auth-core | `keel_sdk::auth::IdentityProvider` | find and create users, start sessions, verify access tokens (used by auth-oauth, auth-mfa, rbac and the host's guards) |
| `authorizer` | rbac | `keel_sdk::authz::Authorizer` | roles and permissions for `role:` and `permission:` guards, and `assign_role` for plugins that create tenants |
| `guard:plan` | billing-stripe | `keel_sdk::authz::GuardCheck` | the `plan:<name>` route guard |

auth-mfa also hooks into auth-core's login through `IdentityProvider::set_second_factor`, so every login asks for a second factor once a user has turned it on.

## Which one to use

- Something happened and others may care: **event**.
- You need an answer from another plugin: **interface**.
- You want a new kind of route guard: an interface named `guard:<name>` (see [Routes and guards](routes-and-guards.md#writing-a-guard-in-a-plugin)).
