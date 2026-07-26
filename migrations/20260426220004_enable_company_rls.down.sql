-- Down: remove the company RLS fence for payment module

-- Reverse the company RLS fence for payment.aging_snapshots
DROP POLICY IF EXISTS aging_snapshots_company_isolation ON payment.aging_snapshots;
ALTER TABLE payment.aging_snapshots NO FORCE ROW LEVEL SECURITY;
ALTER TABLE payment.aging_snapshots DISABLE ROW LEVEL SECURITY;

-- Reverse the company RLS fence for payment.aging_buckets
DROP POLICY IF EXISTS aging_buckets_company_isolation ON payment.aging_buckets;
ALTER TABLE payment.aging_buckets NO FORCE ROW LEVEL SECURITY;
ALTER TABLE payment.aging_buckets DISABLE ROW LEVEL SECURITY;

-- Reverse the company RLS fence for payment.dunning_runs
DROP POLICY IF EXISTS dunning_runs_company_isolation ON payment.dunning_runs;
ALTER TABLE payment.dunning_runs NO FORCE ROW LEVEL SECURITY;
ALTER TABLE payment.dunning_runs DISABLE ROW LEVEL SECURITY;

-- Reverse the company RLS fence for payment.dunning_actions
DROP POLICY IF EXISTS dunning_actions_company_isolation ON payment.dunning_actions;
ALTER TABLE payment.dunning_actions NO FORCE ROW LEVEL SECURITY;
ALTER TABLE payment.dunning_actions DISABLE ROW LEVEL SECURITY;

-- Reverse the company RLS fence for payment.payment_entries
DROP POLICY IF EXISTS payment_entries_company_isolation ON payment.payment_entries;
ALTER TABLE payment.payment_entries NO FORCE ROW LEVEL SECURITY;
ALTER TABLE payment.payment_entries DISABLE ROW LEVEL SECURITY;

-- Reverse the company RLS fence for payment.payment_allocations
DROP POLICY IF EXISTS payment_allocations_company_isolation ON payment.payment_allocations;
ALTER TABLE payment.payment_allocations NO FORCE ROW LEVEL SECURITY;
ALTER TABLE payment.payment_allocations DISABLE ROW LEVEL SECURITY;

