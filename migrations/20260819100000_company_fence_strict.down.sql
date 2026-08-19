-- Revert the ADR-0014 strict fence re-statement for payment module.
-- The fence predates this migration (ADR-0008-era), so the honest reverse is to
-- re-state the same live policy, not to disarm the tables: a down that disabled RLS
-- would leave company data unfenced — a posture this module never had.

-- Re-state the pre-existing fence for payment.aging_buckets (identical policy; see header).
DROP POLICY IF EXISTS aging_buckets_company_isolation ON payment.aging_buckets;
CREATE POLICY aging_buckets_company_isolation ON payment.aging_buckets
    FOR ALL
    USING      (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid)
    WITH CHECK (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid);

-- Re-state the pre-existing fence for payment.aging_snapshots (identical policy; see header).
DROP POLICY IF EXISTS aging_snapshots_company_isolation ON payment.aging_snapshots;
CREATE POLICY aging_snapshots_company_isolation ON payment.aging_snapshots
    FOR ALL
    USING      (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid)
    WITH CHECK (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid);

-- Re-state the pre-existing fence for payment.dunning_actions (identical policy; see header).
DROP POLICY IF EXISTS dunning_actions_company_isolation ON payment.dunning_actions;
CREATE POLICY dunning_actions_company_isolation ON payment.dunning_actions
    FOR ALL
    USING      (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid)
    WITH CHECK (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid);

-- Re-state the pre-existing fence for payment.dunning_runs (identical policy; see header).
DROP POLICY IF EXISTS dunning_runs_company_isolation ON payment.dunning_runs;
CREATE POLICY dunning_runs_company_isolation ON payment.dunning_runs
    FOR ALL
    USING      (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid)
    WITH CHECK (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid);

-- Re-state the pre-existing fence for payment.payment_allocations (identical policy; see header).
DROP POLICY IF EXISTS payment_allocations_company_isolation ON payment.payment_allocations;
CREATE POLICY payment_allocations_company_isolation ON payment.payment_allocations
    FOR ALL
    USING      (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid)
    WITH CHECK (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid);

-- Re-state the pre-existing fence for payment.payment_entries (identical policy; see header).
DROP POLICY IF EXISTS payment_entries_company_isolation ON payment.payment_entries;
CREATE POLICY payment_entries_company_isolation ON payment.payment_entries
    FOR ALL
    USING      (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid)
    WITH CHECK (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid);

