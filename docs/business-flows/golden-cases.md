# Payment — Golden Cases (the numeric oracle)

Mirrors `tests/payment_golden_cases.rs`, `tests/integrity_probes.rs`, and the cross-module settlement
seam in `tests/settlement_seam.rs`. Money is exact IDR (2dp, half-up).

## Write path + posting (`tests/payment_golden_cases.rs`)

| Case | Input | Expected |
|------|-------|----------|
| **PGC-1** | receive 1,000,000, allocate 600,000 to one invoice | allocated `600,000`, unallocated `400,000`; post `Dr Bank 1,000,000 · Cr A/R 1,000,000 [customer]` (balanced); `PaymentSettled` (1 alloc of 600k, paid 1,000,000) + `PaymentReceivedOnAccount` 400,000. |
| **PGC-2** | pay a supplier 500,000 | post `Dr A/P 500,000 [supplier] · Cr Bank 500,000`. |
| **PGC-3** | post the same payment twice | second post `idempotent_reuse=true`, same journal, sink hit once, `PaymentSettled` emitted once. |
| **PGC-4** | over-allocate / non-positive / duplicate number | `over_allocated` / `non_positive_amount` / `duplicate_number`. |
| **PGC-5** | receive fully allocated (unallocated = 0) | no `PaymentReceivedOnAccount` event. |

## Integrity probes (`tests/integrity_probes.rs`)

| Case | Input | Expected |
|------|-------|----------|
| **IP-1** | GL sink rejects, then a good sink retries | first: `posting_state=failed`, `status=draft`, no journal (recoverable); retry → `posted`. |
| **IP-2** | post a non-IDR (USD) payment | `unsupported_currency`; no mis-valued post. |
| **IP-3** | two `tokio::join!`ed `post_payment` calls racing on a barrier | `PaymentSettled` fires **exactly once** — the pending→posted gate stops a double `apply_settlement` that would draw an invoice's outstanding down twice (2 without the gate, 1 with). |

## Settlement seam — billing ↔ payment ↔ accounting (`tests/settlement_seam.rs` + `scripts/settlement_seam_roundtrip.sh`)

| Case | Input | Expected |
|------|-------|----------|
| **SSEAM-1** | billing posts a Sales Invoice (1,000,000; installments 600k+400k); payment A receives 600,000 → post → `PaymentSettled` → `apply_settlement`; payment B receives 400,000 → settle | both cash journals balance; after A: invoice `partially_paid`, outstanding `400,000`, installment 1 `paid` / installment 2 `unpaid` (fill-in-order); after B: `paid`, outstanding `0`, installment 2 `paid`. Zero normal Cargo edge. |
| **SSEAM-2** (council 2026-07-05) | two 600,000 receipts each allocate 600,000 to the SAME 1,000,000 invoice; both post, both `apply_settlement` | the split invariant COMPOSES: second apply CLAMPS to the remaining `400,000` (returns `applied=400,000`) → invoice `paid`; A/R credited `1,200,000` − invoice debit `1,000,000` = `200,000` **on-account party credit** (retrievable, not stranded). GL ties to the subledger. Fails under reject semantics (cash vanishes). |
| **SSEAM-3** (completeness council 2026-07-05) | post a receive settling a 1,000,000 invoice to `paid`, then `reverse_payment` → `PaymentCancelled` → `reverse_settlement`; then reverse again | reversal `posting_type="reversal"` journal posts (sign-flipped); payment → `cancelled`; invoice restored to `1,000,000` / `submitted`; customer A/R net owes `1,000,000` again. Re-reverse: **one** reversal post (accounting dedups), `PaymentCancelled` emitted **once** (gate), outstanding **not** double-restored. Fails when `reverse_payment` is absent (invoice stuck `paid`, cash stranded). |
| **§5 round-trip** | regen BOTH payment + billing, re-run | all seam ACL/consumer files byte-identical; SSEAM-1/2/3 still green — survives regen of both modules. |

## Conventions
- One balanced settlement `AccountingPost` per payment; the receivable/payable line carries the party.
- Allocation is bounded at create (`Σalloc ≤ paid`); billing bounds the drawdown (`amount ≤ outstanding`).
- Posting is idempotent (`source_id = payment id`) + repost-recoverable; the seam event is
  transition-gated (exactly-once under concurrency); IDR-only for now.
- Payment records settlement + on-account remainder; **bank clearing is backbone-banking's**.
