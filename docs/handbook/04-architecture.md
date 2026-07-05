<!-- Reader: Maintainer · Mode: Explanation -->
# Architecture

A Backbone module is a **library crate** that owns one bounded domain as four DDD layers. It does
not run on its own — a `backend-service` composes it, hands it a database pool, and mounts its
router. Everything in `src/` is either generated from the schema YAML or lives inside a regen-safe
custom region. This page shows the system top-down (C4), then traces one request through all four
layers.

## 1. Context

Who uses the module, and what it depends on.

```mermaid
C4Context
    title System Context — a Backbone module
    Person(dev, "App developer", "Edits the schema YAML, writes custom logic")
    System(module, "Backbone module (this crate)", "One bounded domain, 4-layer DDD, schema-generated")
    System_Ext(svc, "backend-service", "Composes modules, owns main(), installs tracing")
    System_Ext(pg, "PostgreSQL", "Owns the module's schema + tables")
    System_Ext(sapiens, "sapiens module", "Owns User identity")
    System_Ext(cli, "metaphor CLI", "Generates code + migrations from the schema")

    Rel(dev, module, "edits schema, writes custom code")
    Rel(dev, cli, "runs generate / migrate / test")
    Rel(cli, module, "writes generated source + migrations")
    Rel(svc, module, "builds Module, mounts http_routes()")
    Rel(module, pg, "SQLx, compile-time-checked")
    Rel(module, sapiens, "logical FK (created_by → sapiens.User.id)")
```

*What to notice: the module is a **dependency**, never an entrypoint. The `metaphor` CLI writes
into it; a service consumes it; identity comes from a **sibling module by logical reference**, not
a copied-in table.*

## 2. Containers

The runnable/deployable pieces and how they talk. The module compiles into the service binary;
there is no separate module process.

```mermaid
flowchart LR
    client[HTTP client] -->|REST /api/v1/payment_entries| svc
    subgraph svc[backend-service process]
        router[Axum Router]
        subgraph mod[Backbone module - linked in]
            handler[BackboneCrudHandler]
            service[GenericCrudService]
            repo[GenericCrudRepository]
        end
        router --> handler --> service --> repo
    end
    repo -->|SQLx| pg[(PostgreSQL<br/>own schema)]
```

*What to notice: the module contributes a `Router` that the service **merges** — the same object
Axum uses everywhere. Nothing about the module is a special runtime; it is ordinary linked-in Rust.*

## 3. Components / modules — the DDD 4-layer shape

Dependencies point **inward only**. Domain depends on nothing.

```mermaid
flowchart TD
    P["Presentation<br/>presentation/http/payment_entry_handler.rs<br/>presentation/http/routes/generated.rs"]
    A["Application<br/>application/service/payment_entry_service.rs<br/>presentation/dto/payment_entry_dto.rs"]
    D["Domain<br/>domain/entity/payment_entry.rs<br/>domain/repositories/payment_entry_repository.rs"]
    I["Infrastructure<br/>infrastructure/persistence/payment_entry_repository.rs"]

    P --> A
    A --> D
    I --> D
    P -. mounts .-> M["configure_routes(HttpServices) — presentation/http/routes/generated.rs"]
```

The payment module owns three entities — `PaymentEntry`, `PaymentAllocation`, and `ModeOfPayment`.
`PaymentEntry` is the exemplar below; the other two follow the identical shape (their own
`*_service.rs`, `*_repository.rs`, `*_dto.rs`, `*_handler.rs`).

| Layer | Directory | Holds for `PaymentEntry` | May depend on |
|-------|-----------|--------------------------|---------------|
| **Domain** | `src/domain/` | `PaymentEntry` entity (+ id, builder, `apply_patch`, audit accessors), the `PaymentType` / `PaymentPartyType` / `PaymentStatus` / `GlPostingState` enums, the `PaymentEntryRepository` **trait** (port) | nothing |
| **Application** | `src/application/` | `PaymentEntryService` (type alias over `GenericCrudService`), Create/Update/Patch/Response DTOs and their conversions, `ServiceError`/`ServiceResult` (re-exported from `backbone-core`) | domain |
| **Infrastructure** | `src/infrastructure/` | `PaymentEntryRepository` newtype over `GenericCrudRepository<PaymentEntry, SoftDelete>` | domain, application |
| **Presentation** | `src/presentation/` | `create_payment_entry_routes()` wiring `BackboneCrudHandler`, `PaymentEntryError` → HTTP mapping, `configure_routes(HttpServices)` | application |
| **Composition** | `src/module.rs`, `src/lib.rs` | `Module` / `ModuleBuilder`, public re-exports | all layers (it is the root) |

A subtlety worth internalizing: there are **two `PaymentEntryRepository`s**. The domain layer defines
a `trait PaymentEntryRepository` (the *port* — the contract). The infrastructure layer defines a
`struct PaymentEntryRepository` (the *adapter* — a newtype over `GenericCrudRepository`). The port is
the contract; the adapter is the Postgres implementation.

