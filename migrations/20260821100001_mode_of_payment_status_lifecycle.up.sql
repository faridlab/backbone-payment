-- Migration: replace the payment-mode lifecycle boolean with a status enum
-- mode_of_payments carried `is_active BOOLEAN NOT NULL DEFAULT TRUE`; the
-- tree-wide convention is one `status` enum field per lifecycle (see
-- docs/refactoring-schema in the serpa workspace). The boolean migrates only
-- rows deviating from its own column default. The enum type is created
-- unqualified so it lands beside the module's other enum types (public), where
-- the generated sqlx type_name resolves.

DO $$ BEGIN
    CREATE TYPE mode_of_payment_status AS ENUM ('active', 'inactive');
EXCEPTION WHEN duplicate_object THEN NULL; END $$;

ALTER TABLE payment.mode_of_payments ADD COLUMN status mode_of_payment_status NOT NULL DEFAULT 'active';
UPDATE payment.mode_of_payments SET status = 'inactive' WHERE NOT is_active;
ALTER TABLE payment.mode_of_payments DROP COLUMN is_active;

DROP INDEX IF EXISTS payment.idx_mode_of_payments_mode_type_is_active;
CREATE INDEX idx_mode_of_payments_mode_type_status ON payment.mode_of_payments (mode_type, status);
