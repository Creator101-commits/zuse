-- Tell API processes that plugin executor rows changed.
--
-- The plugin executor keeps the rows each tool invocation reads (apps and
-- deployments, profiles, accounts, OAuth grants) in memory. Any insert,
-- update or delete in an `executor_*` table sends NOTIFY 'plugins' on channel
-- 'zuse_cloud_work:<schema>' (function from 0043), like 0046 does for plugin
-- connections. `serve` receives it on its LISTEN connection and passes a new
-- change epoch with its next executor request, which drops the executor's
-- kept rows; the executor opens no LISTEN connection of its own. The
-- executor keeps rows only when every `executor_*` table has this trigger.
--
-- The SDK creates these tables when `plugin-migrate` runs, which then runs
-- this file again. Bootstrap installs the triggers only on existing tables
-- that belong to its role. Safe to repeat.
DO $executor$
DECLARE item record;
BEGIN
 IF to_regprocedure('zuse_cloud_notify_work()') IS NULL THEN
  EXECUTE $create$
   CREATE FUNCTION zuse_cloud_notify_work() RETURNS trigger
   LANGUAGE plpgsql AS $body$
   BEGIN
       PERFORM pg_notify(left('zuse_cloud_work:' || TG_TABLE_SCHEMA, 63), TG_ARGV[0]);
       RETURN NULL;
   END $body$
  $create$;
 END IF;
 FOR item IN
  SELECT c.relname FROM pg_class c
  WHERE c.relnamespace=current_schema()::regnamespace AND c.relkind='r'
   AND left(c.relname,9)='executor_' AND pg_has_role(c.relowner,'USAGE')
 LOOP
  EXECUTE format('DROP TRIGGER IF EXISTS zuse_cloud_executor_wake ON %I', item.relname);
  EXECUTE format('CREATE TRIGGER zuse_cloud_executor_wake AFTER INSERT OR UPDATE OR DELETE ON %I FOR EACH ROW EXECUTE FUNCTION zuse_cloud_notify_work(%L)', item.relname, 'plugins');
 END LOOP;
END $executor$;
