//! Outbound GL-posting port (hand-authored, user-owned) — re-export of the shared contract.
//!
//! The GL-posting wire types (`AccountingPostEnvelope`, `GlPostLine`, `GlPostAck`, `GlPostRejected`)
//! and the `GlPostSink` port now live in the shared `backbone-gl-posting` crate (backbone-framework
//! v2.7.5) — the single source for all producers (ADR-0011 §6, phase 1). This file re-exports them
//! under payment's existing paths so `payment_write_service`, the tests, and `application::service::*`
//! resolve unchanged. Payment is the settlement emitter: a receive posts `Dr Bank · Cr A/R [customer]`;
//! a pay posts `Dr A/P [supplier] · Cr Bank`, reached only through a `GlPostSink`; the ACL maps the
//! envelope into accounting's `PostingRequest`. Zero normal Cargo edge into backbone-accounting.

pub use backbone_gl_posting::{
    AccountingPostEnvelope, GlPostAck, GlPostLine, GlPostRejected, GlPostSink,
};
