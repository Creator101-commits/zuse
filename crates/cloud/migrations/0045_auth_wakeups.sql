-- Wake the credential worker (auth-worker) when sign-in work appears, so it
-- can sleep until NOTIFY or its next known deadline instead of polling.
--
-- Reuses zuse_cloud_notify_work() from 0043_work_wakeups.sql with payload
-- 'auth'. As there, NOTIFY is only a latency hint: the worker also sleeps
-- until its next known deadline and runs an idle safety pass, so a lost
-- notification only delays work, never loses it.

-- A sign-in (queued) or a credential proposal waiting for verification
-- became claimable. The worker's own claim sets a fresh lease_id and so
-- stays silent; heartbeats only touch lease_expires_at.
DROP TRIGGER IF EXISTS zuse_cloud_auth_operations_wake ON cloud_auth_operations;
CREATE TRIGGER zuse_cloud_auth_operations_wake
AFTER INSERT OR UPDATE OF state, lease_id ON cloud_auth_operations
FOR EACH ROW WHEN (NEW.state = 'queued' OR (NEW.state = 'authorizing' AND NEW.lease_id = 'credential-proposal-v1'))
EXECUTE FUNCTION zuse_cloud_notify_work('auth');

-- A refresh started: the worker learns the dispatch lease deadline after
-- which an unfinished refresh becomes uncertain. A checkpointed result needs
-- no worker action while its connection still matches.
DROP TRIGGER IF EXISTS zuse_cloud_credential_rotations_wake ON zuse_rust_credential_rotations;
CREATE TRIGGER zuse_cloud_credential_rotations_wake
AFTER INSERT ON zuse_rust_credential_rotations
FOR EACH ROW EXECUTE FUNCTION zuse_cloud_notify_work('auth');

-- A connection that held a refresh lease left 'connected' (disconnect or
-- reconnect-required): a checkpointed refresh result for it is now stale and
-- the worker discards it.
DROP TRIGGER IF EXISTS zuse_cloud_agent_connections_wake ON cloud_agent_connections;
CREATE TRIGGER zuse_cloud_agent_connections_wake
AFTER UPDATE OF state ON cloud_agent_connections
FOR EACH ROW WHEN (OLD.refresh_lease_id IS NOT NULL AND OLD.state = 'connected' AND NEW.state <> 'connected')
EXECUTE FUNCTION zuse_cloud_notify_work('auth');

-- The worker's due-time query looks up pending connections and the
-- operations of a connection without scanning either table.
CREATE INDEX IF NOT EXISTS cloud_agent_connections_pending_idx ON cloud_agent_connections(connection_id) WHERE state = 'pending';
CREATE INDEX IF NOT EXISTS cloud_auth_operations_connection_idx ON cloud_auth_operations(connection_id);
