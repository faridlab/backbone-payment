-- Hand-authored (user-owned). Not regenerated.
--
-- Strip every company-fence artifact from the payment tables (ADR-0029): the module is
-- tenant-agnostic; org scoping is installed by the COMPOSING service's tenancy decorator,
-- never by the module. Dropped here, per table: the company-leading indexes, the
-- <table>_company_isolation RLS policy, and the company_id column itself. The surviving
-- uniques (the payment-number unique, the dunning-action per-invoice-level unique, the
-- mode-of-payment code unique) were never company-leading and are untouched.
--
-- Ordering guard (the decorator must run FIRST on any database with data): the module
-- never moves tenancy data. A table is safe to strip when EITHER
--   a) it carries org_unit_id with no NULLs — the decorator backfilled it from company_id —
--      or b) it is empty (a fresh database: the earlier chain files created it empty).
-- Otherwise the strip RAISEs, naming the decorator step, rather than dropping a column
-- that still holds the only tenancy key. The file is re-runnable (every drop is IF EXISTS
-- and the tracker has no checksums), so a failed run retries cleanly after the decorator
-- lands.
--
-- RLS enable/force flags are deliberately NOT touched: the decorator owns those now.
--
-- The per-(unit, as_of_date, direction) aging-snapshot unique is NOT restored at module
-- level in any form: its tenant-free global shape would forbid two companies of one
-- tenant from aging on the same date. The per-unit unique arrives with the composing
-- decorator's tenancy declaration; until then the dunning service's snapshot upsert
-- arbitrates idempotency with a scoped SELECT inside its transaction.

DO $$
DECLARE
    t text;
    has_org boolean;
    org_nulls bigint;
    total bigint;
    offenders text := '';
BEGIN
    FOREACH t IN ARRAY ARRAY[
        'payment_entries', 'payment_allocations',
        'aging_snapshots', 'aging_buckets',
        'dunning_runs', 'dunning_actions'
    ]
    LOOP
        IF to_regclass(format('payment.%I', t)) IS NULL THEN
            CONTINUE; -- chain not fully applied on this database; nothing to strip
        END IF;

        SELECT EXISTS (
                   SELECT 1 FROM information_schema.columns
                   WHERE table_schema = 'payment' AND table_name = t AND column_name = 'org_unit_id'
               )
        INTO has_org;

        EXECUTE format('SELECT count(*) FROM payment.%I', t) INTO total;

        IF has_org THEN
            EXECUTE format(
                'SELECT count(*) FROM payment.%I WHERE org_unit_id IS NULL', t)
            INTO org_nulls;
        ELSE
            org_nulls := total; -- no org column: every row's only tenancy key is company_id
        END IF;

        IF has_org AND org_nulls = 0 THEN
            CONTINUE; -- decorator backfilled: safe
        END IF;
        IF total = 0 THEN
            CONTINUE; -- empty table (fresh database): safe
        END IF;
        offenders := offenders || format(' payment.%s (%s rows, %s rows not covered by org_unit_id);', t, total, org_nulls);
    END LOOP;

    IF offenders <> '' THEN
        RAISE EXCEPTION 'refusing to strip company_id — these tables are not yet covered by the tenancy decorator:%. Apply the composing service''s tenancy decorator (it backfills org_unit_id from company_id) and re-run; it is the only step that moves tenancy data.', offenders;
    END IF;
END $$;

-- ── payment_entries ────────────────────────────────────────────────────────────
DROP INDEX IF EXISTS payment.idx_payment_entries_company_id_party_type_party_id_status;
DROP POLICY IF EXISTS payment_entries_company_isolation ON payment.payment_entries;
ALTER TABLE payment.payment_entries DROP COLUMN IF EXISTS company_id;

-- ── payment_allocations ────────────────────────────────────────────────────────
DROP INDEX IF EXISTS payment.idx_payment_allocations_company_id;
DROP POLICY IF EXISTS payment_allocations_company_isolation ON payment.payment_allocations;
ALTER TABLE payment.payment_allocations DROP COLUMN IF EXISTS company_id;

-- ── aging_snapshots ────────────────────────────────────────────────────────────
DROP INDEX IF EXISTS payment.idx_aging_snapshots_company_id_as_of_date_direction;
DROP INDEX IF EXISTS payment.idx_aging_snapshots_company_id_as_of_date;
DROP POLICY IF EXISTS aging_snapshots_company_isolation ON payment.aging_snapshots;
ALTER TABLE payment.aging_snapshots DROP COLUMN IF EXISTS company_id;

-- ── aging_buckets ──────────────────────────────────────────────────────────────
DROP INDEX IF EXISTS payment.idx_aging_buckets_company_id_party_id_bucket;
DROP POLICY IF EXISTS aging_buckets_company_isolation ON payment.aging_buckets;
ALTER TABLE payment.aging_buckets DROP COLUMN IF EXISTS company_id;

-- ── dunning_runs ───────────────────────────────────────────────────────────────
DROP INDEX IF EXISTS payment.idx_dunning_runs_company_id_as_of_date;
DROP POLICY IF EXISTS dunning_runs_company_isolation ON payment.dunning_runs;
ALTER TABLE payment.dunning_runs DROP COLUMN IF EXISTS company_id;

-- ── dunning_actions ────────────────────────────────────────────────────────────
DROP INDEX IF EXISTS payment.idx_dunning_actions_company_id_party_id_status;
DROP POLICY IF EXISTS dunning_actions_company_isolation ON payment.dunning_actions;
ALTER TABLE payment.dunning_actions DROP COLUMN IF EXISTS company_id;
