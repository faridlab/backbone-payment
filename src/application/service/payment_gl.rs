//! Outbound GL-posting + reconciliation ports (hand-authored, user-owned) — re-export of the shared
//! contracts.
//!
//! The GL-posting wire types (`AccountingPostEnvelope`, `GlPostLine`, `GlPostAck`, `GlPostRejected`)
//! and the `GlPostSink` port now live in the shared `backbone-gl-posting` crate (backbone-framework
//! v2.7.5) — the single source for all producers (dedup of the former per-producer copies). This file re-exports them
//! under payment's existing paths so `payment_write_service`, the tests, and `application::service::*`
//! resolve unchanged. Payment is the settlement emitter: a receive posts `Dr Bank · Cr A/R [customer]`;
//! a pay posts `Dr A/P [supplier] · Cr Bank`, reached only through a `GlPostSink`; the ACL maps the
//! envelope into accounting's `PostingRequest`. Zero normal Cargo edge into backbone-accounting.
//!
//! The reconciliation contract (`ReconcileSink` + its wire types, backbone-framework v2.7.9) rides
//! the same crate. Payment itself never calls it — billing's settlement consumers do, with the
//! payment's own journal lines as the credit-side locators — but the re-export keeps the seam's
//! vocabulary importable from the producer side for tests and composing hosts.

pub use backbone_gl_posting::{
    AccountingPostEnvelope, GlPostAck, GlPostLine, GlPostRejected, GlPostSink, ReconcileEdgeAck,
    ReconcileLine, ReconcileOrigin, ReconcilePairRequest, ReconcileRejected, ReconcileSink,
    UnreconcilePairRequest,
};
