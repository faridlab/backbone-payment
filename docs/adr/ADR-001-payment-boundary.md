# ADR-001: Payment owns settlement; it posts ONE balanced cash journal per payment, allocation bounded

**Status**: Accepted — Applied 2026-07-05
**Deciders**: Farid (owner), build session 2026-07-05
**Related**: `docs/erp/financials.md`, `docs/erp/gl-posting-contract.md`,
`docs/erp/modules/backbone-payment.md`, billing ADR-001/002, ADR-002 (settlement seam)

## Context

`backbone-payment` is the settlement & reconciliation context of the Financials pillar. It records
money actually moving (receive from a customer, pay a supplier), allocates it across billing invoices,
and posts the cash movement to the ledger of record. It holds no masters — party/account/company are
logical FKs — and it is the module that closes the cash loop and exercises billing's inert
`outstanding_amount` + payment-schedule hooks. It does **not** own the bank statement (that is
backbone-banking's clearing seam); it owns the intent and allocation of payment to invoice.

## Decision

1. **One document + its allocations, one posting shape.** `PaymentEntry(+PaymentAllocation)`. Money is
   computed server-side; generic CRUD is not mounted on the guarded surface. On post, payment assembles
   **ONE** balanced settlement `AccountingPost` and refuses to emit unless `Σdebit = Σcredit`:
   - **receive:** `Dr Bank (paid) · Cr A/R (paid) [customer]`.
   - **pay:** `Dr A/P (paid) [supplier] · Cr Bank (paid)`.
   The A/R/A/P control is settled by the whole payment; the control line carries the party (subledger).
2. **Allocation is bounded — payment's half of the settlement invariant.** `Σ allocations ≤ paid_amount`
   (`over_allocated` otherwise); no negative allocation. **Billing owns the other half**
   (`amount ≤ outstanding`, in `apply_settlement`). This split keeps each module's invariant local — a
   module never reaches into the other's tables from its shipped library. It mirrors buying's
   received≤ordered / billed≤received bounding.
3. **Posting is idempotent, recoverable, and the seam event is transition-gated.** `source_id =
   payment id` (accounting dedupes). A rejected post leaves `posting_state=failed`, `status=draft`,
   retryable. The reconcile + `PaymentSettled` publish are gated on the pending→posted UPDATE's
   `rows_affected == 1`, so a concurrent double-post posts once and emits the seam event once — never
   double-drawing an invoice's outstanding (the lesson carried forward from billing's council).
4. **On-account remainder is first-class.** `unallocated_amount > 0` emits `PaymentReceivedOnAccount`
   (an unlinked credit awaiting reconciliation); the GL still credits the full control account.
5. **Refund / reversal is in scope (KEEP; completeness council 2026-07-05).** `reverse_payment` posts
   the sign-flipped mirror journal (`posting_type = "reversal"`, linked via `reverses_post_id`) and
   emits `PaymentCancelled` carrying the allocations, so an ACL routes each → billing's
   `reverse_settlement` to restore the invoice's outstanding + rewind schedules. It is **all-or-nothing**
   (settled allocations AND the on-account remainder unwind together — a partial reverse would reopen
   the split invariant), and exactly-once (accounting dedups the reversal post on `(company,
   source_type, source_id, posting_type)`; the emit is gated on the `posted→cancelled` transition). This
   is the exit for an on-account credit or a mis-applied settlement — the operator never hand-edits
   posted GL. Partial reversal / re-settlement-after-partial-reverse stay parked.
6. **IDR-only for now**; multi-currency settlement + FX revaluation deferred.

## Consequences

- The settlement math + allocation bound + state machine are locked by `tests/payment_golden_cases.rs`
  (5) and the failure/concurrency surface by `tests/integrity_probes.rs` (3, incl. the exactly-once
  concurrent-post probe).
- Payment is independently composable: it needs only a Postgres pool, a `GlPostSink`, and a
  `PaymentEventSink`.
- Deferred (per the brief): bank clearing (backbone-banking), gateways/requests/orders (basic gateway
  hookup — an operator keys `reference_no` manually until a gateway with a fee-posting line or callback
  dedup is onboarded), POS tender, advance-adjustment automation, multi-currency/FX,
  withholding-at-payment, and **partial** reversal / re-settlement-after-partial-reverse.
