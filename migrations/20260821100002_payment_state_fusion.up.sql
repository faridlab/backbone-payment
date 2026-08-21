-- Fuse the payment lifecycle: PaymentStatus drops `posted` (that truth stays in
-- posting_state, the GL-sync column) and gains the confirmation dimension
-- in_flight/paid plus the terminal `rejected`; the entry gains the channel
-- dimension `method` + `provider_txn_id`; allocations gain the materialized
-- early-pay-discount decision (amount + account, stamped at post).
--
-- The status swap backfills CONDITIONALLY: `posted` rows split by whether the
-- bank already confirmed them — a clearance recorded against the payment means
-- paid, otherwise in_flight (awaiting confirmation; a re-drift command exists).
-- The clearance probe is guarded with to_regclass because this module's
-- standalone test database has no banking schema — an unguarded cross-schema
-- EXISTS would hard-fail there. Schema absent ⇒ every posted row conservatively
-- lands in_flight. Enum types are created UNQUALIFIED so they land beside the
-- module's other enum types (public), where the generated sqlx type_name resolves.

DO $$ BEGIN
    CREATE TYPE payment_method AS ENUM ('manual', 'bank_transfer', 'cash', 'cheque', 'gateway');
EXCEPTION WHEN duplicate_object THEN NULL; END $$;

DO $$ BEGIN
    CREATE TYPE payment_status_v2 AS ENUM ('draft', 'submitted', 'in_flight', 'paid', 'cancelled', 'rejected');
EXCEPTION WHEN duplicate_object THEN NULL; END $$;

-- Split each posted row: paid when a live clearance names it, else in_flight.
ALTER TABLE payment.payment_entries ADD COLUMN status_new payment_status_v2;
UPDATE payment.payment_entries SET status_new = CASE
    WHEN status::text <> 'posted' THEN status::text::payment_status_v2
    WHEN to_regclass('banking.bank_clearances') IS NOT NULL AND EXISTS (
        SELECT 1 FROM banking.bank_clearances bc
        WHERE bc.matched_source_type = 'payment'
          AND bc.matched_source_id = payment.payment_entries.id
          AND (bc.metadata->>'deleted_at') IS NULL
    ) THEN 'paid'::payment_status_v2
    ELSE 'in_flight'::payment_status_v2
END;

ALTER TABLE payment.payment_entries DROP COLUMN status;
ALTER TABLE payment.payment_entries RENAME COLUMN status_new TO status;
ALTER TYPE payment_status RENAME TO payment_status_old;
ALTER TYPE payment_status_v2 RENAME TO payment_status;
DROP TYPE payment_status_old;

ALTER TABLE payment.payment_entries
    ADD COLUMN method payment_method NOT NULL DEFAULT 'manual',
    ADD COLUMN provider_txn_id UUID;

CREATE INDEX idx_payment_entries_provider_txn ON payment.payment_entries (provider_txn_id)
    WHERE provider_txn_id IS NOT NULL;

ALTER TABLE payment.payment_allocations
    ADD COLUMN discount_amount NUMERIC(18, 2) NOT NULL DEFAULT 0 CHECK (discount_amount >= 0),
    ADD COLUMN discount_account_id UUID;

-- The method column backfills everything to 'manual' by default; a deployment
-- that can distinguish historical channels from mode_of_payments may refine it
-- with its own UPDATE before the drift consumer first runs.
