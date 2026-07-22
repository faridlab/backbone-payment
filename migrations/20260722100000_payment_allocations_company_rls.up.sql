-- Migration: direct company_id + FORCE RLS on payment.payment_allocations
-- ADR-0010 Decision A (payment review F1).
--
-- Before this change, payment_allocations inherited tenancy only via the payment_id
-- FK into payment_entries; the table itself had NO company_id column and NO RLS policy,
-- so a non-fenced read (e.g. a join that skipped payment_entries, or a mis-bound scope)
-- could cross tenants. This adds a denormalized company_id, backfills it from
-- payment_entries via the existing payment_id FK, sets it NOT NULL, and applies the
-- ADR-0008 invariant #1 fence (FORCE RLS + USING/WITH CHECK on app.company_id).
--
-- company_id is a LOGICAL FK to organization.Company.id (matches PaymentEntry's own
-- pattern: @exclude_from_foreign_key_check — the companies table lives in another
-- module's schema, so no hard SQL FK is added here).

-- 1. Add the column nullable so the backfill can run on existing rows.
ALTER TABLE payment.payment_allocations ADD COLUMN company_id UUID;

-- 2. Backfill from the parent payment_entry via payment_id.
--    payment_allocations.payment_id → payment.payment_entries.id is the single parent
--    FK (migrations/20260426220003_create_payment_allocation_table.up.sql:62), and
--    payment_entries.company_id is NOT NULL, so every allocation gets exactly one
--    company.
UPDATE payment.payment_allocations AS pa
   SET company_id = pe.company_id
  FROM payment.payment_entries AS pe
 WHERE pa.payment_id = pe.id
   AND pa.company_id IS NULL;

-- 3. Tighten to NOT NULL.
ALTER TABLE payment.payment_allocations ALTER COLUMN company_id SET NOT NULL;

-- 4. Index the tenant vector (new扫描 path for company-scoped lists).
CREATE INDEX IF NOT EXISTS idx_payment_allocations_company_id
    ON payment.payment_allocations (company_id);

-- 5. ADR-0008 invariant #1 fence: ENABLE + FORCE RLS, then the USING/WITH CHECK policy.
--    FORCE is required so the policy also binds the table OWNER (migrations/seeders
--    still bypass via BYPASSRLS on the owner role).
ALTER TABLE payment.payment_allocations ENABLE ROW LEVEL SECURITY;
ALTER TABLE payment.payment_allocations FORCE  ROW LEVEL SECURITY;

DROP POLICY IF EXISTS payment_allocations_company_isolation ON payment.payment_allocations;
CREATE POLICY payment_allocations_company_isolation ON payment.payment_allocations
    FOR ALL
    USING      (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid)
    WITH CHECK (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid);
