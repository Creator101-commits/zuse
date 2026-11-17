-- Delete an archived chat's computer when its retention ends, without polling.
--
-- `machine_delete_due_at` is when the runtime worker should next try to retire
-- the provider computer of an archived workspace (back up its full state, then
-- delete the machine; the chat, transcript and project stay). The worker's
-- due-time query includes the earliest value, so it wakes exactly then.
--
-- Every writer of `archive_delete_at` (archive, re-archive, unarchive, delete)
-- keeps this column in step through the BEFORE trigger below, including API
-- processes still running the previous release during a rolling deploy. The
-- worker itself only pushes the value later (admission back-off), which never
-- touches `archive_delete_at`.
--
-- NOTIFY 'archive_retention' (function from 0043) is sent only when a due time
-- appears or moves earlier, so the worker's own back-off never wakes it.
ALTER TABLE api_cloud_workspaces ADD COLUMN IF NOT EXISTS machine_delete_due_at bigint;

-- Archived computers that are still retained: delete seven days after the
-- archive request (zuse_protocol::cloud::ARCHIVE_RETENTION_MS), or at the
-- stored deadline for legacy rows without a request time.
UPDATE api_cloud_workspaces
   SET machine_delete_due_at = COALESCE(archive_requested_at + 604800000, archive_delete_at)
 WHERE state = 'archived' AND desired_state = 'archived'
   AND provider_sandbox_id IS NOT NULL AND machine_delete_due_at IS NULL
   AND COALESCE(archive_requested_at, archive_delete_at) IS NOT NULL;

-- Matches the worker's due query and admission predicate exactly.
CREATE INDEX IF NOT EXISTS api_cloud_workspaces_machine_delete_due_idx
  ON api_cloud_workspaces (machine_delete_due_at)
  WHERE machine_delete_due_at IS NOT NULL AND state = 'archived'
    AND desired_state = 'archived' AND provider_sandbox_id IS NOT NULL;

CREATE OR REPLACE FUNCTION zuse_cloud_archive_machine_due() RETURNS trigger
LANGUAGE plpgsql AS $body$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.machine_delete_due_at := COALESCE(NEW.machine_delete_due_at, NEW.archive_delete_at);
    ELSIF NEW.archive_delete_at IS DISTINCT FROM OLD.archive_delete_at THEN
        NEW.machine_delete_due_at := NEW.archive_delete_at;
    END IF;
    RETURN NEW;
END $body$;

DROP TRIGGER IF EXISTS zuse_cloud_archive_machine_due ON api_cloud_workspaces;
CREATE TRIGGER zuse_cloud_archive_machine_due
BEFORE INSERT OR UPDATE OF archive_delete_at ON api_cloud_workspaces
FOR EACH ROW EXECUTE FUNCTION zuse_cloud_archive_machine_due();

DROP TRIGGER IF EXISTS zuse_cloud_archive_machine_due_wake ON api_cloud_workspaces;
CREATE TRIGGER zuse_cloud_archive_machine_due_wake
AFTER UPDATE OF machine_delete_due_at, archive_delete_at ON api_cloud_workspaces
FOR EACH ROW WHEN (NEW.machine_delete_due_at IS NOT NULL
    AND (OLD.machine_delete_due_at IS NULL OR NEW.machine_delete_due_at < OLD.machine_delete_due_at))
EXECUTE FUNCTION zuse_cloud_notify_work('archive_retention');

DROP TRIGGER IF EXISTS zuse_cloud_archive_machine_due_insert_wake ON api_cloud_workspaces;
CREATE TRIGGER zuse_cloud_archive_machine_due_insert_wake
AFTER INSERT ON api_cloud_workspaces
FOR EACH ROW WHEN (NEW.machine_delete_due_at IS NOT NULL)
EXECUTE FUNCTION zuse_cloud_notify_work('archive_retention');
