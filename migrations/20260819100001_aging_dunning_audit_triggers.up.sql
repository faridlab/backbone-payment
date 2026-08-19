-- Audit timestamp triggers for the aging/dunning tables (payment module).
-- These stanzas originally sat in 20260426220006, before their tables existed in the
-- chain — fresh databases failed there. They run here, after the creates. On
-- databases that already applied the old file content, everything below is
-- idempotent (CREATE OR REPLACE FUNCTION, DROP TRIGGER IF EXISTS + CREATE).

-- ==============================================================================
-- Table: AgingSnapshot (payment.aging_snapshots)
-- ==============================================================================

-- Function to set metadata timestamps
CREATE OR REPLACE FUNCTION payment.aging_snapshots_audit_timestamp() RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.metadata = jsonb_set(NEW.metadata::jsonb, '{created_at}', to_jsonb(NOW()));
        NEW.metadata = jsonb_set(NEW.metadata::jsonb, '{updated_at}', to_jsonb(NOW()));
    ELSIF TG_OP = 'UPDATE' THEN
        NEW.metadata = jsonb_set(NEW.metadata::jsonb, '{updated_at}', to_jsonb(NOW()));
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Trigger to set timestamps on INSERT
DROP TRIGGER IF EXISTS aging_snapshots_insert_audit ON payment.aging_snapshots;
CREATE TRIGGER aging_snapshots_insert_audit BEFORE INSERT ON payment.aging_snapshots
    FOR EACH ROW EXECUTE FUNCTION payment.aging_snapshots_audit_timestamp();

-- Trigger to set updated_at on UPDATE
DROP TRIGGER IF EXISTS aging_snapshots_update_audit ON payment.aging_snapshots;
CREATE TRIGGER aging_snapshots_update_audit BEFORE UPDATE ON payment.aging_snapshots
    FOR EACH ROW EXECUTE FUNCTION payment.aging_snapshots_audit_timestamp();

-- ==============================================================================
-- Table: AgingBucket (payment.aging_buckets)
-- ==============================================================================

-- Function to set metadata timestamps
CREATE OR REPLACE FUNCTION payment.aging_buckets_audit_timestamp() RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.metadata = jsonb_set(NEW.metadata::jsonb, '{created_at}', to_jsonb(NOW()));
        NEW.metadata = jsonb_set(NEW.metadata::jsonb, '{updated_at}', to_jsonb(NOW()));
    ELSIF TG_OP = 'UPDATE' THEN
        NEW.metadata = jsonb_set(NEW.metadata::jsonb, '{updated_at}', to_jsonb(NOW()));
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Trigger to set timestamps on INSERT
DROP TRIGGER IF EXISTS aging_buckets_insert_audit ON payment.aging_buckets;
CREATE TRIGGER aging_buckets_insert_audit BEFORE INSERT ON payment.aging_buckets
    FOR EACH ROW EXECUTE FUNCTION payment.aging_buckets_audit_timestamp();

-- Trigger to set updated_at on UPDATE
DROP TRIGGER IF EXISTS aging_buckets_update_audit ON payment.aging_buckets;
CREATE TRIGGER aging_buckets_update_audit BEFORE UPDATE ON payment.aging_buckets
    FOR EACH ROW EXECUTE FUNCTION payment.aging_buckets_audit_timestamp();

-- ==============================================================================
-- Table: DunningRun (payment.dunning_runs)
-- ==============================================================================

-- Function to set metadata timestamps
CREATE OR REPLACE FUNCTION payment.dunning_runs_audit_timestamp() RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.metadata = jsonb_set(NEW.metadata::jsonb, '{created_at}', to_jsonb(NOW()));
        NEW.metadata = jsonb_set(NEW.metadata::jsonb, '{updated_at}', to_jsonb(NOW()));
    ELSIF TG_OP = 'UPDATE' THEN
        NEW.metadata = jsonb_set(NEW.metadata::jsonb, '{updated_at}', to_jsonb(NOW()));
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Trigger to set timestamps on INSERT
DROP TRIGGER IF EXISTS dunning_runs_insert_audit ON payment.dunning_runs;
CREATE TRIGGER dunning_runs_insert_audit BEFORE INSERT ON payment.dunning_runs
    FOR EACH ROW EXECUTE FUNCTION payment.dunning_runs_audit_timestamp();

-- Trigger to set updated_at on UPDATE
DROP TRIGGER IF EXISTS dunning_runs_update_audit ON payment.dunning_runs;
CREATE TRIGGER dunning_runs_update_audit BEFORE UPDATE ON payment.dunning_runs
    FOR EACH ROW EXECUTE FUNCTION payment.dunning_runs_audit_timestamp();

-- ==============================================================================
-- Table: DunningAction (payment.dunning_actions)
-- ==============================================================================

-- Function to set metadata timestamps
CREATE OR REPLACE FUNCTION payment.dunning_actions_audit_timestamp() RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.metadata = jsonb_set(NEW.metadata::jsonb, '{created_at}', to_jsonb(NOW()));
        NEW.metadata = jsonb_set(NEW.metadata::jsonb, '{updated_at}', to_jsonb(NOW()));
    ELSIF TG_OP = 'UPDATE' THEN
        NEW.metadata = jsonb_set(NEW.metadata::jsonb, '{updated_at}', to_jsonb(NOW()));
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Trigger to set timestamps on INSERT
DROP TRIGGER IF EXISTS dunning_actions_insert_audit ON payment.dunning_actions;
CREATE TRIGGER dunning_actions_insert_audit BEFORE INSERT ON payment.dunning_actions
    FOR EACH ROW EXECUTE FUNCTION payment.dunning_actions_audit_timestamp();

-- Trigger to set updated_at on UPDATE
DROP TRIGGER IF EXISTS dunning_actions_update_audit ON payment.dunning_actions;
CREATE TRIGGER dunning_actions_update_audit BEFORE UPDATE ON payment.dunning_actions
    FOR EACH ROW EXECUTE FUNCTION payment.dunning_actions_audit_timestamp();

-- ==============================================================================
