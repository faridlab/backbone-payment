-- Down: drop payment.payment_allocations table
DROP TABLE IF EXISTS payment.payment_allocations CASCADE;
DROP FUNCTION IF EXISTS payment.payment_allocations_audit_timestamp() CASCADE;
