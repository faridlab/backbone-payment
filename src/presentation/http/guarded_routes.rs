//! Guarded route composition — the RECOMMENDED way to mount the payment module.
//!
//! Hand-authored (user-owned). Read documents + **validated create** (payment-entry with
//! allocations); generic create/update/delete CRUD is NOT mounted, so a caller cannot write a
//! payment that over-allocates or bypass the settlement path. `PaymentWriteService` is built from
//! the pool (regen-safe). Posting (`post_payment`) needs a `GlPostSink` composition layer, so it is
//! service/job-driven, not an HTTP route.

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::post, Json, Router};
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
struct ErrorBody { error: String, message: String }
#[derive(Debug, Serialize)]
struct IdResponse { id: Uuid }
fn err(e: PaymentError) -> axum::response::Response {
    let s = StatusCode::from_u16(e.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (s, Json(ErrorBody { error: e.code(), message: e.to_string() })).into_response()
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
        NewAllocation { invoice_ref: b.invoice_ref, invoice_kind: b.invoice_kind, amount: b.amount }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreatePaymentBody {
    payment_number: String,
    company_id: Uuid,
    #[serde(default)] branch_id: Option<Uuid>,
    payment_type: String,
    #[serde(default)] party_type: Option<String>,
    #[serde(default)] party_id: Option<Uuid>,
    posting_date: chrono::NaiveDate,
    #[serde(default)] currency: Option<String>,
    #[serde(default)] mode_of_payment_id: Option<Uuid>,
    bank_account_id: Uuid,
    party_account_id: Uuid,
    paid_amount: Decimal,
    #[serde(default)] reference_no: Option<String>,
    #[serde(default)] allocations: Vec<AllocationBody>,
}
async fn create_payment(State(svc): State<Arc<PaymentWriteService>>, Json(b): Json<CreatePaymentBody>) -> axum::response::Response {
    let p = NewPayment {
        payment_number: b.payment_number, company_id: b.company_id, branch_id: b.branch_id,
        payment_type: b.payment_type, party_type: b.party_type, party_id: b.party_id,
        posting_date: b.posting_date, currency: b.currency, mode_of_payment_id: b.mode_of_payment_id,
        bank_account_id: b.bank_account_id, party_account_id: b.party_account_id, paid_amount: b.paid_amount,
        reference_no: b.reference_no,
        allocations: b.allocations.into_iter().map(Into::into).collect(),
    };
    match svc.create_payment(p).await {
        Ok(id) => (StatusCode::CREATED, Json(IdResponse { id })).into_response(),
        Err(e) => err(e),
    }
}

fn write_routes(svc: Arc<PaymentWriteService>) -> Router {
    Router::new()
        .route("/payment-entries", post(create_payment))
        .with_state(svc)
}

/// Mount the payment module: read documents + validated creates. Generic mutation is not mounted.
/// **Prefer this over `PaymentModule::all_crud_routes()` for any real deployment.**
pub fn create_guarded_payment_routes(m: &PaymentModule, pool: PgPool) -> Router {
    let write = Arc::new(PaymentWriteService::new(pool));
    Router::new()
        .merge(create_mode_of_payment_read_routes(m.mode_of_payment_service.clone()))
        .merge(create_payment_entry_read_routes(m.payment_entry_service.clone()))
        .merge(write_routes(write))
}
