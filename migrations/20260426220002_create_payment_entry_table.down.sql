-- Down: drop payment.payment_entries table
DROP TABLE IF EXISTS payment.payment_entries CASCADE;
DROP FUNCTION IF EXISTS payment.payment_entries_audit_timestamp() CASCADE;
