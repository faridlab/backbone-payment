# PRD — backbone-payment

> Tier 2 · Financials · Indonesia-first ERP. Status: built. Date: 2026-07-05.

## Problem & intent
Invoices are only half the story — money has to actually move and be knocked off what is owed.
`backbone-payment` owns **settlement**: it records payment entries (receive from a customer, pay a
supplier), allocates each payment across one-or-many invoices, posts the cash movement to the ledger,
and draws the invoices' balances down in billing — **closing the cash loop** (order-to-cash-to-bank,
procure-to-pay-to-bank). It is the 4th GL producer and the module that finally exercises the
`outstanding_amount` + payment-schedule hooks billing deliberately left inert.

## Goals
- Own **PaymentEntry** (receive/pay) + **PaymentAllocation** (one payment → many invoices) +
  **ModeOfPayment** (cash / transfer / card / e-wallet / virtual account / QRIS).
- Compute money **server-side**; guarded surface (no generic mutation of allocations).
- Post **one balanced settlement `AccountingPost`** per payment: receive `Dr Bank · Cr A/R
  [customer]`; pay `Dr A/P [supplier] · Cr Bank`. Idempotent, reject-recoverable.
- Drive the **settlement seam** payment→billing (`PaymentSettled` → `apply_settlement`): draw down
  each invoice's `outstanding_amount` + payment schedules, flip status → `partially_paid`/`paid`.
- Keep an **on-account** remainder (unallocated) as a first-class output (`PaymentReceivedOnAccount`).

## Non-goals (this phase / deferred)
Bank statement import + clearing/reconciliation (backbone-banking), payment gateways / requests /
orders (adapter surface), POS tender capture (backbone-pos), advance/prepayment adjustment
automation, refunds/reversal posts, multi-currency settlement + FX revaluation, withholding at
payment time (tax is computed upstream at invoice time by backbone-tax).

## Personas
AR clerk (receives + reconciles customer payments), AP clerk (disburses to suppliers), Integrating
engineer (subscribes to payment events, wires the settlement seam + a gateway/bank adapter).

## Success criteria
- Settlement math + allocation bounds + state machine locked by a numeric oracle (5 golden cases) +
  integrity probes (3, incl. a concurrent double-post that emits the seam event exactly once).
- The cash-loop seam proven end-to-end against the real ledger (SSEAM-1, partial→full + fill-in-order
  schedule drawdown) + survives regen of both modules (§5, `scripts/settlement_seam_roundtrip.sh`).
- Indonesia-ready: ModeOfPayment defaults (QRIS, virtual account, e-wallet) seed via the `id` overlay,
  not the base enum; remove the overlay → generic settlement still runs.
