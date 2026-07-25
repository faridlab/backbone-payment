-- Reverse ADR-003: remove PPh fields (restores the pre-withholding shape).
ALTER TABLE payment.payment_entries DROP COLUMN IF EXISTS withholding_tax_type;
ALTER TABLE payment.payment_entries DROP COLUMN IF EXISTS withholding_account_id;
ALTER TABLE payment.payment_entries DROP COLUMN IF EXISTS withholding_amount;
