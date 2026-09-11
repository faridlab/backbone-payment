//! Guarded route composition — the RECOMMENDED way to mount the payment module.
//!
//! Hand-authored (user-owned). Read documents + **validated create** (payment-entry with
//! allocations); generic create/update/delete CRUD is NOT mounted, so a caller cannot write a
//! payment that over-allocates or bypass the settlement path. `PaymentWriteService` is built from
//! the pool (regen-safe). Posting (`post_payment`) needs a `GlPostSink` composition layer, so it is
//! service/job-driven, not an HTTP route.
//!
//! Tenancy (ADR-0029): the module carries no tenancy of its own. `org_auth` verifies the Bearer
//! token, resolves the session's org scope against the request's tenant tree, and runs every
//! handler inside that scope — the module's statements ride the request-dedicated connection it
//! binds, and the composing service's tenancy decorator does the actual row-level fencing. The
//! guard reads the tenant database from the `backbone_orm::PgPool` request extension, so this
//! surface must be mounted inside the composing service's tenant router (the same wiring every
//! org-guarded module requires).

use std::sync::Arc;

use axum::{
    extract::State, http::StatusCode, middleware::from_fn_with_state, response::IntoResponse,
    routing::post, Json, Router,
};
use backbone_auth::org::{org_auth, OrgVerifier};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::application::service::payment_write_service::{
    NewAllocation, NewPayment, PaymentError, PaymentWriteService,
};
use crate::PaymentModule;

