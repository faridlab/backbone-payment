-- Down: drop enum types for payment module
DROP TYPE IF EXISTS settlement_kind CASCADE;
DROP TYPE IF EXISTS gl_posting_state CASCADE;
DROP TYPE IF EXISTS payment_status CASCADE;
DROP TYPE IF EXISTS payment_party_type CASCADE;
DROP TYPE IF EXISTS payment_type CASCADE;
DROP TYPE IF EXISTS mode_type CASCADE;
