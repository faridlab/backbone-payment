//! EPD — the early-pay-discount settlement golden: the three-legged GL, the materialized decision,
//! the clamp, the v1.1 event fields, and the mirrored reversal. The discount resolver is a fixed
//! stub (payment treats it as "the invoice module said so"); the MATH is what this file pins:
//! receive 1,000,000 @ 2% ⇒ Dr Bank 980,000 · Dr Discount 20,000 · Cr A/R 1,000,000 — balanced by
//! construction, allocations knocking off GROSS. Requires DATABASE_URL (:5433/backbone_payment).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;

use std::future::Future;

/// Drive a settlement/reversal verb inside the org request scope the composition layer always
/// binds (ADR-0029) — those verbs read the legacy company twin off the ambient scope.
async fn in_org_scope<R>(pool: &PgPool, f: impl Future<Output = R>) -> R {
    backbone_orm::org_scope::with_org_request_scope(
        pool,
        backbone_orm::org_scope::OrgScope::for_company_unit(Uuid::new_v4()),
        f,
    )
    .await
    .expect("bind org request scope")
}

use backbone_payment::application::service::payment_discount::{
    discount_for, EarlyPayDecision, SettlementDiscountPort,
};
use backbone_payment::application::service::payment_events::{PaymentEvent, PaymentEventSink};
use backbone_payment::application::service::payment_gl::{
    AccountingPostEnvelope, GlPostAck, GlPostLine, GlPostRejected, GlPostSink,
};
use backbone_payment::application::service::payment_lifecycle::BankReconcilablePort;
use backbone_payment::application::service::payment_write_service::{
    NewAllocation, NewPayment, PaymentError, PaymentWriteService,
};

/// Tests inject the reconcilability read — the real probe reads accounting, which these fixtures do
/// not populate.
struct AlwaysReconcilable;
#[async_trait::async_trait]
impl BankReconcilablePort for AlwaysReconcilable {
    async fn bank_reconcilable(
        &self,
        _pool: &sqlx::PgPool,
        _company_id: uuid::Uuid,
        _account_id: uuid::Uuid,
    ) -> Result<bool, PaymentError> {
        Ok(true)
    }
}

/// The fixed resolver: per-invoice decisions from a map, absent ⇒ no discount. Counts calls so the
/// golden can pin that a resolved decision is STAMPED and never re-asked for a committed post.
struct FixedDiscount {
    decisions: HashMap<Uuid, EarlyPayDecision>,
    calls: Arc<Mutex<usize>>,
}
#[async_trait::async_trait]
impl SettlementDiscountPort for FixedDiscount {
    async fn resolve(
        &self,
        _company_id: Uuid,
        invoice_ref: Uuid,
        _invoice_kind: &str,
        _on_date: chrono::NaiveDate,
    ) -> Result<Option<EarlyPayDecision>, PaymentError> {
        *self.calls.lock().unwrap() += 1;
        Ok(self.decisions.get(&invoice_ref).cloned())
    }
}

fn d(s: &str) -> Decimal {
    Decimal::from_str_exact(s).unwrap()
}
fn uq(p: &str) -> String {
    format!("{p}-{}", &Uuid::new_v4().simple().to_string()[..8])
}
async fn pool() -> PgPool {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://postgres:postgres@localhost:5433/backbone_payment".to_string()
    });
    PgPool::connect(&url).await.expect("connect DB")
}

#[derive(Default, Clone)]
struct Recorder {
    events: Arc<Mutex<Vec<PaymentEvent>>>,
}
impl PaymentEventSink for Recorder {
    fn publish(&self, e: PaymentEvent) {
        self.events.lock().unwrap().push(e);
    }
}

/// A GL sink that REFUSES everything — for the deterministic-replay case.
struct RejectingGl;
#[async_trait::async_trait]
impl GlPostSink for RejectingGl {
    async fn post(&self, _e: &AccountingPostEnvelope) -> Result<GlPostAck, GlPostRejected> {
        Err(GlPostRejected {
            code: "probe_refused".into(),
            message: "golden replay probe".into(),
        })
    }
}

#[derive(Default, Clone)]
struct OkGl;
#[async_trait::async_trait]
impl GlPostSink for OkGl {
    async fn post(&self, env: &AccountingPostEnvelope) -> Result<GlPostAck, GlPostRejected> {
        assert!(
            env.is_balanced(),
            "payment emitted an UNBALANCED post: {env:?}"
        );
        Ok(GlPostAck {
            post_id: Uuid::new_v4(),
            journal_id: Uuid::new_v4(),
            idempotent_reuse: false,
        })
    }
}

