//! PSM — the fused payment state machine, end to end through the verbs. Payment-only (fake GL);
//! the landing computation, the CAS refusals, and the terminal states are asserted against the DB.
//! Requires DATABASE_URL (:5433/backbone_payment).
//!
//! The contract (the `PaymentStatus` doc block): draft→submitted (submit) · submitted→in_flight|paid
//! (post, landing computed) · in_flight→paid (bank-confirmation consumer ONLY) · in_flight|paid→
//! cancelled (reverse) · submitted→rejected (reject, terminal — the PRE-settlement stop; a landed
//! payment is exited by reverse, never reject). `posting_state` stays the
//! GL-sync truth and owns idempotency throughout.

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

use backbone_payment::application::service::payment_events::{PaymentEvent, PaymentEventSink};
use backbone_payment::application::service::payment_gl::{
    AccountingPostEnvelope, GlPostAck, GlPostRejected, GlPostSink,
};
use backbone_payment::application::service::payment_lifecycle::{
    landing_state, BankReconcilablePort,
};
use backbone_payment::application::service::payment_write_service::{
    NewAllocation, NewPayment, PaymentError, PaymentWriteService,
};

/// Tests inject the reconcilability read — the real probe reads accounting, which these fixtures do
/// not populate. `Reconcilable(bool)` lets each case pin BOTH landing arms.
struct Reconcilable(bool);
#[async_trait::async_trait]
impl BankReconcilablePort for Reconcilable {
    async fn bank_reconcilable(
        &self,
        _pool: &sqlx::PgPool,
        _company_id: uuid::Uuid,
        _account_id: uuid::Uuid,
    ) -> Result<bool, PaymentError> {
        Ok(self.0)
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

/// Fake GL sink that records envelopes and always acks.
#[derive(Default, Clone)]
struct OkGl {
    seen: Arc<Mutex<Vec<AccountingPostEnvelope>>>,
}
#[async_trait::async_trait]
impl GlPostSink for OkGl {
    async fn post(&self, env: &AccountingPostEnvelope) -> Result<GlPostAck, GlPostRejected> {
        assert!(
            env.is_balanced(),
            "payment emitted an UNBALANCED post: {env:?}"
        );
        self.seen.lock().unwrap().push(env.clone());
        Ok(GlPostAck {
            post_id: Uuid::new_v4(),
            journal_id: Uuid::new_v4(),
            idempotent_reuse: false,
        })
    }
}

fn svc(pool: &PgPool, reconcilable: bool, rec: Recorder) -> PaymentWriteService {
    PaymentWriteService::with_sink(pool.clone(), Arc::new(rec))
        .with_reconcilable_port(Arc::new(Reconcilable(reconcilable)))
}

fn new_payment(
    _company: Uuid,
    method: Option<&str>,
    paid: &str,
    inv: Uuid,
    allocated: &str,
) -> NewPayment {
    NewPayment {
        payment_number: uq("PE"),
        branch_id: None,
        payment_type: "receive".into(),
        party_type: Some("customer".into()),
        party_id: Some(Uuid::new_v4()),
        posting_date: chrono::NaiveDate::from_ymd_opt(2026, 8, 21).unwrap(),
        currency: None,
        mode_of_payment_id: None,
        method: method.map(Into::into),
        provider_txn_id: None,
        bank_account_id: Uuid::new_v4(),
        party_account_id: Uuid::new_v4(),
        paid_amount: d(paid),
        reference_no: None,
        allocations: vec![NewAllocation {
            invoice_ref: inv,
            invoice_kind: "sales".into(),
            amount: d(allocated),
        }],
        withholding_amount: Decimal::ZERO,
        withholding_account_id: None,
        withholding_tax_type: "none".into(),
    }
}

async fn status_of(pool: &PgPool, id: Uuid) -> (String, String) {
    sqlx::query_as(
        "SELECT status::text, posting_state::text FROM payment.payment_entries WHERE id=$1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap()
}

// PSM-1: the landing computation — where a post lands is a pure function of bank reconcilability ×
// channel. Cash, or any non-reconcilable channel, is `paid` on the spot (no statement will ever
// confirm it — waiting would be a stuck state); a reconcilable channel is `in_flight` until the
// bank confirms. Pinned directly, then behaviorally below.
#[test]
fn landing_state_matrix() {
    assert_eq!(landing_state(true, "manual"), "in_flight");
    assert_eq!(landing_state(true, "bank_transfer"), "in_flight");
    assert_eq!(landing_state(true, "cheque"), "in_flight");
    assert_eq!(landing_state(true, "gateway"), "in_flight");
    assert_eq!(
        landing_state(true, "cash"),
        "paid",
        "cash needs no statement confirmation"
    );
    assert_eq!(
        landing_state(false, "manual"),
        "paid",
        "non-reconcilable bank account ⇒ nothing to wait for"
    );
    assert_eq!(landing_state(false, "bank_transfer"), "paid");
    assert_eq!(landing_state(false, "cash"), "paid");
}

// PSM-2: behavioral landing — a reconcilable manual post lands in_flight; a cash post (and a
// non-reconcilable manual post) lands paid immediately. posting_state is 'posted' in all cases: the
// GL synced.
#[tokio::test]
async fn post_lands_by_channel_and_reconcilability() {
    let pool = pool().await;
    let company = Uuid::new_v4();

    // Reconcilable + manual ⇒ in_flight.
    let w = svc(&pool, true, Recorder::default());
    let id = w
        .create_payment(new_payment(
            company,
            None,
            "100000",
            Uuid::new_v4(),
            "100000",
        ))
        .await
        .unwrap();
    in_org_scope(&pool, w.post_payment(id, &OkGl::default())).await.unwrap();
    assert_eq!(
        status_of(&pool, id).await,
        ("in_flight".into(), "posted".into())
    );

    // Reconcilable + cash ⇒ paid now.
    let id = w
        .create_payment(new_payment(
            company,
            Some("cash"),
            "100000",
            Uuid::new_v4(),
            "100000",
        ))
        .await
        .unwrap();
    in_org_scope(&pool, w.post_payment(id, &OkGl::default())).await.unwrap();
    assert_eq!(status_of(&pool, id).await, ("paid".into(), "posted".into()));

    // Non-reconcilable + manual ⇒ paid now.
    let w = svc(&pool, false, Recorder::default());
    let id = w
        .create_payment(new_payment(
            company,
            None,
            "100000",
            Uuid::new_v4(),
            "100000",
        ))
        .await
        .unwrap();
    in_org_scope(&pool, w.post_payment(id, &OkGl::default())).await.unwrap();
    assert_eq!(status_of(&pool, id).await, ("paid".into(), "posted".into()));
}

// PSM-3: the submit verb — draft→submitted; a second submit refuses with the actual state (never a
// silent no-op); a submitted payment posts normally (landing as computed).
#[tokio::test]
async fn submit_verb_and_refusals() {
    let pool = pool().await;
    let company = Uuid::new_v4();
    let w = svc(&pool, true, Recorder::default());
    let id = w
        .create_payment(new_payment(
            company,
            None,
            "100000",
            Uuid::new_v4(),
            "100000",
        ))
        .await
        .unwrap();

    w.submit_payment(id).await.unwrap();
    assert_eq!(
        status_of(&pool, id).await,
        ("submitted".into(), "pending".into())
    );

    match w.submit_payment(id).await.unwrap_err() {
        PaymentError::NotSubmittable(s) => assert_eq!(s, "submitted"),
        e => panic!("expected NotSubmittable, got {e:?}"),
    }

    // Post from submitted: the normal happy path (submit is the operator's readiness mark).
    in_org_scope(&pool, w.post_payment(id, &OkGl::default())).await.unwrap();
    assert_eq!(
        status_of(&pool, id).await,
        ("in_flight".into(), "posted".into())
    );

    // Post-landing, submit is refused.
    match w.submit_payment(id).await.unwrap_err() {
        PaymentError::NotSubmittable(s) => assert_eq!(s, "in_flight"),
        e => panic!("expected NotSubmittable, got {e:?}"),
    }
}

// PSM-4: the reject verb — the terminal operator exit, from submitted only (the pre-settlement
// stop: nothing has committed, so rejecting is a pure label flip). Refused from draft (discard
// instead — nothing settled), from in_flight/paid (reverse instead — those have GL to unwind),
// and from itself.
#[tokio::test]
async fn reject_verb_arms() {
    let pool = pool().await;
    let company = Uuid::new_v4();
    let w = svc(&pool, true, Recorder::default());

    // draft refuses.
    let draft = w
        .create_payment(new_payment(
            company,
            None,
            "100000",
            Uuid::new_v4(),
            "100000",
        ))
        .await
        .unwrap();
    match w.reject_payment(draft).await.unwrap_err() {
        PaymentError::NotRejectable(s) => assert_eq!(s, "draft"),
        e => panic!("expected NotRejectable, got {e:?}"),
    }

    // submitted → rejected, terminal.
    let sub = w
        .create_payment(new_payment(
            company,
            None,
            "100000",
            Uuid::new_v4(),
            "100000",
        ))
        .await
        .unwrap();
    w.submit_payment(sub).await.unwrap();
    w.reject_payment(sub).await.unwrap();
    assert_eq!(
        status_of(&pool, sub).await,
        ("rejected".into(), "pending".into())
    );
    match w.reject_payment(sub).await.unwrap_err() {
        PaymentError::NotRejectable(s) => assert_eq!(s, "rejected"),
        e => panic!("expected NotRejectable, got {e:?}"),
    }

    // in_flight REFUSES — a landed payment has a committed journal and a billing knock-off; its
    // exit is reverse (which unwinds the GL and restores the invoices). Rejecting it would strand
    // the journal with no verb left that could remove it: reverse refuses `rejected`, and the
    // confirmation consumer CASes `in_flight` only.
    let flt = w
        .create_payment(new_payment(
            company,
            None,
            "100000",
            Uuid::new_v4(),
            "100000",
        ))
        .await
        .unwrap();
    in_org_scope(&pool, w.post_payment(flt, &OkGl::default())).await.unwrap();
    assert_eq!(
        status_of(&pool, flt).await,
        ("in_flight".into(), "posted".into())
    );
    match w.reject_payment(flt).await.unwrap_err() {
        PaymentError::NotRejectable(s) => assert_eq!(s, "in_flight"),
        e => panic!("expected NotRejectable, got {e:?}"),
    }
    // …and the exit works: reverse succeeds and emits the cancellation.
    let rec2 = Recorder::default();
    let w2 = svc(&pool, true, rec2.clone());
    in_org_scope(&pool, w2.reverse_payment(flt, &OkGl::default())).await.unwrap();
    assert_eq!(
        status_of(&pool, flt).await,
        ("cancelled".into(), "posted".into())
    );
    assert!(
        rec2.events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, PaymentEvent::PaymentCancelled(..))),
        "reverse emitted PaymentCancelled for the rejected-refused payment"
    );

    // paid refuses — the exit for settled money is a reversal.
    let cash = w
        .create_payment(new_payment(
            company,
            Some("cash"),
            "100000",
            Uuid::new_v4(),
            "100000",
        ))
        .await
        .unwrap();
    in_org_scope(&pool, w.post_payment(cash, &OkGl::default())).await.unwrap();
    match w.reject_payment(cash).await.unwrap_err() {
        PaymentError::NotRejectable(s) => assert_eq!(s, "paid"),
        e => panic!("expected NotRejectable, got {e:?}"),
    }
}

// PSM-5: the reverse verb — in_flight|paid→cancelled (posting_state stays 'posted': the reversal is
// itself a GL post); repeat reverse is an idempotent no-op; draft and rejected refuse (no GL to
// unwind).
#[tokio::test]
async fn reverse_verb_arms() {
    let pool = pool().await;
    let company = Uuid::new_v4();
    let w = svc(&pool, true, Recorder::default());

    // draft refuses.
    let draft = w
        .create_payment(new_payment(
            company,
            None,
            "100000",
            Uuid::new_v4(),
            "100000",
        ))
        .await
        .unwrap();
    match in_org_scope(&pool, w
        .reverse_payment(draft, &OkGl::default()))
        .await
        .unwrap_err()
    {
        PaymentError::NotReversible(s) => assert_eq!(s, "draft"),
        e => panic!("expected NotReversible, got {e:?}"),
    }

    // in_flight → cancelled; posting_state remains posted.
    let id = w
        .create_payment(new_payment(
            company,
            None,
            "100000",
            Uuid::new_v4(),
            "100000",
        ))
        .await
        .unwrap();
    in_org_scope(&pool, w.post_payment(id, &OkGl::default())).await.unwrap();
    let out = in_org_scope(&pool, w.reverse_payment(id, &OkGl::default())).await.unwrap();
    assert!(!out.idempotent_reuse);
    assert_eq!(
        status_of(&pool, id).await,
        ("cancelled".into(), "posted".into())
    );

    // Repeat reverse: idempotent (no second mirror journal effect, no second event).
    let rec = Recorder::default();
    let w2 = svc(&pool, true, rec.clone());
    let out2 = in_org_scope(&pool, w2.reverse_payment(id, &OkGl::default())).await.unwrap();
    assert!(
        out2.idempotent_reuse,
        "second reverse is an idempotent no-op"
    );
    assert!(
        !rec.events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, PaymentEvent::PaymentCancelled(c) if c.payment_id == id)),
        "cancelled event emitted exactly once across two reverses"
    );

    // rejected refuses.
    let sub = w
        .create_payment(new_payment(
            company,
            None,
            "100000",
            Uuid::new_v4(),
            "100000",
        ))
        .await
        .unwrap();
    w.submit_payment(sub).await.unwrap();
    w.reject_payment(sub).await.unwrap();
    match in_org_scope(&pool, w.reverse_payment(sub, &OkGl::default())).await.unwrap_err() {
        PaymentError::NotReversible(s) => assert_eq!(s, "rejected"),
        e => panic!("expected NotReversible, got {e:?}"),
    }
}

