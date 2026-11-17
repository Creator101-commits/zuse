-- Keep archived computers' deletion due times within the seven-day retention.
--
-- Migration 0050 copied `archive_delete_at` into `machine_delete_due_at`. An
-- API process still on the previous release during a rolling deploy writes a
-- ten-day `archive_delete_at`, which would have delayed deletion by three
-- days. The due time is now capped at the archive request plus seven days
-- (zuse_protocol::cloud::ARCHIVE_RETENTION_MS), whatever deadline a writer
-- stores, so deploy ordering does not matter.
--
-- 0050's backfill also skipped archives still in flight (`archiving`): their
-- completion only changes `state`, so the trigger never set a due time. They
-- are backfilled here; deletion still requires `archived` state.
CREATE OR REPLACE FUNCTION zuse_cloud_archive_machine_due() RETURNS trigger
LANGUAGE plpgsql AS $body$
DECLARE deadline bigint;
BEGIN
    -- LEAST ignores NULL: without a request time the stored deadline stands.
    deadline := CASE WHEN NEW.archive_delete_at IS NULL THEN NULL
                     ELSE LEAST(NEW.archive_delete_at, NEW.archive_requested_at + 604800000) END;
    IF TG_OP = 'INSERT' THEN
        NEW.machine_delete_due_at := COALESCE(NEW.machine_delete_due_at, deadline);
    ELSIF NEW.archive_delete_at IS DISTINCT FROM OLD.archive_delete_at
       OR NEW.archive_requested_at IS DISTINCT FROM OLD.archive_requested_at THEN
        NEW.machine_delete_due_at := deadline;
    END IF;
    RETURN NEW;
END $body$;

DROP TRIGGER IF EXISTS zuse_cloud_archive_machine_due ON api_cloud_workspaces;
CREATE TRIGGER zuse_cloud_archive_machine_due
BEFORE INSERT OR UPDATE OF archive_delete_at, archive_requested_at ON api_cloud_workspaces
FOR EACH ROW EXECUTE FUNCTION zuse_cloud_archive_machine_due();

-- Existing due times past the cap, and archives still being completed.
UPDATE api_cloud_workspaces
   SET machine_delete_due_at = LEAST(COALESCE(machine_delete_due_at, archive_delete_at), archive_requested_at + 604800000)
 WHERE desired_state = 'archived' AND state IN ('archived', 'archiving')
   AND provider_sandbox_id IS NOT NULL AND archive_delete_at IS NOT NULL
   AND (machine_delete_due_at IS NULL
        OR machine_delete_due_at > archive_requested_at + 604800000);
