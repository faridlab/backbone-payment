# ADR-005: The backbone-banking clearing seam (design — not yet built)

**Status**: Proposed (design locked; module not yet created)
**Deciders**: Farid (owner), council 2026-07-25
**Related**: ADR-001 (settlement boundary — bank clearing is backbone-banking's job), ADR-002
(settlement seam), docs/erp/financials.md

## Context

Payment owns the INTENT + ALLOCATION of money (a receive posts `Dr Bank · Cr A/R`). But it does NOT
own the bank STATEMENT — the truth of what actually arrived/departed at the bank. ADR-001 explicitly
delegates bank clearing/reconciliation to a separate module (`backbone-banking`). This ADR records the
seam design so the module's scope is clear when it's built.

## Decision (design)

1. **backbone-banking owns the bank STATEMENT.** Its entities:
   - `BankAccount` — a company's bank account (account number, currency, GL link, statement source).
   - `BankStatement` — a statement period (from/to dates, opening/closing balance).
   - `BankTransaction` — one line on a statement (date, amount, direction, counterparty, reference,
     raw description). Imported from CSV/MT940/API (Midtrans settlement, BCA snapshot, etc.).
   - `BankReconciliation` — the matching of bank transactions to payment entries (and other GL
     movements — payroll, tax payments, transfers).

2. **The clearing seam: `PaymentSettled` → banking marks the payment as CLEARED.**
   When payment settles (Dr Bank · Cr A/R), banking's reconciliation matches that bank movement to a
   `BankTransaction` on the statement. The match is a `BankReconciliation` record linking the payment
   to the bank transaction. Until matched, the payment is SETTLED but UNRECONCILED (the GL says money
   moved; the bank hasn't confirmed it). After matched, it's RECONCILED.

3. **Payment's status gains `reconciled`.** Payment currently has `draft → submitted → posted →
   cancelled`. Banking's match transitions `posted → reconciled` (a new status, or a separate flag
   `reconciled_at: datetime?`). This is additive — the existing lifecycle is unchanged.

4. **Auto-matching rules** (in banking, not payment):
   - Match on `reference_no` (the payment's reference matches the bank transaction's description).
   - Match on `amount + date` (±N days tolerance).
   - Match on `mode_of_payment` (e.g., Midtrans settlements batch to one bank transaction).

5. **Zero Cargo edges** — same seam shape as payment↔billing. Banking listens to `PaymentSettled`
   (carrying `bank_account_id`, `paid_amount`, `reference_no`); it does NOT import payment. The match
   is an event-driven consumer + a reconciliation UI.

## What payment does NOT do (banking's job)

- Import/parse bank statements (CSV, MT940, BCA API, Midtrans settlement reports).
- Auto-match bank transactions to payments.
- Surface unreconciled payments (the reconciliation UI).
- Handle bank fees (a separate `BankFee` posting — `Dr Bank Fee Expense · Cr Bank`).
- Handle bank transfers (between own accounts — a `Dr Bank A · Cr Bank B` post).

## Implementation steps (a focused session)

1. `metaphor module create backbone-banking` — scaffold the module.
2. Schema: `BankAccount`, `BankStatement`, `BankTransaction`, `BankReconciliation` entities.
3. Import adapter: CSV (the simplest — BCA/Permata export CSV; MT940 later).
4. Auto-matching engine: match on reference/amount/date.
5. Seam: subscribe to `PaymentSettled`; create `BankReconciliation` on match.
6. Reconciliation report: reconciled vs unreconciled per bank account per period.

## Parking lot

Bank fee handling, bank transfers, MT940 import, Midtrans settlement report import, the reconciliation
UI, and the `posted → reconciled` status transition in payment (additive — deferred until banking
matches its first payment).