fn svc(
    pool: &PgPool,
    rec: Recorder,
    decisions: HashMap<Uuid, EarlyPayDecision>,
    calls: Arc<Mutex<usize>>,
) -> PaymentWriteService {
    PaymentWriteService::with_sink(pool.clone(), Arc::new(rec))
        .with_reconcilable_port(Arc::new(AlwaysReconcilable))
        .with_discount_port(Arc::new(FixedDiscount { decisions, calls }))
}

fn receive(
    _company: Uuid,
    bank: Uuid,
    ar: Uuid,
    customer: Uuid,
    paid: &str,
    allocs: Vec<NewAllocation>,
) -> NewPayment {
    NewPayment {
        payment_number: uq("PE"),
        branch_id: None,
        payment_type: "receive".into(),
        party_type: Some("customer".into()),
        party_id: Some(customer),
        posting_date: chrono::NaiveDate::from_ymd_opt(2026, 8, 21).unwrap(),
        currency: None,
        mode_of_payment_id: None,
        method: None,
        provider_txn_id: None,
        bank_account_id: bank,
        party_account_id: ar,
        paid_amount: d(paid),
        reference_no: None,
        allocations: allocs,
        withholding_amount: Decimal::ZERO,
        withholding_account_id: None,
        withholding_tax_type: "none".into(),
    }
}

fn leg<'a>(env: &'a AccountingPostEnvelope, account: Uuid) -> &'a GlPostLine {
    env.lines
        .iter()
        .find(|l| l.account_id == account)
        .unwrap_or_else(|| panic!("no leg on {account} in {env:?}"))
}

async fn stamp_of(pool: &PgPool, payment_id: Uuid, invoice: Uuid) -> (Decimal, Option<Uuid>) {
    sqlx::query_as(
        "SELECT discount_amount, discount_account_id FROM payment.payment_allocations WHERE payment_id=$1 AND invoice_ref=$2")
        .bind(payment_id).bind(invoice).fetch_one(pool).await.unwrap()
}

// EPD-0: the pure clamp — discount = money(allocated × percent/100), never above the allocated
// amount, zero on non-positive inputs.
#[test]
fn discount_for_clamps() {
    let acct = Uuid::new_v4();
    let dec = |p: &str| EarlyPayDecision {
        percent: d(p),
        outstanding_basis: d("999999999"),
        account_id: acct,
    };
    assert_eq!(discount_for(d("1000000"), &dec("2")), d("20000.00"));
    assert_eq!(discount_for(d("100000"), &dec("2")), d("2000.00"));
    // clamp to the allocated amount…
    assert_eq!(discount_for(d("100000"), &dec("150")), d("100000.00"));
    // …and zero for non-positive sides.
    assert_eq!(discount_for(d("0"), &dec("2")), Decimal::ZERO);
    assert_eq!(discount_for(d("100000"), &dec("0")), Decimal::ZERO);
    assert_eq!(discount_for(d("100000"), &dec("-5")), Decimal::ZERO);
}

