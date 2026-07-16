-- Down: remove the company RLS fence for payment module

-- Reverse the company RLS fence for payment.payment_entries
DROP POLICY IF EXISTS payment_entries_company_isolation ON payment.payment_entries;
ALTER TABLE payment.payment_entries NO FORCE ROW LEVEL SECURITY;
ALTER TABLE payment.payment_entries DISABLE ROW LEVEL SECURITY;

