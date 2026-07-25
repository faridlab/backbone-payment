---
date: 2026-07-25
repo_type: module
unit: backbone-payment
focus: maturity
roster: chair, skeptic, steelman, yagni-business, ddd-bounded-context, contract-seat, domain-expert (financials)
---

# Council — module:backbone-payment — focus: maturity

## Best call
The payment module is **production-mature for IDR-only domestic settlement**. Ship it with the outbox
enabled (the composition layer already does this). Plan PPh (withholding) as the next feature track —
it's the #1 compliance blocker for Indonesian production.

- Residual negative value: a crash between the posted-transition and `emit_settled` can lose the
  `PaymentSettled` event IF the outbox is not enabled — silent GL/subledger divergence. The composition
  layer enables it, but the module ships without it as default.
- Reversibility: easy (the outbox is already wired via `with_outbox_schema`; enabling it is one builder
  call in the composition).
- What would flip this: evidence that the outbox path has a durability bug (the relay_rls test proves
  the fence + bypass; the settlement_bus_seam test proves the staging — so this is low-risk).

## Disagreement map
- **Crash-safety opt-in vs. correct library/service separation** — Skeptic says the outbox must be
  default; Steelman + DDD-seat say the module is a library (opt-in is correct; the service enables it).
  Crux: is the composition layer's outbox enablement documented as a HARD requirement, or just a
  convention? Today it's convention — the skeleton does it, but nothing ENFORCES it.
- **PPh (withholding) as blocker vs. correctly deferred** — Domain-expert says PPh is a compliance
  blocker for Indonesian ERP; YAGNI-business says it's not blocking for a domestic IDR MVP this month.
  Crux: is the target a framework demonstration or a production Indonesian ERP? For the former, deferral
  is correct; for the latter, PPh is the #1 gap.

## Recommendations (ranked by leverage)
| # | Move | Leverage | Residual negative | Reversibility | Evidence to flip |
|---|------|----------|-------------------|---------------|------------------|
| 1 | Document the outbox as a HARD go-live requirement (not opt-in convention) | high | None — pure documentation | easy | N/A |
| 2 | Add `correlation_id`/`causation_id` to `PaymentSettled`/`PaymentCancelled` for distributed tracing | med | Tracing gap in 4-hop settlement | easy | If the team adopts a tracing standard that doesn't need these |
| 3 | Plan PPh (withholding) as the next domain feature track | high (for ID production) | Without PPh, the module can't handle compliant Indonesian B2B payments | costly (domain modeling) | If the target market is not Indonesia, PPh is irrelevant |
| 4 | Add a DB-gated test for `reverse_payment` (the reversal path is proven at the unit level but not end-to-end against PG) | med | Reversal untested at the DB level | easy | If the settlement_seam test is extended to cover reversal |

## Maturity scorecard
| Seat | Axis | Score (1–5) | One sentence why |
|------|------|-------------|------------------|
| DDD-bounded-context | bounded-context cleanliness | 4 | Clean edges, correct separation; the hand-written repo SQL is the right tradeoff but adds maintenance cost |
| Contract-seat | contract stability | 4 | Well-shaped envelopes + events; the correlation_id gap is minor but real for distributed tracing |
| Domain-expert (financials) | domain completeness | 3 | IDR settlement + reconciliation + reversal complete; PPh + FX are real gaps for production |
| Skeptic | operational readiness | 3 | Outbox is opt-in not default; the crash-safety gap is documented but not enforced |
| YAGNI-business | leverage | 4 | Real pain removed today (settlement, billing seam, reversal, gateway); deferred items are correctly scoped |

## Parking lot
- **backbone-banking** (bank statement clearing/reconciliation) — raised by domain-expert, scope: separate module
- **POS tender integration** — raised by domain-expert, scope: backbone-pos
- **Multi-currency/FX revaluation** — raised by domain-expert, scope: backbone-payment (deferred per ADR-001)
- **Real event bus + production consumer ownership** — raised by skeptic, scope: composition layer
- **Partial reversal / re-settlement-after-partial-reverse** — raised by domain-expert, scope: backbone-payment (deferred per ADR-001)
