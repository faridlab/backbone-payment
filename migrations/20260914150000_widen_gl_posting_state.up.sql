-- Widen the shared `gl_posting_state` enum to the values this module depends on.
--
-- The enum-creation migration now guarantees these, which fixes any database
-- built from scratch. A database that already ran that migration carries it in
-- the ledger and will never run it again, so the repair needs its own file —
-- this one — to reach the estate that is already live.
--
-- Idempotent, and a no-op wherever the values are already present.
--
-- Top-level statements on purpose: a new enum value cannot be used in the
-- transaction that added it, so this file widens and does nothing else.

ALTER TYPE gl_posting_state ADD VALUE IF NOT EXISTS 'pending';
ALTER TYPE gl_posting_state ADD VALUE IF NOT EXISTS 'posted';
ALTER TYPE gl_posting_state ADD VALUE IF NOT EXISTS 'failed';
