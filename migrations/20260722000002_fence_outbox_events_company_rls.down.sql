DROP POLICY IF EXISTS outbox_events_company_isolation ON payment.outbox_events;
ALTER TABLE payment.outbox_events NO FORCE ROW LEVEL SECURITY;
ALTER TABLE payment.outbox_events DISABLE ROW LEVEL SECURITY;
DROP INDEX IF EXISTS payment.idx_payment_outbox_company_id;
ALTER TABLE payment.outbox_events DROP COLUMN IF EXISTS company_id;