## 4. Data & control flow — `POST /api/v1/payment_entries` end to end

Trace one create request, top to bottom and back. This is the **generated CRUD path** — a plain
`PaymentEntry` insert. The *settlement* path (posting to the GL, drawing invoices down) is custom
logic that sits beside it; see [The settlement write path](#the-settlement-write-path--where-the-custom-5-lives) below.

```mermaid
sequenceDiagram
    actor Client
    participant H as BackboneCrudHandler
    participant S as PaymentEntryService (GenericCrudService)
    participant R as PaymentEntryRepository (newtype)
    participant G as GenericCrudRepository
    participant DB as PostgreSQL

    Client->>H: POST /api/v1/payment_entries {paymentNumber, paymentType, paidAmount, …}
    Note over H: deserialize CreatePaymentEntryDto<br/>(camelCase, snake_case aliases), validate
    H->>S: create(dto)
    Note over S: FromCreateDto: CreatePaymentEntryDto → PaymentEntry<br/>(uuid v4 id, default metadata, status=draft)
    S->>R: save(&payment_entry)
    R->>G: Deref → save
    G->>DB: INSERT INTO payment.payment_entries (...)
    Note over DB: audit trigger sets<br/>metadata.created_at / updated_at
    DB-->>G: row
    G-->>S: PaymentEntry
    S-->>H: PaymentEntry
    Note over H: PaymentEntry → PaymentEntryResponseDto (From)
    H-->>Client: 201 { id, paymentNumber, status: "draft", metadata }
```

*What to notice:* four layers, but **only the schema-declared shapes cross them** —
`CreatePaymentEntryDto` in, `PaymentEntry` through the middle, `PaymentEntryResponseDto` out. Every
conversion is generated. The `created_at`/`updated_at` stamps are set by a **Postgres trigger**
([`20260426220006_add_audit_triggers.up.sql`](../../migrations/20260426220006_add_audit_triggers.up.sql)),
not by Rust — so audit timestamps hold even for writes that bypass the service.

### The twelve endpoints, for free

`create_payment_entry_routes()` calls `BackboneCrudHandler::routes(service, "/payment_entries")`.
That single call wires **all twelve** endpoints; you write none of them:

`list` · `create` · `get` · `update` · `patch` · `soft_delete` · `restore` · `empty_trash` ·
`bulk_create` · `upsert` · `find_by_id` · `list_deleted`

`configure_routes(HttpServices)` in [`presentation/http/routes/generated.rs`](../../src/presentation/http/routes/generated.rs)
merges all three entities' routers and nests them under `/api/v1`, so the create endpoint above is
`POST /api/v1/payment_entries` (and likewise `/mode_of_payments`, `/payment_allocations`).

### The settlement write path — where the custom 5% lives

The generated CRUD above lets you *store* a payment. It does **not** move money in the ledger — that
is the interesting part, and it is hand-authored in a `user_owned` file the generator never touches:
[`payment_write_service.rs`](../../src/application/service/payment_write_service.rs). On **post**, it
assembles ONE balanced settlement `AccountingPost` (receive: `Dr Bank · Cr A/R [customer]`; pay:
`Dr A/P [supplier] · Cr Bank`) and emits `PaymentSettled{ allocations }` so an anti-corruption layer
draws each invoice down in billing. This is the textbook division of labour: **the generated layer
carries the plumbing; the custom write service carries the domain invariant** (`Σ allocations ≤
paid_amount`). See [ADR-002 — the settlement seam](../adr/ADR-002-settlement-seam.md).

## Where persistence semantics come from

- **Soft delete** is structural: `config.soft_delete: true` in [`index.model.yaml`](../../schema/models/index.model.yaml)
  → `GenericCrudRepository<PaymentEntry, SoftDelete>` → `soft_delete`/`restore`/`empty_trash`/`list_deleted`
  operate on `metadata.deleted_at`, and a partial index on `(metadata->>'deleted_at')` keeps the
  live-row query fast.
- **Audit** (`config.audit: true`) → the `metadata` JSONB column carrying `created_at`, `updated_at`,
  `deleted_at`, `created_by`, `updated_by`, `deleted_by`. Timestamps are trigger-managed; the `*_by`
  actor fields are logical FKs to `sapiens.User.id`.
- **Own schema per module** → migrations emit `CREATE SCHEMA <module>` and qualify tables as
  `<module>.<table>`, so two modules never collide on a table name.

## Key decisions

- [ADR-0001](adr/adr-0001-schema-yaml-ssot.md) — schema YAML is the single source of truth.
- [ADR-0002](adr/adr-0002-generic-crud.md) — services/repositories are generic, inherited not written.
- [ADR-0003](adr/adr-0003-custom-markers.md) — regen-safety via CUSTOM markers and `user_owned`.

---

Next: [Maintainer Guide](05-maintainer-guide.md) — how to add a feature without breaking the machine.
