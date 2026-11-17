-- The worker's due-time query takes min(due/lease) per job kind over pending
-- and running jobs only. Completed and failed jobs are retained, so without a
-- partial index that aggregate visits every historical row of those kinds.
CREATE INDEX IF NOT EXISTS zuse_rust_jobs_active_due_idx
  ON zuse_rust_jobs (kind, due_at, lease_expires_at)
  WHERE state IN ('pending', 'running');
