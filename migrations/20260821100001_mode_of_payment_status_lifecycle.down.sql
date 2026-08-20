-- Down: restore the is_active boolean and the composite index keyed on it.
-- Only 'inactive' rows are written back as FALSE; rows at the column default
-- map to the boolean default TRUE without an UPDATE.

DROP INDEX IF EXISTS payment.idx_mode_of_payments_mode_type_status;

ALTER TABLE payment.mode_of_payments ADD COLUMN is_active BOOLEAN NOT NULL DEFAULT TRUE;
UPDATE payment.mode_of_payments SET is_active = FALSE WHERE status = 'inactive';
ALTER TABLE payment.mode_of_payments DROP COLUMN status;
DROP TYPE IF EXISTS mode_of_payment_status;

CREATE INDEX idx_mode_of_payments_mode_type_is_active ON payment.mode_of_payments (mode_type, is_active);
