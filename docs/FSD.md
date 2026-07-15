# FSD — backbone-payment

> Functional Spec. Tier 2 · Financials. Date: 2026-07-05.

## Entities (schema/models/*.model.yaml — SSoT)
PaymentEntry(+allocations) · ModeOfPayment. PaymentEntry carries `paid_amount` / `allocated_amount` /
`unallocated_amount`, a logical `bank_account_id` (Bank/Cash) + `party_account_id` (A/R or A/P
control), `payment_type`, `party_type`/`party_id`, `posting_state`, and `journal_id`/
`accounting_post_id` (reconciled from the ack). PaymentAllocation carries `invoice_ref` +
`invoice_kind` (SettlementKind: sales/purchase) + `allocated_amount`. Cross-module ids are logical FKs
(`@exclude_from_foreign_key_check`): party→party, account→accounting, company/branch→organization,
`invoice_ref`→billing.

## Services (application/service — hand-authored, user_owned)
- `PaymentWriteService` — `create_payment` (server-side money + the `Σalloc ≤ paid` allocation bound);
  `build_settlement_post` (assemble ONE balanced `AccountingPostEnvelope`); `post_payment`
  (short-circuit if posted → emit through a `GlPostSink` → **gate reconcile + publish on the
  pending→posted UPDATE** → publish `PaymentSettled` + `PaymentReceivedOnAccount` for any remainder);
  `build_reversal_post` + `reverse_payment` (sign-flipped `posting_type="reversal"` mirror → flip
  `posted→cancelled` → publish `PaymentCancelled`; the refund path).
- `payment_gl` — the outbound GL port: `GlPostLine`, `AccountingPostEnvelope`, `GlPostAck`,
  `GlPostRejected`, `GlPostSink` (async trait). The wire contract; zero normal edge.
- `payment_events` — `PaymentEvent` {`PaymentSettled` (carries `allocations`), `PaymentReceivedOnAccount`,
  `PaymentCancelled`} + `PaymentEventSink` + `LoggingSink`.

## HTTP surface (presentation/http/guarded_routes.rs)
`create_guarded_payment_routes(&PaymentModule, pool, TenantVerifier)` — read documents + validated
`POST /payment-entries` (with allocations). No generic mutation. The write surface is tenant-guarded:
`company_id`/`branch_id` are derived from the signed Bearer token (`backbone_auth::tenant`), never
from the request body. Posting needs a `GlPostSink` composition layer, so it is service/job-driven,
not an HTTP route.

## State machines
- Payment (`PaymentStatus`): `draft → posted` on a successful post; `cancelled` (reversal deferred).
- Posting (`GlPostingState`): `pending → posted` (ack) / `failed` (rejected, retryable).

## Integration seams
- **Settlement seam (proven, marquee):** posted payment → `PaymentSettled{allocations}` → ACL →
  billing `apply_settlement` → invoice `outstanding_amount` + schedules drawn down (fill-in-order),
  status → `partially_paid`/`paid`. The settlement post routes through a `GlPostSink` ACL into
  accounting's `PostingService` (real ledger). Zero normal Cargo edge. ADR-002,
  `tests/settlement_seam.rs`, `scripts/settlement_seam_roundtrip.sh`.
- **Inbound (future):** `PaymentRequest` → gateway callback → PaymentEntry; banking imports the
  statement and clears the Bank account (backbone-banking); POS tender capture (backbone-pos).

## Test oracle
`payment_golden_cases` (5: receive + pay math/post, idempotency, validation gates incl. over-allocation,
fully-allocated-no-on-account), `integrity_probes` (3: rejected-post recovery, non-IDR refusal,
**concurrent double-post emits the seam event exactly once**), `settlement_seam` (3: SSEAM-1 real ledger
partial→full + fill-in-order + §5; **SSEAM-2 two payments racing the same invoice reconcile via
CLAMP-and-on-account**; **SSEAM-3 refund/reversal restores the invoice + is idempotent** — councils
2026-07-05). **11 tests** (of the hand-authored suite).