// PSM-6: an unknown channel refuses at CREATE, not at post — the method is part of the payment's
// identity (it decides the landing), so a bad value never persists.
#[tokio::test]
async fn unknown_method_refused_at_create() {
    let pool = pool().await;
    let w = svc(&pool, true, Recorder::default());
    let e = w
        .create_payment(new_payment(
            Uuid::new_v4(),
            Some("carrier_pigeon"),
            "100000",
            Uuid::new_v4(),
            "100000",
        ))
        .await
        .unwrap_err();
    assert!(matches!(e, PaymentError::UnknownPaymentMethod(m) if m == "carrier_pigeon"));
}

// PSM-7: the default reconcilability probe is fail-closed. Without an injected stub, the real read
// runs against accounting; a bank account that does not exist there REFUSES the post — the landing
// state is unknowable, and guessing (either direction) strands or silently confirms real money.
#[tokio::test]
async fn default_probe_refuses_on_unknown_bank_account() {
    let pool = pool().await;
    let w = PaymentWriteService::new(pool.clone());
    let id = w
        .create_payment(new_payment(
            Uuid::new_v4(),
            None,
            "100000",
            Uuid::new_v4(),
            "100000",
        ))
        .await
        .unwrap();
    let gl = OkGl::default();
    match in_org_scope(&pool, w.post_payment(id, &gl)).await.unwrap_err() {
        PaymentError::ReconcilableProbeRefused(_) => {}
        e => panic!("expected ReconcilableProbeRefused, got {e:?}"),
    }
    assert!(
        gl.seen.lock().unwrap().is_empty(),
        "the sink must not be driven when the probe refuses"
    );
    // Nothing moved: still draft/pending.
    assert_eq!(
        status_of(&pool, id).await,
        ("draft".into(), "pending".into())
    );
}