use super::{create_mode_of_payment_read_routes, create_payment_entry_read_routes};

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
    message: String,
}
#[derive(Debug, Serialize)]
struct IdResponse {
    id: Uuid,
}
fn err(e: PaymentError) -> axum::response::Response {
    let s = StatusCode::from_u16(e.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (
        s,
        Json(ErrorBody {
            error: e.code(),
            message: e.to_string(),
        }),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AllocationBody {
    invoice_ref: Uuid,
    invoice_kind: String,
    amount: Decimal,
}
impl From<AllocationBody> for NewAllocation {
    fn from(b: AllocationBody) -> Self {
        NewAllocation {
            invoice_ref: b.invoice_ref,
            invoice_kind: b.invoice_kind,
            amount: b.amount,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreatePaymentBody {
    payment_number: String,
    // No tenancy field: the session scope is the one `org_auth` resolved and bound for the request,
    // never a body value — a client must not be able to name the tenant whose bank/party accounts
    // it moves money against. `branch_id` is a plain organizational label the caller names
    // explicitly, not a tenancy key.
    #[serde(default)]
    branch_id: Option<Uuid>,
    payment_type: String,
    #[serde(default)]
    party_type: Option<String>,
    #[serde(default)]
    party_id: Option<Uuid>,
    posting_date: chrono::NaiveDate,
    #[serde(default)]
    currency: Option<String>,
    #[serde(default)]
    mode_of_payment_id: Option<Uuid>,
    /// "manual" | "bank_transfer" | "cash" | "cheque" | "gateway" — the channel that decides the
    /// post's landing state. Absent = manual.
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    provider_txn_id: Option<Uuid>,
    bank_account_id: Uuid,
    party_account_id: Uuid,
    paid_amount: Decimal,
    #[serde(default)]
    reference_no: Option<String>,
    #[serde(default)]
    allocations: Vec<AllocationBody>,
}
async fn create_payment(
    State(svc): State<Arc<PaymentWriteService>>,
    Json(b): Json<CreatePaymentBody>,
) -> axum::response::Response {
    let p = NewPayment {
        payment_number: b.payment_number,
        branch_id: b.branch_id,
        payment_type: b.payment_type,
        party_type: b.party_type,
        party_id: b.party_id,
        posting_date: b.posting_date,
        currency: b.currency,
        mode_of_payment_id: b.mode_of_payment_id,
        method: b.method,
        provider_txn_id: b.provider_txn_id,
        bank_account_id: b.bank_account_id,
        party_account_id: b.party_account_id,
        paid_amount: b.paid_amount,
        reference_no: b.reference_no,
        allocations: b.allocations.into_iter().map(Into::into).collect(),
        withholding_amount: rust_decimal::Decimal::ZERO,
        withholding_account_id: None,
        withholding_tax_type: "none".into(),
    };
    match svc.create_payment(p).await {
        Ok(id) => (StatusCode::CREATED, Json(IdResponse { id })).into_response(),
        Err(e) => err(e),
    }
}

// The hand lifecycle verbs. There is deliberately NO route that writes `status` directly — the
// fused state machine's writers are exactly: submit, post (computes the landing), reject, reverse,
// and the bank-confirmation consumer. A PATCH-status route would hand callers a fifth writer and
// undo that contract. All of them are ID-only: the scope is the one `org_auth` resolved and bound
// for the request, never a body or path value — a client must not be able to name the tenant it
// writes into.
async fn submit_payment(
    State(svc): State<Arc<PaymentWriteService>>,
    axum::extract::Path(payment_id): axum::extract::Path<Uuid>,
) -> axum::response::Response {
    match svc.submit_payment(payment_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err(e),
    }
}
async fn reject_payment(
    State(svc): State<Arc<PaymentWriteService>>,
    axum::extract::Path(payment_id): axum::extract::Path<Uuid>,
) -> axum::response::Response {
    match svc.reject_payment(payment_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err(e),
    }
}

fn write_routes(svc: Arc<PaymentWriteService>, verifier: OrgVerifier) -> Router {
    Router::new()
        .route("/payment-entries", post(create_payment))
        .route("/payment-entries/:id/submit", post(submit_payment))
        .route("/payment-entries/:id/reject", post(reject_payment))
        // Every write above is scope-bound: `org_auth` rejects a request whose token is absent,
        // invalid, or names a unit outside this tenant's tree, and runs the handler inside the
        // resolved org scope — a handler only ever executes with a proven, fenced session.
        //
        // `route_layer`, not `layer`: `layer` would also wrap this router's fallback, so once merged
        // every *unmatched* path (e.g. the generic CRUD paths this surface deliberately does not
        // mount) would answer 401 instead of 404 — leaking "auth required" for routes that do not
        // exist, and masking the CRUD-bypass probes.
        .route_layer(from_fn_with_state(verifier, org_auth))
        .with_state(svc)
}

/// Mount the payment module: read documents + validated, scope-fenced creates. Generic mutation is
/// not mounted. **Prefer this over `PaymentModule::all_crud_routes()` for any real deployment.**
///
/// The composing service builds one [`OrgVerifier`] from its JWT secret and passes it here; the
/// surface derives its session from the token, so no tenant crosses the wire in a body. Mount
/// inside the tenant router with the `backbone_orm::PgPool` request extension attached — `org_auth`
/// resolves the scope against that pool.
pub fn create_guarded_payment_routes(
    m: &PaymentModule,
    pool: PgPool,
    verifier: OrgVerifier,
) -> Router {
    let write = Arc::new(PaymentWriteService::new(pool));
    // payment_entries carry per-unit data → their read route is scope-bound by the same `org_auth`
    // layer as the writes (it resolves and binds the session's org scope; the composing service's
    // tenancy decorator fences the rows). mode_of_payment is GLOBAL reference data (no org axis, no
    // RLS) — it stays public, unwrapped.
    let entity_reads = Router::new()
        .merge(create_payment_entry_read_routes(
            m.payment_entry_service.clone(),
        ))
        .route_layer(from_fn_with_state(verifier.clone(), org_auth));
    Router::new()
        .merge(create_mode_of_payment_read_routes(
            m.mode_of_payment_service.clone(),
        ))
        .merge(entity_reads)
        .merge(write_routes(write, verifier))
}
