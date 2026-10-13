-- Down: drop the declared foreign keys for payment module
ALTER TABLE IF EXISTS payment.payment_allocations DROP CONSTRAINT IF EXISTS fk_payment_allocations_payment_id;
ALTER TABLE IF EXISTS payment.dunning_actions DROP CONSTRAINT IF EXISTS fk_dunning_actions_run_id;
ALTER TABLE IF EXISTS payment.aging_buckets DROP CONSTRAINT IF EXISTS fk_aging_buckets_snapshot_id;
