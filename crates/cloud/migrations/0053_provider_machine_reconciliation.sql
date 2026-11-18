-- Compare what each provider says is running with what this environment's
-- database expects, and alert on unexpected compute spend.
--
-- Staging and production share one provider organization, and the provider
-- has no machine labels. A machine belongs to this environment only when this
-- schema has recorded its id or name. Allocation names are random per
-- operation and are written before the machine is created, so a record here is
-- proof of ownership that the other environment cannot share.
--
-- `zuse_rust_machine_ledger` keeps those records after a workspace is deleted
-- (deletion purges the allocation and restore rows), so a machine left behind
-- by a deleted workspace is still recognised as ours. Triggers record every
-- machine name and id as it is written; existing rows are backfilled below.
CREATE TABLE IF NOT EXISTS zuse_rust_machine_ledger (
    machine_key text PRIMARY KEY CHECK (machine_key ~ '^(id|name):.+$'),
    workspace_id text,
    source text NOT NULL,
    recorded_at bigint NOT NULL
);

-- The ledger is bookkeeping: it runs as the schema owner (so the application
-- role needs no new grant for its own writes to keep working) and a failure
-- here never fails the allocation or workspace write that fired it.
CREATE OR REPLACE FUNCTION zuse_cloud_machine_ledger_operation() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path FROM CURRENT AS $body$
DECLARE at bigint := (extract(epoch from clock_timestamp()) * 1000)::bigint;
BEGIN
    BEGIN
        IF NEW.machine_name IS NOT NULL AND NEW.machine_name <> '' THEN
            INSERT INTO zuse_rust_machine_ledger VALUES ('name:' || NEW.machine_name, NEW.workspace_id, TG_TABLE_NAME, at)
            ON CONFLICT (machine_key) DO NOTHING;
        END IF;
        IF NEW.machine_id IS NOT NULL AND NEW.machine_id <> '' THEN
            INSERT INTO zuse_rust_machine_ledger VALUES ('id:' || NEW.machine_id, NEW.workspace_id, TG_TABLE_NAME, at)
            ON CONFLICT (machine_key) DO NOTHING;
        END IF;
    EXCEPTION WHEN OTHERS THEN
        RAISE WARNING 'zuse_machine_ledger_unavailable: %', SQLSTATE;
    END;
    RETURN NULL;
END $body$;

CREATE OR REPLACE FUNCTION zuse_cloud_machine_ledger_workspace() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path FROM CURRENT AS $body$
BEGIN
    BEGIN
        INSERT INTO zuse_rust_machine_ledger
        VALUES ('id:' || NEW.provider_sandbox_id, NEW.workspace_id, TG_TABLE_NAME,
                (extract(epoch from clock_timestamp()) * 1000)::bigint)
        ON CONFLICT (machine_key) DO NOTHING;
    EXCEPTION WHEN OTHERS THEN
        RAISE WARNING 'zuse_machine_ledger_unavailable: %', SQLSTATE;
    END;
    RETURN NULL;
END $body$;

-- Project build machines belong to an account, not a workspace.
CREATE OR REPLACE FUNCTION zuse_cloud_machine_ledger_build() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path FROM CURRENT AS $body$
BEGIN
    BEGIN
        INSERT INTO zuse_rust_machine_ledger
        VALUES ('id:' || NEW.provider_sandbox_id, NULL, TG_TABLE_NAME,
                (extract(epoch from clock_timestamp()) * 1000)::bigint)
        ON CONFLICT (machine_key) DO NOTHING;
    EXCEPTION WHEN OTHERS THEN
        RAISE WARNING 'zuse_machine_ledger_unavailable: %', SQLSTATE;
    END;
    RETURN NULL;
END $body$;

DROP TRIGGER IF EXISTS zuse_cloud_machine_ledger ON zuse_rust_allocations;
CREATE TRIGGER zuse_cloud_machine_ledger
AFTER INSERT OR UPDATE OF machine_name, machine_id ON zuse_rust_allocations
FOR EACH ROW EXECUTE FUNCTION zuse_cloud_machine_ledger_operation();

DROP TRIGGER IF EXISTS zuse_cloud_machine_ledger ON zuse_rust_restores;
CREATE TRIGGER zuse_cloud_machine_ledger
AFTER INSERT OR UPDATE OF machine_name, machine_id ON zuse_rust_restores
FOR EACH ROW EXECUTE FUNCTION zuse_cloud_machine_ledger_operation();

DROP TRIGGER IF EXISTS zuse_cloud_machine_ledger ON zuse_rust_provider_creates;
CREATE TRIGGER zuse_cloud_machine_ledger
AFTER INSERT OR UPDATE OF machine_name, machine_id ON zuse_rust_provider_creates
FOR EACH ROW EXECUTE FUNCTION zuse_cloud_machine_ledger_operation();

DROP TRIGGER IF EXISTS zuse_cloud_machine_ledger ON api_cloud_workspaces;
CREATE TRIGGER zuse_cloud_machine_ledger
AFTER INSERT OR UPDATE OF provider_sandbox_id ON api_cloud_workspaces
FOR EACH ROW WHEN (NEW.provider_sandbox_id IS NOT NULL AND NEW.provider_sandbox_id <> '')
EXECUTE FUNCTION zuse_cloud_machine_ledger_workspace();

