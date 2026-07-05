# Extension Guide — backbone-payment

> Public contract per `docs/erp/extension-contract.md`. Stable path:
> `backbone_payment::application::service::*` (the generated `exports/` tree is unwired scaffolding).

## Public surface
**A. Domain events** (`payment_events`, the 3-variant `PaymentEvent`): `PaymentSettled` {payment_id,
company_id, journal_id, post_id, payment_type, allocations[], paid_amount}, `PaymentReceivedOnAccount`
{payment_id, company_id, party_id, unallocated_amount}, `PaymentCancelled` {payment_id, company_id,
journal_id, post_id, payment_type, allocations[], paid_amount} — the reversal event (mirror of
`PaymentSettled`), routed → billing `reverse_settlement`.

**B. The GL-posting port** (`payment_gl`) — `AccountingPostEnvelope` is the serialized wire contract
into `backbone-accounting`; a consumer implements `GlPostSink` (async `post(&envelope)`) over
accounting's `PostingService`. Payment never imports accounting in the shipped library.

## How a consumer extends
1. **Post to the GL** — implement `GlPostSink`, mapping `AccountingPostEnvelope` → accounting's
   `PostingRequest`; pass it to `post_payment`. (Reference ACL: `tests/settlement_seam.rs`.)
2. **Wire the settlement seam** — route `PaymentSettled{allocations}` → billing's `apply_settlement`,
   drawing each invoice's `outstanding_amount` + schedules down. This is the cash-loop close.
   **And the reverse seam** — route `PaymentCancelled{allocations}` → billing's `reverse_settlement`,
   restoring outstanding when a payment is reversed (`reverse_payment`). Both halves ship together.
3. **React to on-account credits** — subscribe to `PaymentReceivedOnAccount` to drive later
   reconciliation of unlinked payments to invoices.
4. **Add a gateway / bank adapter** — capture an external callback → `create_payment` → `post_payment`;
   the Bank account is a clearing account that backbone-banking later reconciles against a statement.
5. Keep logic in `user_owned`/`*_custom.rs` — survives regen (proven by
   `scripts/settlement_seam_roundtrip.sh`).

## Bounded-context split (important)
Payment owns **"you cannot allocate more money than moved"** (`Σalloc ≤ paid`, at create).
Billing owns **"you cannot knock off more than is owed"** (`amount ≤ outstanding`, in
`apply_settlement`). Neither reaches into the other's tables from its shipped library.

## Not a contract
Generated CRUD events; internal repositories/services; `// <<< CUSTOM` blocks (own edits only).

## Deferred surfaces
Bank clearing (backbone-banking), gateways/requests/orders, POS tender, refunds/reversal posts,
multi-currency — additive when built.
