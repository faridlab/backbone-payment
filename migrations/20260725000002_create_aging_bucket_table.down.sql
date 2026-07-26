-- Down: drop payment.aging_buckets table
DROP TABLE IF EXISTS payment.aging_buckets CASCADE;
DROP FUNCTION IF EXISTS payment.aging_buckets_audit_timestamp() CASCADE;
