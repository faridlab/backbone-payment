-- Reverse the payment lifecycle fusion: collapse the confirmation dimension back
-- to the dual-state shape (in_flight/paid → posted; rejected → cancelled — the
-- pre-fusion lifecycle had no rejected state to land in), drop the channel and
-- discount columns, and restore the original enum type.

DROP INDEX IF EXISTS payment.idx_payment_entries_provider_txn;

ALTER TABLE payment.payment_allocations
    DROP COLUMN IF EXISTS discount_amount,
    DROP COLUMN IF EXISTS discount_account_id;

ALTER TABLE payment.payment_entries
    DROP COLUMN IF EXISTS method,
    DROP COLUMN IF EXISTS provider_txn_id;

DO $$ BEGIN
    CREATE TYPE payment_status_old AS ENUM ('draft', 'submitted', 'posted', 'cancelled');
EXCEPTION WHEN duplicate_object THEN NULL; END $$;

ALTER TABLE payment.payment_entries ADD COLUMN status_old payment_status_old;
UPDATE payment.payment_entries SET status_old = CASE status::text
    WHEN 'in_flight' THEN 'posted'
    WHEN 'paid' THEN 'posted'
    WHEN 'rejected' THEN 'cancelled'
    ELSE status::text
END::payment_status_old;

ALTER TABLE payment.payment_entries DROP COLUMN status;
ALTER TABLE payment.payment_entries RENAME COLUMN status_old TO status;
ALTER TYPE payment_status RENAME TO payment_status_v2;
ALTER TYPE payment_status_old RENAME TO payment_status;
DROP TYPE payment_status_v2;

DROP TYPE IF EXISTS payment_method;
