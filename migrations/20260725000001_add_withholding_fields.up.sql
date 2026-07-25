-- ADR-003: add PPh (withholding tax) fields to payment_entries.
-- All nullable/defaulted — backward-compatible (existing payments have no withholding → 2-line post).
ALTER TABLE payment.payment_entries ADD COLUMN IF NOT EXISTS withholding_amount DECIMAL(18,2) NOT NULL DEFAULT 0;
ALTER TABLE payment.payment_entries ADD COLUMN IF NOT EXISTS withholding_account_id UUID;
ALTER TABLE payment.payment_entries ADD COLUMN IF NOT EXISTS withholding_tax_type withholding_tax_type NOT NULL DEFAULT 'none';
