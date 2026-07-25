# ADR-003: Withholding tax (PPh) — the third settlement-post line

**Status**: Accepted — Applied 2026-07-25
**Deciders**: Farid (owner), council 2026-07-25 (maturity review)
**Related**: ADR-001 (settlement boundary), ADR-002 (settlement seam), docs/erp/gl-posting-contract.md

## Context

The council maturity review (2026-07-25) identified PPh (Pajak Penghasilan — Indonesian income tax
withholding) as the #1 domain gap for production Indonesian ERP use. Indonesian B2B payments routinely
require the payer to withhold a portion (PPh 22: imports/government 1.5–7.5%; PPh 23: services/rent
2/10/15%; PPh 26: non-resident 20%) and remit it to the tax authority (DJP). Payments without
withholding are non-compliant.

Payment's current settlement post is 2-line (receive: `Dr Bank · Cr A/R`; pay: `Dr A/P · Cr Bank`),
both at the GROSS amount. PPh introduces a THIRD line — the withheld portion — so the bank movement
is NET (gross − withheld) while the A/R or A/P is cleared at GROSS.

## Decision

1. **Three-line settlement post when `withholding_amount > 0`.** The schema gains:
   - `withholding_amount: decimal @default(0) @non_negative` — the withheld amount (0 = no withholding).
   - `withholding_account_id: uuid @exclude_from_foreign_key_check` — the PPh Payable (pay) or PPh
     Receivable (receive) GL account. Required when `withholding_amount > 0`.
   - `withholding_tax_type: WithholdingTaxType` — enum: `none` (default) | `pph_22` | `pph_23` | `pph_26`.

2. **Posting shapes:**

   **Pay a supplier WITH PPh 23 (e.g., pay 1,000,000, withhold 20% = 200,000, net 800,000):**
   ```
   Dr A/P [supplier] 1,000,000     (full gross — liability cleared)
   Cr Bank 800,000                 (net paid)
   Cr PPh 23 Payable 200,000       (withheld — owed to DJP)
   ```

   **Receive from a customer WITH PPh 22 (e.g., invoice 1,000,000, customer withholds 10% = 100,000, net 900,000):**
   ```
   Dr Bank 900,000                 (net received)
   Dr PPh 22 Receivable 100,000    (withheld by customer — we claim credit)
   Cr A/R [customer] 1,000,000     (full gross — receivable cleared)
   ```

   When `withholding_amount = 0` the post stays 2-line (the current shape, unchanged) — PPh is
   backward-compatible.

3. **The money invariant extends:** `paid_amount = bank_amount + withholding_amount`. The `paid_amount`
   field stays GROSS (it's what clears the A/R or A/P). The bank movement is `paid_amount − withholding_amount`.
   The current `Σ allocations ≤ paid_amount` bound is unchanged (it bounds against the gross).

4. **The settlement envelope (`AccountingPostEnvelope`) gains a third line** — the gateway's fee
   companion post is unaffected (it's a separate envelope). The seam events (`PaymentSettled`) carry
   `paid_amount` at gross; the withholding details stay internal (billing sees only the gross knock-off).

5. **IDR-only.** PPh rates and types are Indonesia-specific. Multi-currency PPh (e.g., PPh 26 on USD
   payments to non-residents) is deferred with multi-currency settlement.

## Consequences

- The settlement post builder (`build_settlement_post`) gains a conditional third line. The 2-line
  path (no withholding) is unchanged → existing tests + behavior are preserved.
- `NewPayment` gains `withholding_amount` + `withholding_account_id` + `withholding_tax_type` fields
  (all optional/ defaulted → backward-compatible).
- The settlement math (`Σ debit = Σ credit` balance check) naturally extends to 3 lines.
- **Implementation steps (next session):**
  1. Schema YAML: add the 3 fields + the `WithholdingTaxType` enum to `payment_entry.model.yaml`.
  2. `metaphor make entity` + `metaphor migration generate add_withholding_fields` → entity/DTO/handler
     regenerated; migration adds the columns (nullable/defaulted, backward-compatible).
  3. `payment_write_service`: `build_settlement_post` adds the withholding line when `> 0`; `NewPayment`
     gains the fields; `create_payment` validates `withholding_amount ≤ paid_amount`.
  4. Test: a golden case (GGC-7) for the 3-line post + an integrity probe (IP-5) for the DB-level
     reverse of a withholding payment (reverse must also reverse the PPh line).

## Parking lot

PPh remittance (filing + paying DJP), PPh 21 (employee payroll withholding — owned by backbone-payroll),
PPh on multi-currency payments (deferred with FX), and the PPh compliance reporting (SPT — e-filing).
