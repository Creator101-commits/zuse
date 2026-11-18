-- Earliest idle-sleep deadline: the same eligibility as idle_candidates.sql
-- without its time predicates, which become the deadline instead
-- ($2 = idle timeout + activity coalescing). Computers with a live command
-- have no deadline; finishing one records activity, which moves its deadline
-- at least a full idle timeout into the future. outcome-unknown is terminal
-- and counts like any finished command: its last write starts the window.
SELECT min(GREATEST(w.next_action_at, w.last_activity_at+$2, COALESCE(e.updated_at,0)+$2))::bigint
FROM api_cloud_workspaces w
JOIN zuse_rust_control_owners o
  ON o.resource_kind='workspace' AND o.resource_id=w.workspace_id
 AND o.backend='rust'
 AND o.generation=COALESCE((w.request_config->>'runtimeGeneration')::bigint,1)
LEFT JOIN LATERAL (SELECT max(x.updated_at) AS updated_at FROM zuse_rust_task_command_execution x WHERE x.workspace_id=w.workspace_id) e ON true
WHERE ($1::text[] IS NULL OR w.account_id=ANY($1))
  AND w.state='ready' AND w.desired_state='ready' AND w.runtime_state='online'
  AND w.request_config->>'commandDeliveryMode'=$3
  AND w.request_config->>'rustCommandAdmissionVersion'='2'
  AND w.request_config->>'rustCommandMailboxEnabled'='true'
  AND w.request_config->>'runtimeRpcProtocol'='zuse-rpc-v1'
  AND w.request_config#>>'{rustStorageEnrollment,phase}'='committed'
  AND w.request_config#>>'{runtimeBootstrapReceipt,workspaceId}'=w.workspace_id
  AND w.request_config#>>'{runtimeBootstrapReceipt,generation}'=COALESCE(w.request_config->>'runtimeGeneration','1')
  AND w.request_config#>>'{runtimeBootstrapReceipt,gatewayEpoch}'=COALESCE(w.request_config->>'gatewayEpoch',w.request_config->>'runtimeGeneration','1')
  AND w.request_config#>>'{rustStorageEnrollment,generation}'=COALESCE(w.request_config->>'runtimeGeneration','1')
  AND w.request_config#>>'{rustStorageEnrollment,sourceSandboxId}'=w.provider_sandbox_id
  AND w.request_config#>>'{rustStorageEnrollment,storageIncarnationId}'=w.request_config#>>'{rustRuntimeStorage,storageIncarnationId}'
  AND w.request_config#>>'{rustStorageEnrollment,credentialKeyThumbprint}'=w.request_config->>'runtimeCredentialKeyThumbprint'
  AND w.request_config#>>'{rustStorageEnrollment,signingKeyThumbprint}'=w.request_config->>'runtimeSigningKeyThumbprint'
  AND w.request_config#>'{rustStorageEnrollment,capabilities}' @> '["rust-workspace-lifecycle-v1","rust-session-commands-v1","rust-task-command-admission-v2"]'::jsonb
  AND COALESCE(w.request_config->>'runtimeReleaseChangeRequested','false')<>'true'
  AND COALESCE(w.request_config#>>'{runtimeActivation,phase}','') NOT IN ('preparing','launching','confirming','rolling-back')
  AND (w.request_config#>>'{rustRecreation,operationId}' IS NULL OR w.request_config#>>'{rustRecreation,phase}' IN ('ready','cancelled'))
  AND COALESCE(w.request_config#>>'{rustLifecycleOperation,phase}','ready')='ready'
  AND NOT EXISTS(SELECT 1 FROM api_cloud_workspace_launch_intents i WHERE i.workspace_id=w.workspace_id)
  AND NOT EXISTS(
    SELECT 1 FROM zuse_rust_task_command_execution e WHERE e.workspace_id=w.workspace_id
      AND e.state IN('queued','starting','running','waiting-input')
  )
