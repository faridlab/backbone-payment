<!-- Reader: All · Mode: Reference -->
# Glossary — ubiquitous language

One term, one meaning, used everywhere in this handbook and in the code. When a term here names a
type or file, that name is exact. If you find a doc using a different word for one of these, the doc
is the bug.

### Aggregate / Entity
A domain object with identity and a lifecycle, defined by one `schema/models/<name>.model.yaml`.
In this module: `PaymentEntry`, `PaymentAllocation`, `ModeOfPayment`. Generated into
`src/domain/entity/<name>.rs` with a strongly-typed id, a builder, `apply_patch`, and audit
accessors. `PaymentEntry` is the aggregate root; `PaymentAllocation` lines are its children.

### Application layer
The use-case layer (`src/application/`): services and DTOs. Depends on the domain; knows nothing
about HTTP or SQL.

### Audit metadata
The `metadata` JSONB field (`created_at`, `updated_at`, `deleted_at`, `created_by`, `updated_by`,
`deleted_by`) added when `config.audit: true`. Timestamps are set by a Postgres trigger; the `*_by`
actor fields are logical FKs to `sapiens.User.id`.

### `BackboneCrudHandler`
The `backbone-core` type that produces an Axum `Router` with all **twelve** CRUD endpoints for an
entity. Invoked as `BackboneCrudHandler::<…>::routes(service, "/collection")`. You never hand-write
these routes.

### Bounded context
The single business domain a module owns. One module = one bounded context. A module never edits
another's schema; it references other modules by logical FK.

### Composition root
`src/module.rs` — the `Module` struct and `ModuleBuilder`. Wires each service to its repository and
composes the routers. The one place that is allowed to depend on every layer.

### CUSTOM marker
A `// <<< CUSTOM … // END CUSTOM` region inside a generated file. Content between the markers
survives regeneration. Spelling varies per file (`// <<< CUSTOM METHODS START >>>`, `// <<< CUSTOM
DTOs`, …) — match what is already there.

### DTO (Data Transfer Object)
A wire-shape struct in `src/application/dto/`. Per entity: `Create…Dto`, `Update…Dto`, `Patch…Dto`,
`…ResponseDto`, `…SummaryDto`, `…ListResponseDto`. Serialized `camelCase`. Generated, with
`From`/`Apply` conversions to and from the entity.

### Domain layer
The innermost layer (`src/domain/`): entities, value objects, enums, invariants, and repository
**traits** (ports). Depends on nothing.

### Generation targets
The 31 kinds of artifact `metaphor schema schema generate` can emit (`rust`, `sql`, `dto`,
`handler`, `repository`, `service`, `proto`, `openapi`, …). `--target all` (default) emits the lot;
a comma-separated subset emits part.

### `GenericCrudRepository` / `GenericCrudService`
The `backbone-orm` / `backbone-core` generics that carry all standard CRUD. A module's repository is
a **newtype** over `GenericCrudRepository<Entity, SoftDelete>`; its service is a **type alias** over
`GenericCrudService<Entity, CreateDto, UpdateDto, Repository>`. Inherited, never re-implemented.

### Infrastructure layer
The adapter layer (`src/infrastructure/`): repository implementations, cache, messaging, jobs.
Depends on domain and application.

### Logical foreign key
A cross-module reference declared with `@foreign_key(module.Type.field)` (e.g.
`@foreign_key(sapiens.User.id)`). It documents the relationship and is *not* enforced by a database
constraint, so modules stay independently deployable.

### `metaphor`
The workspace CLI (v0.2.0) that orchestrates the projects and dispatches to plugins
(`metaphor-schema`, `metaphor-codegen`, `metaphor-dev`). Prefer it over raw `cargo`/`sqlx`. Note:
the standalone `backbone-schema` binary the README mentions is **not** installed; use `metaphor
schema schema …`.

### Module
A **library crate** owning one bounded context in 4-layer DDD, schema-driven. `[lib]` only — no
`main.rs`. Composed into a `backend-service`; never run alone. This repo is the **payment** module.

### Own schema (per module)
Each module gets its own Postgres schema (`schema: payment` in `index.model.yaml`). Migrations
`CREATE SCHEMA <module>` and qualify tables as `<module>.<table>`, so modules never collide on a
table name.

### Port / Adapter
The DDD names for the two `PaymentEntryRepository`s: the **port** is the domain-layer `trait`
(the contract); the **adapter** is the infrastructure-layer `struct` (the Postgres implementation).