DROP TRIGGER IF EXISTS zuse_cloud_machine_ledger ON api_cloud_project_builds;
CREATE TRIGGER zuse_cloud_machine_ledger
AFTER INSERT OR UPDATE OF provider_sandbox_id ON api_cloud_project_builds
FOR EACH ROW WHEN (NEW.provider_sandbox_id IS NOT NULL AND NEW.provider_sandbox_id <> '')
EXECUTE FUNCTION zuse_cloud_machine_ledger_build();

INSERT INTO zuse_rust_machine_ledger
SELECT key, workspace_id, source, at FROM (
    SELECT 'name:' || machine_name AS key, workspace_id, 'zuse_rust_allocations' AS source, created_at AS at FROM zuse_rust_allocations WHERE machine_name <> ''
    UNION ALL SELECT 'id:' || machine_id, workspace_id, 'zuse_rust_allocations', created_at FROM zuse_rust_allocations WHERE machine_id <> ''
    UNION ALL SELECT 'name:' || machine_name, workspace_id, 'zuse_rust_restores', created_at FROM zuse_rust_restores WHERE machine_name <> ''
    UNION ALL SELECT 'id:' || machine_id, workspace_id, 'zuse_rust_restores', created_at FROM zuse_rust_restores WHERE machine_id <> ''
    UNION ALL SELECT 'name:' || machine_name, workspace_id, 'zuse_rust_provider_creates', updated_at FROM zuse_rust_provider_creates WHERE machine_name <> ''
    UNION ALL SELECT 'id:' || machine_id, workspace_id, 'zuse_rust_provider_creates', updated_at FROM zuse_rust_provider_creates WHERE machine_id <> ''
    UNION ALL SELECT 'id:' || provider_sandbox_id, workspace_id, 'api_cloud_workspaces', created_at FROM api_cloud_workspaces WHERE provider_sandbox_id <> ''
    UNION ALL SELECT 'id:' || provider_sandbox_id, NULL, 'api_cloud_project_builds', created_at FROM api_cloud_project_builds WHERE provider_sandbox_id <> ''
) recorded
ON CONFLICT (machine_key) DO NOTHING;

-- When each reconciliation task next runs. The worker's due-time query reads
-- `next_run_at`, so it sleeps until then instead of polling; a lease keeps
-- two workers from running the same pass.
CREATE TABLE IF NOT EXISTS zuse_rust_provider_reconciliation_schedule (
    task text PRIMARY KEY CHECK (task IN ('provider-machines', 'cost-anomalies')),
    next_run_at bigint NOT NULL,
    lease_token text,
    lease_expires_at bigint,
    last_completed_at bigint,
    last_summary jsonb,
    updated_at bigint NOT NULL
);
INSERT INTO zuse_rust_provider_reconciliation_schedule (task, next_run_at, updated_at)
VALUES ('provider-machines', 0, 0), ('cost-anomalies', 0, 0)
ON CONFLICT (task) DO NOTHING;

-- Machines whose provider state disagrees with this database, with when that
-- was first seen. Actions wait until the same verdict has held for a
-- confirmation window (deletion: a longer safety window).
CREATE TABLE IF NOT EXISTS zuse_rust_provider_machine_observations (
    provider text NOT NULL,
    machine_id text NOT NULL,
    machine_name text NOT NULL,
    verdict text NOT NULL,
    reason text NOT NULL,
    workspace_id text,
    first_seen_at bigint NOT NULL,
    last_seen_at bigint NOT NULL,
    last_action text,
    last_action_at bigint,
    PRIMARY KEY (provider, machine_id)
);

-- Operator-visible alerts: one row per (kind, subject), refreshed while the
-- condition holds and resolved when it clears. Every alert is also logged at
-- WARN.
CREATE TABLE IF NOT EXISTS zuse_rust_operator_alerts (
    kind text NOT NULL,
    subject text NOT NULL,
    summary text NOT NULL,
    details jsonb NOT NULL DEFAULT '{}'::jsonb,
    first_seen_at bigint NOT NULL,
    last_seen_at bigint NOT NULL,
    occurrences bigint NOT NULL DEFAULT 1,
    resolved_at bigint,
    PRIMARY KEY (kind, subject)
);
CREATE INDEX IF NOT EXISTS zuse_rust_operator_alerts_open_idx
  ON zuse_rust_operator_alerts (last_seen_at DESC) WHERE resolved_at IS NULL;

DO $grants$
DECLARE schema_name text := current_schema(); app_role text;
BEGIN
 app_role := CASE schema_name WHEN 'zuse_staging' THEN 'zuse_cloud_staging' WHEN 'zuse_production' THEN 'zuse_cloud_production' END;
 IF app_role IS NOT NULL AND EXISTS(SELECT 1 FROM pg_roles WHERE rolname=app_role) THEN
  EXECUTE format('GRANT SELECT,INSERT,UPDATE,DELETE ON TABLE %I.zuse_rust_machine_ledger TO %I',schema_name,app_role);
  EXECUTE format('GRANT SELECT,INSERT,UPDATE,DELETE ON TABLE %I.zuse_rust_provider_reconciliation_schedule TO %I',schema_name,app_role);
  EXECUTE format('GRANT SELECT,INSERT,UPDATE,DELETE ON TABLE %I.zuse_rust_provider_machine_observations TO %I',schema_name,app_role);
  EXECUTE format('GRANT SELECT,INSERT,UPDATE,DELETE ON TABLE %I.zuse_rust_operator_alerts TO %I',schema_name,app_role);
 END IF;
END $grants$;
