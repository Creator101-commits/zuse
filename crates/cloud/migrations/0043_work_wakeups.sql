-- Wake sleeping workers when work is created or rescheduled.
--
-- Each trigger sends NOTIFY on channel 'zuse_cloud_work:<schema>' with a
-- short payload naming the kind of work. Staging and production share one
-- database, so the schema in the channel name keeps them from waking each
-- other. pg_notify is transactional: listeners hear it only after commit,
-- and identical notifications in one transaction are delivered once.
--
-- NOTIFY is only a latency hint. Workers also sleep until the next known due
-- time and poll on an idle timer, so a lost notification (or a listener
-- that is disabled or reconnecting) only delays work, never loses it.
CREATE OR REPLACE FUNCTION zuse_cloud_notify_work() RETURNS trigger
LANGUAGE plpgsql AS $body$
BEGIN
    PERFORM pg_notify(left('zuse_cloud_work:' || TG_TABLE_SCHEMA, 63), TG_ARGV[0]);
    RETURN NULL;
END $body$;

-- New or retried jobs. Claims (state 'running') and completions stay silent,
-- so a worker never wakes itself by claiming.
CREATE TRIGGER zuse_cloud_jobs_wake
AFTER INSERT OR UPDATE OF state, due_at ON zuse_rust_jobs
FOR EACH ROW WHEN (NEW.state = 'pending')
EXECUTE FUNCTION zuse_cloud_notify_work('jobs');

CREATE TRIGGER zuse_cloud_task_drains_wake
AFTER INSERT OR UPDATE OF next_attempt_at ON zuse_rust_task_command_drains
FOR EACH ROW EXECUTE FUNCTION zuse_cloud_notify_work('task_drain');

CREATE TRIGGER zuse_cloud_recreations_wake
AFTER INSERT OR UPDATE OF phase, due_at ON zuse_rust_computer_recreations
FOR EACH ROW WHEN (NEW.phase NOT IN ('ready', 'cancelled', 'attention-required'))
EXECUTE FUNCTION zuse_cloud_notify_work('recreation');

CREATE TRIGGER zuse_cloud_machine_images_wake
AFTER INSERT OR UPDATE OF state, next_attempt_at ON cloud_machine_images
FOR EACH ROW WHEN (NEW.state IN ('queued', 'building', 'cleaning', 'retiring'))
EXECUTE FUNCTION zuse_cloud_notify_work('machine_images');

CREATE TRIGGER zuse_cloud_account_events_wake
AFTER INSERT ON zuse_rust_account_event_outbox
FOR EACH ROW EXECUTE FUNCTION zuse_cloud_notify_work('account_events');

-- Every command mailbox change bumps its revision. The API process holding a
-- guest's command long-poll learns of commands committed by the worker.
CREATE TRIGGER zuse_cloud_mailboxes_wake
AFTER UPDATE OF revision ON zuse_rust_mailboxes
FOR EACH ROW EXECUTE FUNCTION zuse_cloud_notify_work('mailbox');

-- Project environment caches are an optional migration (0028). When it is
-- installed later, those workers still run on due time and the idle timer.
DO $optional$
BEGIN
 IF to_regclass('zuse_rust_project_preparation_requests') IS NOT NULL THEN
  CREATE TRIGGER zuse_cloud_project_preparation_wake
  AFTER INSERT OR UPDATE OF state, next_attempt_at ON zuse_rust_project_preparation_requests
  FOR EACH ROW WHEN (NEW.state IN ('queued', 'resolving', 'preparing', 'blocked'))
  EXECUTE FUNCTION zuse_cloud_notify_work('project_cache');
 END IF;
 IF to_regclass('zuse_rust_project_cache_operations') IS NOT NULL THEN
  CREATE TRIGGER zuse_cloud_project_cache_operations_wake
  AFTER INSERT OR UPDATE OF state, next_attempt_at ON zuse_rust_project_cache_operations
  FOR EACH ROW WHEN (NEW.state IN ('queued', 'running', 'uncertain', 'cleaning'))
  EXECUTE FUNCTION zuse_cloud_notify_work('project_cache');
 END IF;
END $optional$;
