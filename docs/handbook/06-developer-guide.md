<!-- Reader: App developer · Mode: Tutorial → How-to -->
# Developer Guide

Get from a checkout of the **payment** module to a running service with its twelve REST endpoints
per entity, then extend it. The tutorial part holds your hand once; the recipes assume you know your
way around.

Commands here were run against `metaphor 0.2.0`. Where the top-level [README](../../README.md)
shows a `backbone-schema`/`backbone` command, use the `metaphor` form below — those are the ones
that work today.

## Prerequisites

- **Rust** (2021 edition toolchain) and **Cargo**.
- The **`metaphor`** CLI on your `PATH` (`metaphor --version` → `metaphor 0.2.0` or newer).
- A reachable **PostgreSQL** instance.

## What this module owns

Three entities, each with the full twelve-endpoint CRUD surface:

| Entity | Collection (route) | What it is |
|--------|--------------------|-----------|
| `PaymentEntry` | `/api/v1/payment_entries` | The settlement document — money moving |
| `PaymentAllocation` | `/api/v1/payment_allocations` | A knock-off line against one invoice |
| `ModeOfPayment` | `/api/v1/mode_of_payments` | How money moves (cash, transfer, QRIS, …) |

The *settlement* behaviour — posting to the GL and drawing invoices down — is custom logic in
`payment_write_service.rs`, not part of generated CRUD (see [Architecture](04-architecture.md#the-settlement-write-path--where-the-custom-5-lives)).

## Quickstart — prove the toolchain end to end

```bash
# From the module directory:
export DATABASE_URL="postgresql://root:password@localhost:5432/paymentdb"

# 1. Validate the schema.
metaphor schema schema validate

# 2. Apply the migrations (enums + the three payment tables + audit triggers).
metaphor migration run

# 3. Run the module's tests (unit + integration + the settlement-seam round-trip).
metaphor dev test
```

Expected: validation passes, migrations report `payment.payment_entries`, `payment.payment_allocations`,
and `payment.mode_of_payments` created (each under the module's own `payment` Postgres schema), and
the test run is green.

To see the HTTP surface, compose the module into a service and `metaphor dev serve`, then create a
receive payment:

```bash
curl -s -X POST localhost:8080/api/v1/payment_entries \
  -H 'content-type: application/json' \
  -d '{
        "paymentNumber": "PAY-0001",
        "companyId":     "00000000-0000-0000-0000-000000000001",
        "paymentType":   "receive",
        "partyType":     "customer",
        "postingDate":   "2026-07-05",
        "paidAmount":    "1000000.00",
        "bankAccountId":  "00000000-0000-0000-0000-0000000000b1",
        "partyAccountId": "00000000-0000-0000-0000-0000000000a1"
      }'
# → 201 { "id": "…", "paymentNumber": "PAY-0001", "status": "draft", "metadata": { "createdAt": "…" } }
```

Note the JSON is **camelCase** (`paymentNumber`, `createdAt`) even though the Rust and SQL are
snake_case — that is the generated `#[serde(rename_all = "camelCase")]` at work. The DTO also accepts
snake_case keys as aliases, so `payment_number` is tolerated too.

## Change an entity — edit the schema, regenerate

You never hand-edit the generated entity/DTO/handler. To add a field, change an enum, or add an
index, edit the model YAML and regenerate:

```bash
# 1. Edit the model — e.g. add a field to schema/models/payment_entry.model.yaml.

# 2. Validate, generate, migrate.
metaphor schema schema validate
metaphor schema schema generate --target all --force
metaphor migration generate AddFieldToPaymentEntry payment
metaphor migration run

# 3. Re-run tests.
metaphor dev test
```

(`payment` is the module name — auto-detected from the current directory when omitted, but passing
it explicitly is clearer in scripts.) Custom logic in `payment_write_service.rs`, `*_custom.rs`
files, and `// <<< CUSTOM` markers survives `generate --force`; everything else is overwritten.

## Key concepts

Five ideas carry you the rest of the way. One line each; the linked page explains *why*.

- **Schema YAML is the source of truth.** You edit [`schema/models/*.model.yaml`](../schema/RULE_FORMAT_MODELS.md);
  the entity, DTOs, migration, repository, service, handler, and routes are generated from it.
  ([Philosophy](01-philosophy.md).)
- **A module is a library, not a service.** It has no `main.rs`. A `backend-service` composes it
  via `Module::builder().with_database(pool).build()?` and mounts `module.http_routes()`.
  ([Architecture](04-architecture.md).)
- **Twelve endpoints come free per entity.** `BackboneCrudHandler` gives list / create / get /
  update / patch / soft_delete / restore / empty_trash / bulk_create / upsert / find_by_id /
  list_deleted, mounted under `/api/v1/<collection>`.
- **CRUD is inherited, not written.** `Service = GenericCrudService<…>` is a type alias;
  `Repository` is a newtype over `GenericCrudRepository`. You add methods, never a fresh `impl`.
  ([ADR-0002](adr/adr-0002-generic-crud.md).)
- **Custom code survives regeneration** if it sits in `// <<< CUSTOM` markers, `*_custom.rs` files,
  or a `user_owned` path. Anything else is overwritten by `generate --force`.
  ([ADR-0003](adr/adr-0003-custom-markers.md).)

## Recipes

### How do I add a second entity to a module?

Follow the golden path in the [Maintainer Guide → Adding a new entity](05-maintainer-guide.md#adding-a-new-entity-the-golden-path).
In short: add the `.model.yaml`, add it to `index.model.yaml` `imports:`, `validate`, `generate`,
`migration generate`, `migration run`, then register the service in `src/module.rs`.

### How do I add a business rule (e.g. "you can't allocate more than you paid")?

Write it in a hand-authored service file, not in the generated service. The module's settlement
invariant lives in [`payment_write_service.rs`](../../src/application/service/payment_write_service.rs)
(a `user_owned` file), which is exactly this pattern:

```rust
// application/service/payment_write_service.rs — the shape of a custom write path
if allocations.iter().map(|a| a.allocated_amount).sum::<Decimal>() > input.paid_amount {
    return Err(ServiceError::Validation(
        "Σ allocations exceeds paid_amount".into(),
    ));
}
// … then post the balanced AccountingPost and emit PaymentSettled.
```

For a smaller rule on a single entity, add a `*_custom.rs` beside the generated service (e.g.
`payment_entry_service_custom.rs`) and register it under a `// <<< CUSTOM` marker. See the
[custom-logic-specialist](../schema/EXAMPLES.md) territory.

### How do I add a non-CRUD endpoint?

Don't edit the generated handler. Add a handler fn in a `*_custom.rs`, compose it in `routes/`
beside the `BackboneCrudHandler` merge, and protect the file with a `user_owned` glob. Full steps:
[Maintainer Guide → Adding a non-CRUD endpoint](05-maintainer-guide.md#adding-a-non-crud-endpoint).

### How do I reference a user (or another module's entity)?

By **logical foreign key**, declared in the schema — never by copying the table in. The skeleton
already does this for audit actors:

```yaml
# schema/models/index.model.yaml
external_imports:
  - module: sapiens
    types: [User]
# …
created_by:
  type: uuid?
  attributes: ["@foreign_key(sapiens.User.id)"]
```

### How do I seed sample data?

Edit the seeders in `src/seeders/` (`mode_of_payment_seeder.rs`, `payment_entry_seeder.rs`,
`payment_allocation_seeder.rs`), then:

```bash
metaphor migration seed payment            # run Rust seeders
metaphor migration generate-seeds payment  # emit SQL seed files
```

The Indonesia `ModeOfPayment` defaults (cash, bank transfer, QRIS, e-wallet, …) are seeded through
the `id` overlay data-seed layer, not hard-coded into the `ModeType` enum.

## Configuration

Defaults live in [`config/application.yml`](../../config/application.yml); override per environment
and at runtime.

| Option | Default | When to change |
|--------|---------|----------------|
| `server.host` | `0.0.0.0` | Bind to a specific interface. |
| `server.port` | `8080` | Port conflicts / multi-service hosts. |
| `database.url` | `postgresql://root:password@localhost:5432/skeletondb` | **Always** in real deployments — override with the `DATABASE_URL` env var, which takes precedence. (The shipped default database name is a skeleton leftover; rename freely.) |
| `database.max_connections` | `10` | Tune to your Postgres pool budget. |
| `logging.level` | `info` | `debug`/`trace` when diagnosing; `warn` in noisy prod. |

Layered files: `application.yml` (base) → `application-dev.yml` / `application-prod.yml`
(overrides). `DATABASE_URL` in the environment always wins over the YAML.

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `backbone-schema: command not found` | Following the stale README | Use `metaphor schema schema …`. `backbone-schema` is not a separate binary here. |
| `metaphor migration run` can't connect | `DATABASE_URL` unset or Postgres down | `export DATABASE_URL=postgresql://…`; confirm Postgres is reachable. |
| My custom method vanished after regen | Code sat outside a protected region | Move it inside a `// <<< CUSTOM` marker, a `*_custom.rs` file, or a `user_owned` glob ([Maintainer Guide](05-maintainer-guide.md#regen-safety--the-rules-that-keep-your-logic-alive)). |
| New endpoint returns 404 | Route not composed, or service not registered | Merge the route in `routes/`; register the service field in `src/module.rs`. |
| `type PaymentStatus not found` after adding an enum variant | Migration not regenerated / not applied | Regenerate, `metaphor migration generate`, `metaphor migration run`. |
| Schema change ignored | Edited generated Rust instead of the YAML | Revert the Rust, edit `schema/models/*.model.yaml`, regenerate. |
| JSON field names look wrong (`created_at` vs `createdAt`) | Expecting snake_case on the wire | DTOs are `camelCase` by design; snake_case is DB/Rust only. |

---

Next: [Contributing](07-contributing.md) to send a change back, or the
[Glossary](08-glossary.md) to pin down a term.
