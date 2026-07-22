-- Reverse: drop the RLS fence + policy, then the company_id column.
-- Re-enables the pre-ADR-0010 shape (tenancy via payment_id FK only).

DROP POLICY IF EXISTS payment_allocations_company_isolation ON payment.payment_allocations;

ALTER TABLE payment.payment_allocations NO FORCE ROW LEVEL SECURITY;
ALTER TABLE payment.payment_allocations DISABLE ROW LEVEL SECURITY;

DROP INDEX IF EXISTS payment.idx_payment_allocations_company_id;

ALTER TABLE payment.payment_allocations DROP COLUMN company_id;
