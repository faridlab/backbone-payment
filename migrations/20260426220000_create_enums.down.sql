-- Down: drop enum types for payment module
DROP TYPE IF EXISTS withholding_tax_type CASCADE;
DROP TYPE IF EXISTS settlement_kind CASCADE;
DROP TYPE IF EXISTS gl_posting_state CASCADE;
DROP TYPE IF EXISTS payment_status CASCADE;
DROP TYPE IF EXISTS payment_party_type CASCADE;
DROP TYPE IF EXISTS payment_type CASCADE;
DROP TYPE IF EXISTS mode_type CASCADE;
DROP TYPE IF EXISTS dunning_run_status CASCADE;
DROP TYPE IF EXISTS snapshot_status CASCADE;
DROP TYPE IF EXISTS dunning_action_status CASCADE;
DROP TYPE IF EXISTS dunning_action_type CASCADE;
DROP TYPE IF EXISTS dunning_level CASCADE;
DROP TYPE IF EXISTS aging_bucket_name CASCADE;