// EPD-1: THE golden — receive 1,000,000 fully allocated at 2%:
//   Dr Bank 980,000 · Dr Discount 20,000 · Cr A/R 1,000,000 (gross, party-tagged)
// balanced by construction; the stamp lands on the allocation row; the v1.1 event carries the
// landing status + the discount.
#[tokio::test]
async fn three_leg_receive_discount_golden() {
    let pool = pool().await;
    let (company, bank, ar, customer, discount_acct, inv) = (
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    let rec = Recorder::default();
    let calls = Arc::new(Mutex::new(0));
    let w = svc(
        &pool,
        rec.clone(),
        HashMap::from([(
            inv,
            EarlyPayDecision {
                percent: d("2"),
                outstanding_basis: d("999999999"),
                account_id: discount_acct,
            },
        )]),
        calls.clone(),
    );

    let id = w
        .create_payment(receive(
            company,
            bank,
            ar,
            customer,
            "1000000",
            vec![NewAllocation {
                invoice_ref: inv,
                invoice_kind: "sales".into(),
                amount: d("1000000"),
            }],
        ))
        .await
        .unwrap();

    // Build the envelope the post will send (pure read of the committed decision path).
    in_org_scope(&pool, w.post_payment(id, &OkGl)).await.unwrap();
    let env = in_org_scope(&pool, w.build_settlement_post(id)).await.unwrap();
    assert_eq!(
        env.totals(),
        (d("1000000.00"), d("1000000.00")),
        "3 legs, balanced"
    );
    assert_eq!(
        leg(&env, bank).debit,
        d("980000.00"),
        "bank moves NET of the discount"
    );
    assert_eq!(
        leg(&env, discount_acct).debit,
        d("20000.00"),
        "the discount leg on the term's account"
    );
    let ar_line = leg(&env, ar);
    assert_eq!(
        ar_line.credit,
        d("1000000.00"),
        "A/R settles GROSS — the knock-off is unchanged"
    );
    assert_eq!(ar_line.party_type.as_deref(), Some("customer"));

    // The stamp: materialized on the allocation row, inside the posted tx.
    assert_eq!(
        stamp_of(&pool, id, inv).await,
        (d("20000.00"), Some(discount_acct))
    );

    // v1.1 fields on the emitted event: status + per-allocation discount.
    let settled = rec
        .events
        .lock()
        .unwrap()
        .iter()
        .find_map(|e| match e {
            PaymentEvent::PaymentSettled(s) if s.payment_id == id => Some(s.clone()),
            _ => None,
        })
        .expect("PaymentSettled");
    assert_eq!(settled.status.as_deref(), Some("in_flight"));
    assert_eq!(settled.allocations.len(), 1);
    assert_eq!(
        settled.allocations[0].amount,
        d("1000000.00"),
        "the event allocation stays gross"
    );
    assert_eq!(settled.allocations[0].discount_amount, Some(d("20000.00")));
}

// EPD-2: partial payment inside the window — the discount follows the ALLOCATED amount (2,000 on a
// 100,000 partial @ 2%), the bank moves 98,000, A/R still settles the gross 100,000 allocation.
#[tokio::test]
async fn partial_payment_discounts_on_the_allocated_amount() {
    let pool = pool().await;
    let (company, bank, ar, customer, discount_acct, inv) = (
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    let w = svc(
        &pool,
        Recorder::default(),
        HashMap::from([(
            inv,
            EarlyPayDecision {
                percent: d("2"),
                outstanding_basis: d("999999999"),
                account_id: discount_acct,
            },
        )]),
        Arc::new(Mutex::new(0)),
    );

    let id = w
        .create_payment(receive(
            company,
            bank,
            ar,
            customer,
            "100000",
            vec![NewAllocation {
                invoice_ref: inv,
                invoice_kind: "sales".into(),
                amount: d("100000"),
            }],
        ))
        .await
        .unwrap();
    in_org_scope(&pool, w.post_payment(id, &OkGl)).await.unwrap();

    let env = in_org_scope(&pool, w.build_settlement_post(id)).await.unwrap();
    assert_eq!(leg(&env, bank).debit, d("98000.00"));
    assert_eq!(leg(&env, discount_acct).debit, d("2000.00"));
    assert_eq!(leg(&env, ar).credit, d("100000.00"));
    assert_eq!(
        stamp_of(&pool, id, inv).await,
        (d("2000.00"), Some(discount_acct))
    );
}

// EPD-3: two allocations, two DISTINCT discount accounts ⇒ two distinct legs (an A/R discount
// expense never merges with another account's), one und discounted — the legs are exactly the
// named accounts, Σ debits = Σ credits = paid.
#[tokio::test]
async fn distinct_discount_accounts_stay_distinct_legs() {
    let pool = pool().await;
    let (company, bank, ar, customer) = (
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    let (inv_a, inv_b, inv_c, acct_a, acct_b) = (
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    let w = svc(
        &pool,
        Recorder::default(),
        HashMap::from([
            (
                inv_a,
                EarlyPayDecision {
                    percent: d("2"),
                    outstanding_basis: d("999999999"),
                    account_id: acct_a,
                },
            ), // 400,000 @ 2% = 8,000
            (
                inv_b,
                EarlyPayDecision {
                    percent: d("5"),
                    outstanding_basis: d("999999999"),
                    account_id: acct_b,
                },
            ), // 300,000 @ 5% = 15,000
               // inv_c: no decision ⇒ no discount leg
        ]),
        Arc::new(Mutex::new(0)),
    );

    let id = w
        .create_payment(receive(
            company,
            bank,
            ar,
            customer,
            "1000000",
            vec![
                NewAllocation {
                    invoice_ref: inv_a,
                    invoice_kind: "sales".into(),
                    amount: d("400000"),
                },
                NewAllocation {
                    invoice_ref: inv_b,
                    invoice_kind: "sales".into(),
                    amount: d("300000"),
                },
                NewAllocation {
                    invoice_ref: inv_c,
                    invoice_kind: "sales".into(),
                    amount: d("300000"),
                },
            ],
        ))
        .await
        .unwrap();
    in_org_scope(&pool, w.post_payment(id, &OkGl)).await.unwrap();

    let env = in_org_scope(&pool, w.build_settlement_post(id)).await.unwrap();
    assert_eq!(env.totals(), (d("1000000.00"), d("1000000.00")));
    assert_eq!(leg(&env, acct_a).debit, d("8000.00"));
    assert_eq!(leg(&env, acct_b).debit, d("15000.00"));
    assert_eq!(
        leg(&env, bank).debit,
        d("977000.00"),
        "1,000,000 − 8,000 − 15,000"
    );
    assert_eq!(
        leg(&env, ar).credit,
        d("1000000.00"),
        "A/R still settles the gross Σ"
    );
}

// EPD-4: no applicable discount (the resolver's Ok(None) — the out-of-window case is the INVOICE
// module's judgment, payment just sees "none") ⇒ the classic two-legged post, no discount column
// touched.
#[tokio::test]
async fn no_discount_is_the_two_leg_classic() {
    let pool = pool().await;
    let (company, bank, ar, customer) = (
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    let inv = Uuid::new_v4();
    let w = svc(
        &pool,
        Recorder::default(),
        HashMap::new(),
        Arc::new(Mutex::new(0)),
    );

    let id = w
        .create_payment(receive(
            company,
            bank,
            ar,
            customer,
            "500000",
            vec![NewAllocation {
                invoice_ref: inv,
                invoice_kind: "sales".into(),
                amount: d("500000"),
            }],
        ))
        .await
        .unwrap();
    in_org_scope(&pool, w.post_payment(id, &OkGl)).await.unwrap();

    let env = in_org_scope(&pool, w.build_settlement_post(id)).await.unwrap();
    assert_eq!(env.lines.len(), 2, "no discount ⇒ no third leg");
    assert_eq!(leg(&env, bank).debit, d("500000.00"));
    assert_eq!(leg(&env, ar).credit, d("500000.00"));
    assert_eq!(stamp_of(&pool, id, inv).await, (Decimal::ZERO, None));
}

// EPD-5: the mirrored reversal — reverse reads the STAMPS (not the resolver), flips every leg, and
// the `PaymentCancelled` event carries the discount per allocation. A payment reversed after its
// discount never re-asks the invoice module.
#[tokio::test]
async fn reversal_mirrors_the_discount_legs_from_the_stamps() {
    let pool = pool().await;
    let (company, bank, ar, customer, discount_acct, inv) = (
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    let rec = Recorder::default();
    let calls = Arc::new(Mutex::new(0));
    let w = svc(
        &pool,
        rec.clone(),
        HashMap::from([(
            inv,
            EarlyPayDecision {
                percent: d("2"),
                outstanding_basis: d("999999999"),
                account_id: discount_acct,
            },
        )]),
        calls.clone(),
    );

    let id = w
        .create_payment(receive(
            company,
            bank,
            ar,
            customer,
            "1000000",
            vec![NewAllocation {
                invoice_ref: inv,
                invoice_kind: "sales".into(),
                amount: d("1000000"),
            }],
        ))
        .await
        .unwrap();
    in_org_scope(&pool, w.post_payment(id, &OkGl)).await.unwrap();
    let resolved_after_post = *calls.lock().unwrap();
    assert_eq!(
        resolved_after_post, 1,
        "the decision resolved exactly once at post"
    );

    // Rebuild the settlement envelope from committed state — the resolver must NOT run again.
    let before = *calls.lock().unwrap();
    let _ = in_org_scope(&pool, w.build_settlement_post(id)).await.unwrap();
    assert_eq!(
        *calls.lock().unwrap(),
        before,
        "a committed post reads its stamps, never the resolver"
    );

    // The reversal: sign-flipped mirror, stamps as the source.
    in_org_scope(&pool, w.reverse_payment(id, &OkGl)).await.unwrap();
    let rev = in_org_scope(&pool, w.build_reversal_post(id)).await.unwrap();
    assert_eq!(rev.posting_type, "reversal");
    assert_eq!(rev.totals(), (d("1000000.00"), d("1000000.00")));
    assert_eq!(leg(&rev, bank).credit, d("980000.00"));
    assert_eq!(leg(&rev, discount_acct).credit, d("20000.00"));
    assert_eq!(leg(&rev, ar).debit, d("1000000.00"));
    assert_eq!(
        *calls.lock().unwrap(),
        before,
        "the reversal never re-asks the resolver"
    );

    // The cancel event mirrors the discount per allocation.
    let cancelled = rec
        .events
        .lock()
        .unwrap()
        .iter()
        .find_map(|e| match e {
            PaymentEvent::PaymentCancelled(c) if c.payment_id == id => Some(c.clone()),
            _ => None,
        })
        .expect("PaymentCancelled");
    assert_eq!(cancelled.allocations.len(), 1);
    assert_eq!(
        cancelled.allocations[0].discount_amount,
        Some(d("20000.00"))
    );
    assert_eq!(cancelled.allocations[0].amount, d("1000000.00"));
}

// EPD-6: deterministic replay — a GL-refused first attempt leaves no stamp; the retry re-resolves
// the SAME decision and assembles the IDENTICAL envelope (the window cannot flip between attempts
// because the posting date — the window's clock — does not move with wall time).
#[tokio::test]
async fn refused_then_retry_assembles_the_identical_envelope() {
    let pool = pool().await;
    let (company, bank, ar, customer, discount_acct, inv) = (
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    let w = svc(
        &pool,
        Recorder::default(),
        HashMap::from([(
            inv,
            EarlyPayDecision {
                percent: d("2"),
                outstanding_basis: d("999999999"),
                account_id: discount_acct,
            },
        )]),
        Arc::new(Mutex::new(0)),
    );

    let id = w
        .create_payment(receive(
            company,
            bank,
            ar,
            customer,
            "1000000",
            vec![NewAllocation {
                invoice_ref: inv,
                invoice_kind: "sales".into(),
                amount: d("1000000"),
            }],
        ))
        .await
        .unwrap();

    // First attempt: the ledger refuses; no stamp, no landing.
    assert!(matches!(
        in_org_scope(&pool, w.post_payment(id, &RejectingGl))
            .await
            .unwrap_err(),
        PaymentError::GlRejected { .. }
    ));
    assert_eq!(
        stamp_of(&pool, id, inv).await.0,
        Decimal::ZERO,
        "a refused attempt stamps nothing"
    );

    // Retry: same decision, same three legs, committed.
    in_org_scope(&pool, w.post_payment(id, &OkGl)).await.unwrap();
    let env = in_org_scope(&pool, w.build_settlement_post(id)).await.unwrap();
    assert_eq!(leg(&env, bank).debit, d("980000.00"));
    assert_eq!(leg(&env, discount_acct).debit, d("20000.00"));
    assert_eq!(leg(&env, ar).credit, d("1000000.00"));
    assert_eq!(
        stamp_of(&pool, id, inv).await,
        (d("20000.00"), Some(discount_acct))
    );
}

// EPD-7: a discount that would exceed the bank leg is clamped by the pure rule — the post stays
// balanced with a zero bank leg rather than going negative (the UnbalancedPost refusal is for the
// impossible, not the clamped).
#[tokio::test]
async fn clamped_discount_keeps_the_post_balanced() {
    let pool = pool().await;
    let (company, bank, ar, customer, discount_acct, inv) = (
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    let w = svc(
        &pool,
        Recorder::default(),
        HashMap::from([(
            inv,
            EarlyPayDecision {
                percent: d("150"),
                outstanding_basis: d("999999999"),
                account_id: discount_acct,
            },
        )]),
        Arc::new(Mutex::new(0)),
    );

    let id = w
        .create_payment(receive(
            company,
            bank,
            ar,
            customer,
            "100000",
            vec![NewAllocation {
                invoice_ref: inv,
                invoice_kind: "sales".into(),
                amount: d("100000"),
            }],
        ))
        .await
        .unwrap();
    in_org_scope(&pool, w.post_payment(id, &OkGl)).await.unwrap();

    let env = in_org_scope(&pool, w.build_settlement_post(id)).await.unwrap();
    assert_eq!(env.totals(), (d("100000.00"), d("100000.00")));
    assert_eq!(
        leg(&env, discount_acct).debit,
        d("100000.00"),
        "clamped to the allocated amount"
    );
    assert_eq!(leg(&env, bank).debit, Decimal::ZERO);
    assert_eq!(leg(&env, ar).credit, d("100000.00"));
}

// EPD-8: the BASIS clamp — an allocation may exceed the invoice's outstanding (payment bounds Σ
// allocations by the PAID amount only; the invoice module knocks the excess off to on-account
// AFTER this decision), so the discountable amount is min(allocated, outstanding-at-resolve).
// One payment of 1,200,000 against a 1,000,000 invoice at 2% takes 20,000 — never 24,000: the
// cumulative take can never exceed percent × what the invoice actually asked for. The sequential
// variant pins the moving basis: after the first knock-off lands, the resolver reports a smaller
// outstanding and the second payment discounts only the remainder.
#[tokio::test]
async fn over_allocated_payment_discount_clamps_to_invoice_outstanding() {
    let pool = pool().await;
    let (company, bank, ar, customer, discount_acct, inv) = (
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    let rec = Recorder::default();
    let w = svc(
        &pool,
        rec.clone(),
        HashMap::from([(
            inv,
            EarlyPayDecision {
                percent: d("2"),
                outstanding_basis: d("1000000"),
                account_id: discount_acct,
            },
        )]),
        Arc::new(Mutex::new(0)),
    );

    // One payment allocating MORE than the invoice's outstanding: paid 1,200,000, allocated 1,200,000
    // against a 1,000,000 invoice. The stamp is 2% × 1,000,000 — the basis, not the allocation.
    let id = w
        .create_payment(receive(
            company,
            bank,
            ar,
            customer,
            "1200000",
            vec![NewAllocation {
                invoice_ref: inv,
                invoice_kind: "sales".into(),
                amount: d("1200000"),
            }],
        ))
        .await
        .unwrap();
    in_org_scope(&pool, w.post_payment(id, &OkGl)).await.unwrap();
    let (stamped, stamped_acct) = stamp_of(&pool, id, inv).await;
    assert_eq!(stamped, d("20000.00"), "clamped to the invoice basis");
    assert_eq!(stamped_acct, Some(discount_acct));

    // The assembled post stays balanced with the clamped leg:
    //   Dr Bank 1,180,000 · Dr Discount 20,000 · Cr A/R 1,200,000 (the gross allocation).
    let env = in_org_scope(&pool, w.build_settlement_post(id)).await.unwrap();
    assert_eq!(env.totals(), (d("1200000.00"), d("1200000.00")));
    assert_eq!(leg(&env, discount_acct).debit, d("20000.00"));
    assert_eq!(leg(&env, bank).debit, d("1180000.00"));
    assert_eq!(leg(&env, ar).credit, d("1200000.00"));

    // Sequential partials with a MOVING basis: 400,000 first (basis 1,000,000 ⇒ 8,000), then —
    // after that knock-off lands — the resolver reports 600,000 outstanding and an 800,000
    // allocation takes 12,000 on the remainder. Cumulative 20,000 = exactly percent × invoice.
    let inv2 = Uuid::new_v4();
    let w1 = svc(
        &pool,
        rec.clone(),
        HashMap::from([(
            inv2,
            EarlyPayDecision {
                percent: d("2"),
                outstanding_basis: d("1000000"),
                account_id: discount_acct,
            },
        )]),
        Arc::new(Mutex::new(0)),
    );
    let first = w1
        .create_payment(receive(
            company,
            bank,
            ar,
            customer,
            "400000",
            vec![NewAllocation {
                invoice_ref: inv2,
                invoice_kind: "sales".into(),
                amount: d("400000"),
            }],
        ))
        .await
        .unwrap();
    in_org_scope(&pool, w1.post_payment(first, &OkGl)).await.unwrap();
    let (s1, _) = stamp_of(&pool, first, inv2).await;
    assert_eq!(s1, d("8000.00"), "first partial discounts its allocation");

    let w2 = svc(
        &pool,
        rec.clone(),
        HashMap::from([(
            inv2,
            EarlyPayDecision {
                percent: d("2"),
                outstanding_basis: d("600000"),
                account_id: discount_acct,
            },
        )]),
        Arc::new(Mutex::new(0)),
    );
    let second = w2
        .create_payment(receive(
            company,
            bank,
            ar,
            customer,
            "800000",
            vec![NewAllocation {
                invoice_ref: inv2,
                invoice_kind: "sales".into(),
                amount: d("800000"),
            }],
        ))
        .await
        .unwrap();
    in_org_scope(&pool, w2.post_payment(second, &OkGl)).await.unwrap();
    let (s2, _) = stamp_of(&pool, second, inv2).await;
    assert_eq!(
        s2,
        d("12000.00"),
        "second partial discounts the REMAINING basis"
    );
    assert_eq!(
        s1 + s2,
        d("20000.00"),
        "cumulative take = percent × invoice, across over-allocating partials"
    );
}
