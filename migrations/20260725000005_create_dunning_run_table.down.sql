-- Down: drop payment.dunning_runs table
DROP TABLE IF EXISTS payment.dunning_runs CASCADE;
DROP FUNCTION IF EXISTS payment.dunning_runs_audit_timestamp() CASCADE;
