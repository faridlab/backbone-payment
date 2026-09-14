-- Payment modes — the settlement channels an Indonesian business actually uses.
--
-- The `mode_type` enum this column takes already names the Indonesian set,
-- QRIS and virtual account included, so these rows fill in the channels rather
-- than inventing a vocabulary.
--
-- `default_account_id` is left NULL on purpose: the GL account a channel settles
-- to belongs to a company's own chart, which the chart-install verb creates per
-- company. A tenant wires each mode to its account after installing a chart.

INSERT INTO payment.mode_of_payments (id, code, name, mode_type, default_account_id, status, metadata) VALUES
  ('9a000000-0000-4c30-8000-000000000001', 'CASH',    'Tunai',                      'cash',            NULL, 'active', '{}'::jsonb),
  ('9a000000-0000-4c30-8000-000000000002', 'TRANSFER','Transfer Bank',              'bank_transfer',   NULL, 'active', '{}'::jsonb),
  ('9a000000-0000-4c30-8000-000000000003', 'VA',      'Virtual Account',            'virtual_account', NULL, 'active', '{}'::jsonb),
  ('9a000000-0000-4c30-8000-000000000004', 'QRIS',    'QRIS',                       'qris',            NULL, 'active', '{}'::jsonb),
  ('9a000000-0000-4c30-8000-000000000005', 'DEBIT',   'Kartu Debit',                'card',            NULL, 'active', '{}'::jsonb),
  ('9a000000-0000-4c30-8000-000000000006', 'CREDIT',  'Kartu Kredit',               'card',            NULL, 'active', '{}'::jsonb),
  ('9a000000-0000-4c30-8000-000000000007', 'EWALLET', 'Dompet Digital',             'e_wallet',        NULL, 'active', '{}'::jsonb)
ON CONFLICT (id) DO NOTHING;
