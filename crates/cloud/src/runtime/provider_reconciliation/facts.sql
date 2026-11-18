-- What this environment's database records about each listed provider machine.
-- $1 machine ids, $2 machine names (same order; '' when unknown), $3 provider,
-- $4 now (epoch ms), $5 quiet cutoff: work older than this is not "in flight".
-- `/*LEDGER*/` is replaced with the ledger lookup once migration 0053 exists.
WITH m AS (
  SELECT t.id, t.name, t.n FROM unnest($1::text[], $2::text[]) WITH ORDINALITY AS t(id, name, n)
)
SELECT m.n::bigint AS n, jsonb_strip_nulls(jsonb_build_object(
  'workspace', (
    SELECT jsonb_build_object(
      'id', w.workspace_id, 'account', w.account_id, 'state', w.state,
      'desired', w.desired_state, 'runtime', w.runtime_state,
      'lastActivityAt', w.last_activity_at, 'updatedAt', w.updated_at,
      'lifecyclePhase', w.request_config#>>'{rustLifecycleOperation,phase}',
      'liveLease', EXISTS(SELECT 1 FROM zuse_rust_gateway_leases l WHERE l.workspace_id = w.workspace_id AND l.expires_at > $4),
      'launchIntent', EXISTS(SELECT 1 FROM api_cloud_workspace_launch_intents i WHERE i.workspace_id = w.workspace_id),
      'activeTurn', EXISTS(SELECT 1 FROM zuse_rust_task_command_execution e WHERE e.workspace_id = w.workspace_id AND e.state IN ('queued','starting','running','waiting-input')),
      'lastTurnAt', (SELECT max(e.updated_at) FROM zuse_rust_task_command_execution e WHERE e.workspace_id = w.workspace_id),
      'pendingJob', EXISTS(SELECT 1 FROM zuse_rust_jobs j WHERE j.resource_id = w.workspace_id AND j.state IN ('pending','running') AND j.updated_at > $5))
    FROM api_cloud_workspaces w
    WHERE w.provider_sandbox_id = m.id AND w.provider = $3
      AND w.deleted_at IS NULL AND w.state <> 'deleted'
    ORDER BY w.updated_at DESC LIMIT 1),
  'inFlight', CASE
    WHEN EXISTS(SELECT 1 FROM zuse_rust_allocations a WHERE (a.machine_id = m.id OR (m.name <> '' AND a.machine_name = m.name)) AND a.phase <> 'ready' AND a.updated_at > $5) THEN 'allocation'
    WHEN EXISTS(SELECT 1 FROM zuse_rust_restores r WHERE (r.machine_id = m.id OR (m.name <> '' AND r.machine_name = m.name)) AND r.phase <> 'ready' AND r.updated_at > $5) THEN 'restore'
    WHEN EXISTS(SELECT 1 FROM zuse_rust_recovery_operations r WHERE r.source_machine_id = m.id AND r.phase IN ('uploading','verifying','retiring','restoring')) THEN 'archive-backup'
    WHEN EXISTS(SELECT 1 FROM zuse_rust_computer_recreations r WHERE r.source_machine_id = m.id AND r.phase NOT IN ('ready','attention-required','cancelled') AND r.updated_at > $5) THEN 'recreation'
    WHEN EXISTS(SELECT 1 FROM zuse_rust_runtime_repairs r WHERE r.source_machine_id = m.id AND r.phase NOT IN ('ready','failed','attention-required') AND r.updated_at > $5) THEN 'runtime-repair'
    WHEN EXISTS(SELECT 1 FROM zuse_rust_lifecycle_operations o WHERE o.source_machine_id = m.id AND o.phase NOT IN ('paused','ready') AND o.updated_at > $5) THEN 'lifecycle'
    WHEN EXISTS(SELECT 1 FROM cloud_machine_images i WHERE i.state IN ('queued','building','cleaning','retiring') AND (strpos(i.plan::text, m.id) > 0 OR strpos(COALESCE(i.resources::text, ''), m.id) > 0 OR (m.name <> '' AND strpos(i.plan::text, m.name) > 0))) THEN 'machine-image'
    WHEN EXISTS(SELECT 1 FROM zuse_rust_image_operations i WHERE i.record->>'phase' IS DISTINCT FROM 'complete' AND i.updated_at > $5 AND (strpos(i.record::text, m.id) > 0 OR (m.name <> '' AND strpos(i.record::text, m.name) > 0))) THEN 'image-build'
    WHEN EXISTS(SELECT 1 FROM api_cloud_project_builds b WHERE b.provider_sandbox_id = m.id AND b.state NOT IN ('ready','failed','cancelled')) THEN 'project-build'
  END,
  'infrastructure', CASE
    WHEN EXISTS(SELECT 1 FROM api_cloud_auth_authorities a WHERE a.provider_sandbox_id = m.id) THEN 'auth-authority'
    WHEN EXISTS(SELECT 1 FROM cloud_computers c WHERE c.provider_resource_id = m.id) THEN 'computer'
    WHEN EXISTS(SELECT 1 FROM api_machines x WHERE x.provider_server_id = m.id) THEN 'machine'
  END,
  'owner', (
    SELECT jsonb_build_object(
      'workspaceId', h.workspace_id, 'source', h.source,
      'status', CASE WHEN h.workspace_id IS NULL THEN 'none'
                     WHEN ow.workspace_id IS NULL THEN 'missing'
                     WHEN ow.deleted_at IS NOT NULL OR ow.state = 'deleted' OR ow.desired_state = 'deleted' THEN 'deleted'
                     ELSE 'alive' END,
      'account', ow.account_id, 'lastActivityAt', ow.last_activity_at,
      'currentMachine', ow.provider_sandbox_id)
    FROM (
      SELECT x.workspace_id, x.source FROM (
        SELECT a.workspace_id, 'allocation' AS source, 1 AS rank FROM zuse_rust_allocations a WHERE a.machine_id = m.id OR (m.name <> '' AND a.machine_name = m.name)
        UNION ALL SELECT r.workspace_id, 'restore', 1 FROM zuse_rust_restores r WHERE r.machine_id = m.id OR (m.name <> '' AND r.machine_name = m.name)
        UNION ALL SELECT c.workspace_id, 'provider-create', 1 FROM zuse_rust_provider_creates c WHERE c.machine_id = m.id OR (m.name <> '' AND c.machine_name = m.name)
        UNION ALL SELECT o.workspace_id, 'lifecycle', 2 FROM zuse_rust_lifecycle_operations o WHERE o.source_machine_id = m.id
        UNION ALL SELECT r.workspace_id, 'archive-backup', 2 FROM zuse_rust_recovery_operations r WHERE r.source_machine_id = m.id
        UNION ALL SELECT r.workspace_id, 'recreation', 2 FROM zuse_rust_computer_recreations r WHERE r.source_machine_id = m.id
        /*LEDGER*/
      ) x ORDER BY x.workspace_id IS NULL, x.rank LIMIT 1
    ) h
    LEFT JOIN api_cloud_workspaces ow ON ow.workspace_id = h.workspace_id),
  'builder', (
    SELECT k.kind FROM (
      SELECT 'machine-image' AS kind FROM cloud_machine_images i WHERE strpos(i.plan::text, m.id) > 0 OR strpos(COALESCE(i.resources::text, ''), m.id) > 0 OR (m.name <> '' AND strpos(i.plan::text, m.name) > 0)
      UNION ALL SELECT 'image-build' FROM zuse_rust_image_operations i WHERE strpos(i.record::text, m.id) > 0 OR (m.name <> '' AND strpos(i.record::text, m.name) > 0)
      UNION ALL SELECT 'project-build' FROM api_cloud_project_builds b WHERE b.provider_sandbox_id = m.id
    ) k LIMIT 1)
)) AS facts
FROM m ORDER BY m.n
