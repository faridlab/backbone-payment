# ADR-002: The payment↔billing settlement seam (closing the cash loop, end-to-end)

**Status**: Accepted — Applied 2026-07-05 (proven end-to-end; exercises billing's inert drawdown hooks)
**Deciders**: Farid (owner), build session 2026-07-05
**Related**: billing ADR-001/002, `docs/erp/extension-contract.md` §5, `docs/erp/gl-posting-contract.md`

## Context

Billing raised real AR/AP invoices but deliberately left `outstanding_amount` set to grand and
`payment_schedules.paid_amount` inert — "backbone-payment's job" (billing council parking lot). This
ADR records the **settlement seam**: a real payment posts cash to the ledger AND draws the invoice's
outstanding down in billing, flipping it to `partially_paid`/`paid` — the close of the cash loop
(order-to-cash-to-bank, procure-to-pay-to-bank).

## Decision

1. **Every cross-module hop is a serialized envelope mapped by an ACL — zero normal Cargo edges.**
   - Payment posts the cash journal by emitting `AccountingPostEnvelope` through a `GlPostSink`; a
     composition ACL maps it into accounting's `PostingRequest` → real Journal + Ledger.
   - A posted payment emits `PaymentSettled { allocations[], … }`; the composition routes each
     allocation → billing `apply_settlement(invoice_ref, kind, amount)`, which draws down the invoice's
     `outstanding_amount` + payment schedules (fill-in-order, earliest installment first) and flips
     status. Billing **CLAMPS** — it knocks off `applied = min(amount, outstanding)` and returns
     `applied`; the ACL books the remainder as on-account (see decision 4).
   The shipped payment library has **no normal dependency** on billing or accounting
   (`cargo tree -e normal -i backbone-billing`/`-i backbone-accounting` are empty; both are
   dev-dependencies for the seam test only).
2. **The invariant is split, not shared.** Payment enforces `Σalloc ≤ paid` at create; billing bounds
   the drawdown to `outstanding` at apply. Neither module validates against the other's tables — the
   seam is an event carrying the allocation, and each side guards its own books.

4. **CLAMP-and-on-account closes the composition gap (council 2026-07-05, skeptic lead).** The two
   half-invariants did not compose: two receipts each allocating 600k to a 1,000,000 invoice both pass
   payment's per-payment `Σalloc ≤ paid` and both post `Cr A/R 600k` (A/R credited 1,200,000); if
   billing *rejected* the second (600k > 400k remaining), the cash was already on the ledger and — with
   `unallocated=0` — nothing booked it, so 600k was silently stranded and the GL A/R diverged from the
   subledger by 600k. Resolved by making `apply_settlement` a **total function**: it applies
   `min(requested, outstanding)`, returns `applied`, and the ACL books the remainder as an on-account
   party credit (already a credit balance on the `Cr A/R [customer]` control from the settlement post).
   Every rupiah of `paid` is then accounted — knocked-off **or** on-account — and `Σ Cr A/R = Σ
   knocked-off + Σ on-account` reconciles. The bounded-context split holds (billing still owns the
   drawdown bound; it clamps to it instead of throwing across the seam). Proven by SSEAM-2 (which fails
   under reject semantics: the second apply errors and the cash vanishes).
3. **Physical/financial decoupling + transition-gated emission carry over.** The payment + allocations
   commit first; the GL post is emitted after, is idempotent (`source_id = payment id`), and the
   `PaymentSettled` publish is gated on the pending→posted transition — so a concurrent double-post
   cannot draw an invoice down twice.

## Consequences

- **Proven, not asserted:** `tests/settlement_seam.rs` runs the full round-trip — billing posts a Sales
  Invoice (1,000,000, two installments 600k+400k) into the real ledger; a first payment (600,000) posts
  `Dr Bank · Cr A/R` and, via `PaymentSettled` → `apply_settlement`, draws the invoice to
  `partially_paid` with installment 1 paid and installment 2 untouched (fill-in-order); a second payment
  (400,000) settles the rest → `paid`, installment 2 paid. Both cash journals balance.
- **Extension-contract §5 discharged for the seam:** `scripts/settlement_seam_roundtrip.sh` regenerates
  **both** modules and asserts every ACL/consumer file is byte-identical and the seam stays green.
- This is the **fourth proven cross-module GL seam** and the one that closes the cash loop — the
  order-to-cash and procure-to-pay pipelines now run document → GL → settlement end-to-end.
- Residual / parking lot: a real event bus + payment service to own the ACL in production; bank
  clearing/reconciliation (backbone-banking); gateways; refunds/reversal posts; the on-account
  reconciliation UI; multi-currency.
- **Gated for the bus increment (council 2026-07-05):** `apply_settlement` is **not idempotent** —
  it has no dedup key, so at-least-once redelivery of one `PaymentSettled` against an invoice with
  headroom would double-draw (CLAMP prevents the *first-delivery* divergence but not redelivery).
  Parked because emission is fire-and-forget in-process today and there is no production bus (billing
  parked at-least-once delivery on the same precedent). **Gate:** when a bus/outbox lands,
  `apply_settlement` needs a settlement-record dedup key (`payment_id + allocation_id`) **before**
  go-live, plus an outbox to survive a crash between the posted-transition and `emit_settled`.
