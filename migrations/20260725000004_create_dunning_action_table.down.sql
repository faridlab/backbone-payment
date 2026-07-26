-- Down: drop payment.dunning_actions table
DROP TABLE IF EXISTS payment.dunning_actions CASCADE;
DROP FUNCTION IF EXISTS payment.dunning_actions_audit_timestamp() CASCADE;
