-- Tell API processes that plugin connections or custom servers changed.
--
-- `serve` keeps each workspace's plugin connections and custom server
-- definitions in memory so a tool call does not re-read them. Any insert,
-- update or delete of those rows sends NOTIFY 'plugins' on channel
-- 'zuse_cloud_work:<schema>' (function from 0043). Every listening process
-- then drops its cached plugin views; the process that wrote the row has
-- already dropped its own. pg_notify is transactional: listeners hear it only
-- after commit, and identical notifications in one transaction arrive once.
--
-- Unlike worker wakeups, these notifications keep a cache correct. A process
-- uses the cache only while its LISTEN connection is up and both triggers
-- exist; a reconnecting listener drops every cached view.
--
-- The plugin tables are created by `plugin-migrate` (0009_plugins.sql), which
-- runs this file again after creating them. Bootstrap installs the triggers
-- only when the tables already exist and belong to its role. Safe to repeat.
DO $plugins$
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
 IF EXISTS(SELECT 1 FROM pg_class WHERE oid=to_regclass('zuse_plugin_connections') AND pg_has_role(relowner,'USAGE')) THEN
  DROP TRIGGER IF EXISTS zuse_cloud_plugin_connections_wake ON zuse_plugin_connections;
  CREATE TRIGGER zuse_cloud_plugin_connections_wake
  AFTER INSERT OR UPDATE OR DELETE ON zuse_plugin_connections
  FOR EACH ROW EXECUTE FUNCTION zuse_cloud_notify_work('plugins');
 END IF;
 IF EXISTS(SELECT 1 FROM pg_class WHERE oid=to_regclass('zuse_plugin_servers') AND pg_has_role(relowner,'USAGE')) THEN
  DROP TRIGGER IF EXISTS zuse_cloud_plugin_servers_wake ON zuse_plugin_servers;
  CREATE TRIGGER zuse_cloud_plugin_servers_wake
  AFTER INSERT OR UPDATE OR DELETE ON zuse_plugin_servers
  FOR EACH ROW EXECUTE FUNCTION zuse_cloud_notify_work('plugins');
 END IF;
END $plugins$;
