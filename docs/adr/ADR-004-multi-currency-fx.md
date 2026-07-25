# ADR-004: Multi-currency settlement + FX revaluation (design — not yet implemented)

**Status**: Proposed (design locked; implementation deferred to a focused session)
**Deciders**: Farid (owner), council 2026-07-25 (maturity review)
**Related**: ADR-001 (settlement boundary — IDR-only deferral), ADR-003 (PPh — also IDR-only)

## Context

ADR-001 locked payment to IDR-only; the council confirmed FX is the #2 domain gap (after PPh). Import/
export businesses need non-IDR payments (USD, EUR, CNY) — both the settlement post AND the outstanding
reconciliation must handle foreign currency, plus period-end FX revaluation (mark-to-market of open
foreign-currency A/R and A/P balances).

## Decision (design)

1. **Dual-currency on the entry.** `PaymentEntry` gains `currency` (already present — just remove the
   IDR-only guard) + `exchange_rate: decimal` (amount of IDR per 1 unit of foreign currency). The
   settlement post expresses lines in BOTH the transaction currency AND the base (IDR) equivalent:

   ```
   Dr Bank 10,000 USD @ 15,800    = Dr Bank 158,000,000 IDR
   Cr A/R 10,000 USD @ 15,800    = Cr A/R 158,000,000 IDR
   ```

   The envelope carries the IDR amounts (the base-currency post to the GL); the entry stores the
   foreign amounts + the rate for audit/traceability.

2. **Exchange-rate source.** A new `backbone-fx` module (or a rate table in backbone-accounting) stores
   daily rates. The payment service fetches the rate at post time. Historical rates are immutable.

3. **FX revaluation (period-end).** Open foreign-currency balances are revalued at the period-end rate:
   `unrealized_fx_gain_or_loss = balance × (period_end_rate − original_rate)`. This is a separate
   accounting post (Dr/Cr the foreign-currency account · Cr/Dr FX Gain/Loss). It lives in
   backbone-accounting (a period-close job), NOT in payment — payment only settles; accounting owns
   the revaluation.

4. **PPh interaction.** PPh (ADR-003) is IDR-only for now. PPh on foreign-currency payments (e.g., PPh 26
   on USD payments to non-residents) is deferred with FX — it needs the rate to convert the withheld
   amount to IDR.

5. **Seam unchanged.** The `PaymentSettled` event carries `paid_amount` in the transaction currency +
   the rate. Billing's `apply_settlement` must handle foreign-currency knock-offs (the invoice's
   currency vs the payment's currency).

## Implementation steps (a focused session)

1. Schema: add `exchange_rate` field to `payment_entry.model.yaml`. Remove the `@default("IDR")`
   constraint on `currency` (allow any ISO code).
2. Remove the IDR-only guard in `build_settlement_post`.
3. Store the rate at post time (from backbone-fx or a passed-in rate).
4. Tests: a golden case for a USD receive @ 15,800; an integrity probe for a non-IDR post that
   succeeds (the current IP-2 is the inverse — it refuses non-IDR; flip it to accept).
5. Billing seam: `apply_settlement` must handle cross-currency (the invoice may be in IDR while the
   payment is in USD — the allocation converts via the rate).

## Parking lot

The FX revaluation post (period-end), the exchange-rate administration UI, and the PPh-on-foreign-
currency interaction (PPh 26 on USD to non-residents).
