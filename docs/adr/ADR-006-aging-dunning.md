# ADR-006: Aging & Dunning — the receivables-timeline read-model + escalation state

**Status**: Accepted — Applied 2026-07-26
**Deciders**: Farid (owner), build session 2026-07-26
**Related**: billing ADR-001 (boundary), ADR-002 (the seam pattern)

## Context

Billing creates + posts invoices and tracks `outstanding_amount` + `due_date`. Payment settles
them (the cash loop, closed via the settlement seam). But nothing **chases the unpaid** — no aging
(how late?), no dunning (what do we do about it?). Every billing doc defers "aging/dunning" to
payments. This ADR records how payment fills that gap.

## Decision

1. **Payment owns the aging read-model + dunning escalation state.** Aging is *derived data* —
   billing owns the live `outstanding_amount`; payment owns the days-past-due projection. Real-world
   rule: an invoice ages from its `due_date`; a payment allocation de-ages it.

2. **Entities:** `AgingSnapshot` + `AgingBucket` (the read-model: one snapshot per company/date/
   direction, with granular per-invoice rows bucketed by days-past-due) + `DunningRun` +
   `DunningAction` (the escalation state: one action per invoice per level, with a unique fence).

3. **Read seam via port + ACL, zero normal Cargo edges.** `BillingReceivablesPort` (declared in
   payment, implemented by the composition layer) reads billing's outstanding. Payment never imports
   billing (`cargo tree -e normal -i backbone-billing` is empty in the shipped crate).

4. **Cadence:** composition-layer cron calls `run_aging_snapshot` + `run_dunning` daily (same
   ownership split as the outbox relay). The module exposes the methods; registering the cron is an
   app-service concern.

5. **Idempotency fences.** Snapshot `unique(company_id, as_of_date, direction)`; action
   `unique(invoice_ref, invoice_kind, level)` — at-least-once cron delivery cannot duplicate.

## Consequences

- Proven end-to-end by `tests/aging_dunning_seam.rs` (ADSEAM-1): a 45-day-overdue invoice →
  `bucket_31_60`, dunning action at `final_notice`, re-run idempotent.
- Out of scope: credit policy, collections-agency integration, payment-plan re-negotiation, GL
  write-off post (accounting), `DunningPolicy` master (v1 uses hardcoded thresholds).
