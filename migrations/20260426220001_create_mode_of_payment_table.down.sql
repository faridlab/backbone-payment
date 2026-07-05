-- Down: drop payment.mode_of_payments table
DROP TABLE IF EXISTS payment.mode_of_payments CASCADE;
DROP FUNCTION IF EXISTS payment.mode_of_payments_audit_timestamp() CASCADE;