// PSM-6: `mark_failed` is CAS-guarded on posting_state — a `failed` stamp may land only on a
// pending-or-failed entry, never over a committed `posted`. Without the guard, a spurious sink
// error (transport timeout AFTER the GL committed) arriving from a concurrent retry would
// overwrite the live truth, and the entry would enter a state no verb can exit: reverse's gate
// would pass, the mirror journal would post, and `mark_cancelled` would match zero rows — GL
// showing original + reversal while the cancellation event never fires. Pinned at the repository
// altitude because the service's only call site is the deliberately-ignored failure arm.
#[tokio::test]
async fn mark_failed_cannot_overwrite_a_committed_post() {
    let pool = pool().await;
    let company = Uuid::new_v4();
    let w = svc(&pool, true, Recorder::default());
    let id = w
        .create_payment(new_payment(
            company,
            None,
            "100000",
            Uuid::new_v4(),
            "100000",
        ))
        .await
        .unwrap();
    in_org_scope(&pool, w.post_payment(id, &OkGl::default())).await.unwrap();
    assert_eq!(
        status_of(&pool, id).await,
        ("in_flight".into(), "posted".into())
    );

    // The failure stamp bounces off the committed post (0 rows touched — the method reports ()
    // by contract, so the pin is the state itself).
    backbone_payment::PaymentEntryRepository::new(pool.clone())
        .mark_failed(&pool, id)
        .await
        .unwrap();
    assert_eq!(
        status_of(&pool, id).await,
        ("in_flight".into(), "posted".into()),
        "mark_failed must not overwrite a committed posted entry"
    );

    // …and the entry stays fully reversible: reverse succeeds and emits, proving the state the
    // guard prevented (failed-over-posted) is the only one that could have stranded it.
    let rec = Recorder::default();
    let w2 = svc(&pool, true, rec.clone());
    in_org_scope(&pool, w2.reverse_payment(id, &OkGl::default())).await.unwrap();
    assert_eq!(
        status_of(&pool, id).await,
        ("cancelled".into(), "posted".into())
    );
    assert!(
        rec.events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, PaymentEvent::PaymentCancelled(..))),
        "reverse emitted PaymentCancelled — the exit the failed-overwrite would have blocked"
    );
}

