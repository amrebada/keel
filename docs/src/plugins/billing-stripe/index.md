# billing-stripe

Stripe subscriptions, one per tenant: plans in YAML, Stripe Checkout to subscribe, the Stripe customer portal to manage, signed webhooks that keep the subscription state in your database, and a `plan:<name>` route guard.

```yaml
plugins:
  - use: auth-core
  - use: rbac                      # optional: who in a team may manage billing
  - use: billing-stripe
    config:
      public_url: "https://api.shelf.app"
      success_url: "https://shelf.app/billing/success"
      plans:
        pro: { price: price_1PqR… }
        team: { price: price_1PqS…, includes: [pro] }
routes:
  /reports/**: { guard: [authenticated, "plan:pro"] }
```

| | |
| --- | --- |
| Version | 0.1.0 |
| Capabilities | `sql`, `http-client`, `secrets`, `log` |
| Requires plugins | `auth-core ^0.1` |
| Provides interface | `guard:plan` |
| Degrade level | `warn` |
| Setup | `stripe-account`, `stripe-prices`, `stripe-webhook` |

## Tenants

A subscription belongs to a **tenant**: the `X-Tenant-Id` header (a team), or the signed-in user's own id when the header is absent. So the same plugin works for per-user plans and per-team plans. With rbac in the app, starting checkout or opening the portal for a team needs `manage_permission` (default `billing.manage`) in that team.

## Config

| Key | Default | Meaning |
| --- | --- | --- |
| `plans` | required | Plan name to `{price, name?, includes?}`. `price` is a Stripe price id (`price_…`). `includes` lists plans this one also satisfies in `plan:` guards. |
| `public_url` | `http://localhost:8080` | Public base URL of this API, for the webhook endpoint. |
| `success_url` | `http://localhost:3000/billing/success` | Where Checkout sends the browser after paying; Stripe fills `{CHECKOUT_SESSION_ID}`. |
| `cancel_url` | `http://localhost:3000/billing` | Where Checkout sends the browser on cancel. |
| `portal_return_url` | `http://localhost:3000/billing` | Where the portal's back link goes. |
| `manage_permission` | `billing.manage` | With rbac, the permission needed in a tenant to start checkout or open the portal. |
| `api_base` | `https://api.stripe.com` | Stripe API base URL; tests point it at a mock. |

## Setup

`keel setup status` walks through three steps:

1. **stripe-account**: create a Stripe account and set `STRIPE_SECRET_KEY` (`sk_test_…` while testing). Checked by calling Stripe.
2. **stripe-prices**: create a product with a recurring price per plan and put each price id in `plans.<plan>.price`. Checked by asking Stripe for each price.
3. **stripe-webhook**: point Stripe at `{public_url}/billing/webhook` with the events `checkout.session.completed`, `customer.subscription.created`, `customer.subscription.updated` and `customer.subscription.deleted`, and set `STRIPE_WEBHOOK_SECRET` (`whsec_…`). Locally, `stripe listen --forward-to localhost:8080/billing/webhook` prints the secret.

## Routes

| Route | Does |
| --- | --- |
| `GET /billing/plans` | `{plans: [{plan, name, price, includes}]}` |
| `GET /billing/subscription` | `{tenant, plan, status, active, current_period_end, cancel_at_period_end}`; `status: "none"` before the first checkout. |
| `POST /billing/checkout` | `{"plan": "pro"}` → `{id, url, tenant, plan}`. Send the browser to `url`. |
| `POST /billing/portal` | `{url}` of the Stripe customer portal for the tenant. |
| `POST /billing/webhook` | Stripe's webhook. The signature is checked with `STRIPE_WEBHOOK_SECRET`; events are recorded once each. |

## The plan guard

`plan:pro` passes when the request's tenant has an `active` or `trialing` subscription to `pro`, or to a plan that `includes` it. Otherwise it answers:

```json
{"error":{"code":"plan_required","message":"this needs an active `pro` plan; start one with POST /billing/checkout","plan":"pro"}}
```

with status `402`.

## Errors

| Code | Status | When |
| --- | --- | --- |
| `unknown_plan` | 400 | checkout with a plan that isn't configured |
| `already_subscribed` | 409 | checkout while the tenant has an active subscription; use the portal |
| `no_customer` | 404 | portal before the tenant ever checked out |
| `bad_signature` | 400 | a webhook whose signature doesn't verify |
| `stripe_error`, `stripe_unreachable` | 502 | Stripe answered an error, or couldn't be reached |
| `plan_required` | 402 | the `plan:` guard failed |

## Events

`billing.subscription.updated {tenant, plan, status}` after each webhook that changes a subscription.

## Tables

`billing_stripe_customers`, `billing_stripe_subscriptions`, `billing_stripe_events`.