### Presentation layer
The transport layer (`src/presentation/`, `src/routes/`): HTTP handlers, route composition, and
optionally gRPC/GraphQL. Depends on the application layer.

### Regeneration (regen)
Re-running `metaphor schema schema generate … --force` to rebuild all downstream code from the
schema. Overwrites everything **outside** a protected region (CUSTOM markers, `*_custom.rs`,
`user_owned` globs).

### Schema (the SSoT)
`schema/models/*.model.yaml` — the single source of truth. Every entity struct, DTO, migration,
repository, service, handler, and route is generated from it. Not to be confused with the *Postgres
schema* (the per-module namespace).

### Soft delete
Marking a row deleted (`metadata.deleted_at` set) instead of removing it, enabled by
`config.soft_delete: true`. Backs the `soft_delete` / `restore` / `empty_trash` / `list_deleted`
endpoints.

### Twelve endpoints
The standard CRUD surface every entity gets from `BackboneCrudHandler`: `list`, `create`, `get`,
`update`, `patch`, `soft_delete`, `restore`, `empty_trash`, `bulk_create`, `upsert`, `find_by_id`,
`list_deleted`.

### `user_owned`
The `metaphor.codegen.yaml` key listing glob paths the generator skips wholesale — never reads,
merges, or deletes. The skeleton protects `tests/features/**` and `docs/**` (this handbook lives
under one of them). This module additionally owns the settlement write path
(`payment_write_service.rs`, `payment_gl.rs`, `payment_events.rs`).

---

## Payment domain terms

The ubiquitous language of *this* module. These names are exact — they match the schema enums and
entities in [`schema/models/`](../../schema/models/).

### `PaymentEntry`
The settlement document — one record of money actually moving. It carries a `paid_amount`, a
`payment_type` (`receive` / `pay` / `internal_transfer`), the bank and party GL accounts, and a
lifecycle `status`. Its children are `PaymentAllocation` lines. On post it emits one balanced
settlement `AccountingPost`.

### `PaymentAllocation`
One knock-off line: this payment applied against one invoice — the reconciliation. Holds
`invoice_ref` (the settled invoice, a logical reference into billing), `invoice_kind`
(`sales` / `purchase`), and `allocated_amount`. The per-payment invariant is `Σ allocated_amount ≤
paid_amount`.

### `ModeOfPayment`
How money moved — a reference master (cash, bank transfer, card, e-wallet, virtual account, QRIS).
Carries a `default_account_id` (the Bank/Cash GL account a payment defaults to). Indonesia defaults
are seeded via the `id` overlay data-seed layer, not hard-coded into the `ModeType` enum.

### `payment_type`
Direction of a payment: `receive` (from a customer, settles A/R), `pay` (to a supplier, settles
A/P), or `internal_transfer` (between own accounts, no party).

### `party` / `party_type`
The counterparty a payment settles: `customer` (A/R subledger), `supplier` (A/P subledger), or
`employee`. `party_id` is a logical FK into the party module — null for an internal transfer.

### Allocated vs. unallocated
`allocated_amount` is Σ of the allocation lines knocked off invoices; `unallocated_amount` is
`paid_amount − allocated_amount`, kept **on account** as a party credit. Every rupiah of `paid` is
accounted for — knocked-off or on-account.

### On-account
Money received but not (yet) matched to an invoice. It sits as a credit balance on the party's A/R
control account until a later allocation consumes it. See ADR-002's CLAMP-and-on-account rule.

### Settlement `AccountingPost`
The single balanced GL entry a posted payment emits into `backbone-accounting` — *receive:*
`Dr Bank · Cr A/R [customer]`; *pay:* `Dr A/P [supplier] · Cr Bank`. Idempotent on `source_id =
payment id`.

### `PaymentSettled` / the settlement seam
The event a posted payment publishes, carrying its allocation lines. A composition-level
anti-corruption layer routes each allocation to billing's `apply_settlement(invoice_ref, kind,
amount)`, which draws the invoice's `outstanding_amount` down and flips its status. Payment has **no
normal Cargo dependency** on billing or accounting — the seam is a serialized envelope, not a code
edge. See [ADR-002](../adr/ADR-002-settlement-seam.md).

### `posting_state` (`GlPostingState`)
The GL-reconciliation state of a payment, distinct from its document `status`: `pending` (not yet
posted), `posted` (confirmed by accounting), or `failed` (rejected). The `PaymentSettled` emission is
gated on the `pending → posted` transition, so a concurrent double-post never draws an invoice down
twice.