// PSM-7: the concurrent-post race the mark_failed guard exists for — two post attempts on the same
// entry; the winner's sink acks (GL commits, mark_posted lands), the loser's sink returns a
// SPURIOUS error (the transport died after the ledger committed). The loser's mark_failed must
// bounce off the guard (or land before the winner's CAS, whose failed-arm is the legitimate
// retry path) — either interleaving ends posted+landed, and the entry reverses cleanly with the
// cancellation event. Without the guard, the loser's stamp would leave a journal whose
// cancellation can never emit.
#[tokio::test]
async fn sink_error_after_commit_keeps_entry_reversible() {
    let pool = pool().await;
    let company = Uuid::new_v4();
    let rec = Recorder::default();
    let w = svc(&pool, true, rec.clone());
    let id = w
        .create_payment(new_payment(
            company,
            None,
            "100000",
            Uuid::new_v4(),
            "100000",
        ))
        .await
        .unwrap();

    /// Acks the first invocation, then errs forever — the lying transport.
    struct AckThenErr {
        invocations: Arc<Mutex<usize>>,
    }
    #[async_trait::async_trait]
    impl GlPostSink for AckThenErr {
        async fn post(&self, env: &AccountingPostEnvelope) -> Result<GlPostAck, GlPostRejected> {
            let n = {
                let mut g = self.invocations.lock().unwrap();
                *g += 1;
                *g
            };
            if n == 1 {
                assert!(env.is_balanced(), "unbalanced post: {env:?}");
                Ok(GlPostAck {
                    post_id: Uuid::new_v4(),
                    journal_id: Uuid::new_v4(),
                    idempotent_reuse: false,
                })
            } else {
                Err(GlPostRejected {
                    code: "transport_timeout_after_commit".into(),
                    message: "spurious: the ledger committed".into(),
                })
            }
        }
    }

    let invocations = Arc::new(Mutex::new(0));
    let sink = AckThenErr {
        invocations: invocations.clone(),
    };
    // A second service handle over the same pool — two operators racing the same entry.
    let w2 = svc(&pool, true, Recorder::default());
    // Both racers run inside org request scopes — the settle path reads the legacy company
    // twin off the ambient scope (ADR-0029).
    let (a, b) = tokio::join!(
        in_org_scope(&pool, w.post_payment(id, &sink)),
        in_org_scope(&pool, w2.post_payment(id, &sink)),
    );
    // Exactly one acked, exactly one got the spurious rejection (whichever won the race).
    assert!(a.is_err() != b.is_err(), "one winner, one spurious loser");
    assert_eq!(*invocations.lock().unwrap(), 2);

    // The committed truth survived the loser's failure stamp.
    assert_eq!(
        status_of(&pool, id).await,
        ("in_flight".into(), "posted".into()),
        "the spurious mark_failed cannot overwrite the committed post"
    );

    // And the entry still exits cleanly, event and all.
    let rec3 = Recorder::default();
    let w3 = svc(&pool, true, rec3.clone());
    in_org_scope(&pool, w3.reverse_payment(id, &OkGl::default())).await.unwrap();
    assert_eq!(
        status_of(&pool, id).await,
        ("cancelled".into(), "posted".into())
    );
    assert!(
        rec3.events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, PaymentEvent::PaymentCancelled(..))),
        "reverse emitted PaymentCancelled after the raced post"
    );
}
