# BRD — backbone-payment

> Business Requirements & Rules. Tier 2 · Financials. Date: 2026-07-05. Pairs with
> `docs/business-flows/golden-cases.md`.

## Documents
PaymentEntry (+PaymentAllocation lines; the settlement) · ModeOfPayment (reference master).

## Business rules
**BR-1 (server-side money).** `allocated_amount = Σ allocation amounts`; `unallocated_amount =
paid_amount − allocated_amount`. 2dp half-up. `paid_amount > 0`.

**BR-2 (allocation bound — the module invariant).** `Σ allocations ≤ paid_amount` — you cannot
allocate more money than actually moved. → `over_allocated`. No negative allocation. This is
payment's half of the settlement invariant; **billing owns the other half** (`amount ≤ outstanding`,
BR-7).

**BR-3 (unique numbers).** Payment numbers unique (soft-delete aware). → `duplicate_number`.

**BR-4 (one balanced settlement post — ADR-001).** On post, payment assembles ONE balanced
`AccountingPost` and refuses to emit unless `Σdebit = Σcredit`:
- **receive:** `Dr Bank (paid) · Cr A/R (paid) [customer party]`.
- **pay:** `Dr A/P (paid) [supplier party] · Cr Bank (paid)`.
The A/R/A/P control is settled by the whole payment; the receivable/payable line carries the party.

**BR-5 (idempotent + recoverable + gated).** `source_id = payment id`; a re-post reuses the recorded
ack. A rejected post leaves `posting_state=failed`, `status=draft`, no journal — retryable. Only IDR
is supported end-to-end for now (→ `unsupported_currency`). The seam event is **gated on the
pending→posted transition** — a concurrent double-post posts once to the GL and emits `PaymentSettled`
**exactly once** (no double drawdown of an invoice).

**BR-6 (on-account remainder).** `unallocated_amount > 0` emits `PaymentReceivedOnAccount` — an
unlinked credit awaiting later reconciliation. The GL still credits the full A/R (standard on-account
treatment); the remainder is a credit balance on the party.

**BR-7 (settlement seam — ADR-002, CLAMP).** A posted payment emits `PaymentSettled{allocations}`; an
ACL routes each allocation → billing's `apply_settlement(invoice_ref, kind, amount)`, which knocks off
**`applied = min(amount, outstanding)`**, draws the invoice's `outstanding_amount` + payment schedules
(fill-in-order) down, flips status → `partially_paid`/`paid`, and **returns `applied`**. It never
rejects an over-settlement — the cash physically arrived, so the ACL books the remainder
(`amount − applied`) as an **on-account party credit** (already on the A/R control from the settlement
post). This keeps the GL A/R and the billing subledger in agreement even when two payments race the
same invoice (council 2026-07-05). Payment holds no normal Cargo dependency on billing.

**BR-8 (modes seed via overlay).** ModeOfPayment Indonesia defaults (QRIS, virtual account, e-wallet)
are seeded by the `id` overlay layer, not hard-coded into the base `ModeType` enum.

**BR-9 (refund / reversal — KEEP, ADR-001 §5).** `reverse_payment` fully reverses a posted payment:
posts the sign-flipped `posting_type="reversal"` mirror journal (linked via `reverses_post_id`, so
accounting treats it as a distinct-but-idempotent post), flips status `posted → cancelled`, and emits
`PaymentCancelled` carrying the allocations. An ACL routes each → billing's `reverse_settlement`, which
**restores** `outstanding_amount` (bounded by `grand_total`) and rewinds schedules last-installment
first. All-or-nothing; exactly-once (dedup + the `posted→cancelled` gate). → `not_reversible` if the
payment was never posted.

## Events
`PaymentSettled`, `PaymentReceivedOnAccount`, `PaymentCancelled` (reversal). (Consumed downstream:
`PaymentSettled` → billing `apply_settlement`; `PaymentCancelled` → billing `reverse_settlement`; both
by banking to expect/undo clearing.)

## Deferred (with reason)
Bank clearing/reconciliation (backbone-banking), gateways/requests/orders (basic gateway hookup — key
`reference_no` manually meanwhile), POS tender, advance-adjustment automation, multi-currency/FX,
withholding-at-payment, **partial** reversal.
