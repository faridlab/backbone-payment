-- Hand-authored (user-owned). Not regenerated.
--
-- Best-effort restore sketch for the tenancy strip (ADR-0029). This is a breaking module
-- release against dev-stage databases: the down re-adds the company_id column as nullable
-- with the company-leading indexes in their final pre-strip shapes, but restores NO data —
-- rows written after the strip (or after the decorator re-keyed them) carry org_unit_id
-- only. The composing service's tenancy decorator remains the live fence; the
-- <table>_company_isolation policies are NOT recreated here. Treat this down as a
-- schema-shape sketch for archaeology, not a usable rollback.

ALTER TABLE payment.payment_entries    ADD COLUMN IF NOT EXISTS company_id uuid;
ALTER TABLE payment.payment_allocations ADD COLUMN IF NOT EXISTS company_id uuid;
ALTER TABLE payment.aging_snapshots    ADD COLUMN IF NOT EXISTS company_id uuid;
ALTER TABLE payment.aging_buckets      ADD COLUMN IF NOT EXISTS company_id uuid;
ALTER TABLE payment.dunning_runs       ADD COLUMN IF NOT EXISTS company_id uuid;
ALTER TABLE payment.dunning_actions    ADD COLUMN IF NOT EXISTS company_id uuid;

-- ── payment_entries ────────────────────────────────────────────────────────────
CREATE INDEX IF NOT EXISTS idx_payment_entries_company_id_party_type_party_id_status
    ON payment.payment_entries (company_id, party_type, party_id, status);

-- ── payment_allocations ────────────────────────────────────────────────────────
CREATE INDEX IF NOT EXISTS idx_payment_allocations_company_id
    ON payment.payment_allocations (company_id);

-- ── aging_snapshots ────────────────────────────────────────────────────────────
CREATE UNIQUE INDEX IF NOT EXISTS idx_aging_snapshots_company_id_as_of_date_direction
    ON payment.aging_snapshots (company_id, as_of_date, direction) WHERE (metadata->>'deleted_at') IS NULL;
CREATE INDEX IF NOT EXISTS idx_aging_snapshots_company_id_as_of_date
    ON payment.aging_snapshots (company_id, as_of_date);

-- ── aging_buckets ──────────────────────────────────────────────────────────────
CREATE INDEX IF NOT EXISTS idx_aging_buckets_company_id_party_id_bucket
    ON payment.aging_buckets (company_id, party_id, bucket);

-- ── dunning_runs ───────────────────────────────────────────────────────────────
CREATE INDEX IF NOT EXISTS idx_dunning_runs_company_id_as_of_date
    ON payment.dunning_runs (company_id, as_of_date);

-- ── dunning_actions ────────────────────────────────────────────────────────────
CREATE INDEX IF NOT EXISTS idx_dunning_actions_company_id_party_id_status
    ON payment.dunning_actions (company_id, party_id, status);
