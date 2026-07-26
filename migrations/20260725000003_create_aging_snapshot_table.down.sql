-- Down: drop payment.aging_snapshots table
DROP TABLE IF EXISTS payment.aging_snapshots CASCADE;
DROP FUNCTION IF EXISTS payment.aging_snapshots_audit_timestamp() CASCADE;
