//! Boat first start, waited for inside the computer.
//!
//! Boat attaches the image lazily: for several seconds after create its lazy
//! filesystem shows /usr/local owned by the provider's exec user, and the
//! fresh boot loader refuses (`fresh_boot_loader_path_unsafe`). Instead of
//! asking again from here, the worker sends one command right after create
//! (`infra/cloud/managed-base-fresh-start.py`, as root). Inside the computer
//! it moves the uploaded credential into a root-only tmpfs directory, waits
//! for the loader's path rule, runs the unchanged loader and helper, accepts
//! only a ready receipt, and launches the runtime.
//!
//! The worker then waits for the runtime's own enrollment, which pulls this
//! job forward (see `wake_guest_started_allocation`). One status read after
//! the in-computer deadline is the fallback; a failure hands later attempts to
//! the server-side path, whose helper call reconciles the same journal.
use super::*;
use crate::managed_bases::bootstrap::{LOADER, Purpose, Request};
use crate::providers::contract::VerifiedCreateSource;
use base64::Engine as _;

pub(crate) const STARTER: &str =
    include_str!("../../../../../infra/cloud/managed-base-fresh-start.py");
/// How long the computer waits for a safe path before it reports a timeout.
pub(crate) const GUEST_DEADLINE_SECONDS: i64 = 60;
/// When the worker reads the status once if the runtime has not enrolled.
const FALLBACK_AFTER_MS: i64 = (GUEST_DEADLINE_SECONDS + 15) * 1000;
/// Boat answers create with a provisioned machine that runs ~2 s later.
const RUNNING_WAIT: std::time::Duration = std::time::Duration::from_secs(20);
const RUNNING_POLL: std::time::Duration = std::time::Duration::from_millis(300);
const STATUS_ROOT: &str = "/run/zuse-fresh-start";
const MARKER: &str = "rustGuestStart";
/// Set at creation for new Boat workspaces; older ones keep the server path.
pub(crate) const VERSION_KEY: &str = "rustGuestStartVersion";
/// Job payload flag: enrollment may pull this allocation's job forward.
pub(crate) const DISPATCHED_FLAG: &str = "guestStartDispatched";

pub struct GuestStart<'a> {
    pub machine: &'a str,
    pub artifact: &'a str,
    pub name: &'a str,
    pub request: &'a Request,
    pub receipt: Option<&'a VerifiedCreateSource>,
    pub launch: &'a RuntimeLaunch,
}

/// Where the computer keeps the credential while it waits (root-only tmpfs).
pub(crate) fn credential_path(operation: &str) -> String {
    format!("{STATUS_ROOT}/{operation}/boot")
}

/// The single provider command. `launch` runs only after a ready receipt.
pub(crate) fn command(
    request: &Request,
    launch: &str,
    staged: &str,
) -> Result<String, ProviderError> {
    let args = json!({"version":1,"operationId":request.operation_id,"staged":staged,
        "starter":STARTER,"loader":LOADER,"helperSha256":request.image_pins.helper_sha256,
        "request":request.encoded()?,"launch":launch,"deadlineSeconds":GUEST_DEADLINE_SECONDS});
    let encoded = base64::engine::general_purpose::STANDARD
        .encode(serde_json::to_vec(&args).map_err(|_| ProviderError::Rejected)?);
    Ok(format!(
        "sudo -n /usr/bin/python3 -I -B -c {} {}",
        quote(STARTER),
        quote(&encoded)
    ))
}

pub(crate) fn status_command(operation: &str) -> String {
    format!(
        "sudo -n sh -c 'if test -f \"$1\"; then cat -- \"$1\"; fi' sh {}",
        quote(&format!("{STATUS_ROOT}/{operation}/status.json"))
    )
}

/// The dispatch answer: accepted, a transient refusal, or a rejection.
pub(crate) fn dispatch_reply(exit_code: i32, stdout: &[u8]) -> Result<(), ProviderError> {
    let value: Value =
        serde_json::from_slice(stdout).map_err(|_| ProviderError::InvalidResponse)?;
    match (exit_code, value["status"].as_str()) {
        (0, Some("accepted")) => Ok(()),
        (1, Some("rejected")) => {
            let code = value["code"].as_str().unwrap_or("");
            tracing::warn!(code = %safe_code(code), "Boat in-computer start refused");
            // The upload may not have landed or systemd may be briefly busy;
            // only these are worth another attempt.
            if matches!(
                code,
                "fresh_start_credential_missing" | "fresh_start_unit_failed"
            ) {
                Err(ProviderError::Unavailable)
            } else {
                Err(ProviderError::Rejected)
            }
        }
        _ => Err(ProviderError::InvalidResponse),
    }
}

fn safe_code(code: &str) -> &str {
    if code.len() <= 80
        && code
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        code
    } else {
        "unknown"
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The runtime was launched after a ready receipt the server accepts.
    Launched,
    /// Not started; later attempts use the server-side path.
    Failed(&'static str),
    /// A verification refused the start, or the record does not match.
    Rejected,
}

pub(crate) fn outcome(status: &Value, request: &Request) -> Outcome {
    if status.as_object().is_none_or(|v| v.is_empty()) {
        return Outcome::Failed("cloud_guest_start_lost");
    }
    if status["version"] != 1 || status["operationId"] != request.operation_id.as_str() {
        return Outcome::Rejected;
    }
    match status["status"].as_str() {
        Some("launched" | "launching") => match request.response(&status["receipt"]) {
            Ok(Some(_)) if status["status"] == "launched" => Outcome::Launched,
            Ok(Some(_)) => Outcome::Failed("cloud_guest_start_launch_unfinished"),
            _ => Outcome::Rejected,
        },
        Some("rejected") => Outcome::Rejected,
        Some("timeout" | "waiting") => Outcome::Failed("cloud_guest_start_timeout"),
        _ => Outcome::Failed("cloud_guest_start_failed"),
    }
}

impl RuntimeControl {
    /// `true` when this attempt was handled by the in-computer start.
    pub(super) async fn guest_start_allocation(
        &self,
        provider: &dyn AllocationProvider,
        job: &jobs::Job,
        workspace: &Value,
        operation: &Value,
        machine: &str,
    ) -> Result<bool, ApiError> {
        if !provider.guest_start() || workspace["request_config"][VERSION_KEY] != 1 {
            return Ok(false);
        }
        let Some(request) = initial_boot::request(
            workspace,
            operation,
            machine,
            Purpose::InitialAllocation,
            required(operation, "storage_incarnation_id")?,
        )?
        else {
            return Ok(false);
        };
        let source = required(operation, "snapshot_id")?;
        let name = required(operation, "machine_name")?;
        let marker = &workspace["request_config"][MARKER];
        if marker["version"] == 1
            && marker["operationId"] == request.operation_id.as_str()
            && marker["machineId"] == machine
        {
            if marker["serverPath"] == true {
                return Ok(false);
            }
            if workspace["runtime_credential_hash"].is_string() {
                // Enrolled: continue exactly as after a server-side launch.
                self.defer_allocation_observed(job, 2000, &provider.startup_observations())
                    .await?;
                return Ok(true);
            }
            let due = marker["fallbackAt"].as_i64().unwrap_or(0);
            if clock_ms() < due {
                // Woken early, not by enrollment. No provider call.
                self.save_guest_start(job, &request, None, Some(due), &json!({}))
                    .await?;
                return Ok(true);
            }
            let status = provider
                .guest_start_status(machine, &request.operation_id)
                .await
                .map_err(provider_error)?;
            let code = match outcome(&status, &request) {
                Outcome::Rejected => {
                    tracing::warn!(workspace = %job.resource_id,
                        code = %safe_code(status["code"].as_str().unwrap_or("")),
                        "Boat in-computer start was refused");
                    return Err(provider_error(ProviderError::Rejected));
                }
                Outcome::Launched => "cloud_guest_start_not_enrolled",
                Outcome::Failed(code) => code,
            };
            let mut next = marker.clone();
            next["serverPath"] = json!(true);
            next["code"] = json!(code);
            self.save_guest_start(
                job,
                &request,
                Some(next),
                None,
                &provider.startup_observations(),
            )
            .await?;
            return Err(ApiError::unavailable(code));
        }
        let config = &workspace["request_config"];
        let unbound = !workspace["runtime_credential_hash"].is_string()
            && config["runtimeBootstrapReceipt"].is_null()
            && config["rustStorageEnrollment"].is_null()
            && workspace["runtime_state"] != "online"
            && !initial_boot::saved(workspace, &request, source);
        if !unbound {
            return Ok(false);
        }
        // Source verification requires the original machine to be running.
        let until = tokio::time::Instant::now() + RUNNING_WAIT;
        loop {
            let observed = provider
                .inspect_id(machine)
                .await
                .map_err(provider_error)?
                .ok_or_else(|| ApiError::conflict("cloud_allocation_machine_missing"))?;
            if observed.id != machine || observed.name != name {
                return Err(ApiError::conflict("cloud_allocation_identity_conflict"));
            }
            match observed.state {
                MachineState::Running => break,
                MachineState::Starting if tokio::time::Instant::now() < until => {
                    tokio::time::sleep(RUNNING_POLL).await
                }
                MachineState::Starting => {
                    self.defer_starting_allocation(
                        job,
                        operation,
                        machine,
                        &provider.startup_observations(),
                    )
                    .await?;
                    return Ok(true);
                }
                _ => return Err(ApiError::unavailable("cloud_allocation_machine_starting")),
            }
        }
        // The same scope check and fresh fence as the server-side path; the
        // fence also moves the allocation to launching before any credential.
        self.allocation_scope_access(workspace, operation, clock_ms())
            .await?;
        let current = self
            .initial_boot_allocation_fence(job, &request, source, name, None)
            .await?;
        let authority = initial_boot::verified_create_source(&current, &request, source, name);
        let launch = RuntimeLaunch {
            operation_id: request.operation_id.clone(),
            workspace_id: job.resource_id.clone(),
            generation: job.generation,
            incarnation: request.storage_incarnation_id.clone(),
            api_url: self.signer.issuer(),
            boot_token: self.open_boot_token(job, operation)?,
        };
        provider
            .start_in_guest(&GuestStart {
                machine,
                artifact: source,
                name,
                request: &request,
                receipt: authority.as_ref(),
                launch: &launch,
            })
            .await
            .map_err(provider_error)?;
        let now = clock_ms();
        let due = now + FALLBACK_AFTER_MS;
        self.save_guest_start(
            job,
            &request,
            Some(
                json!({"version":1,"operationId":request.operation_id,"machineId":machine,
                "dispatchedAt":now,"fallbackAt":due}),
            ),
            Some(due),
            &provider.startup_observations(),
        )
        .await?;
        Ok(true)
    }

    /// Records the marker and/or defers this lease until `due`, under the
    /// current workspace lock and the original allocation identity.
    async fn save_guest_start(
        &self,
        job: &jobs::Job,
        request: &Request,
        marker: Option<Value>,
        due: Option<i64>,
        observations: &Value,
    ) -> Result<(), ApiError> {
        let mut tx = self.pool.begin().await?;
        let (row, generation, _) = locked_workspace(&mut tx, &job.resource_id).await?;
        let now = clock_ms();
        if generation != job.generation
            || blocked(&row)
            || row["request_config"]["rustAllocationOperationId"] != request.operation_id.as_str()
            || row["provider_sandbox_id"] != request.source_machine_id.as_str()
        {
            return Err(ApiError::conflict("cloud_allocation_fenced"));
        }
        if marker.is_some() || observations.as_object().is_some_and(|v| !v.is_empty()) {
            let mut config = row["request_config"].clone();
            if let Some(marker) = marker {
                config[MARKER] = marker;
            }
            timings::merge(&mut config, observations);
            sqlx::query("UPDATE api_cloud_workspaces SET request_config=$2 WHERE workspace_id=$1")
                .bind(&job.resource_id)
                .bind(config)
                .execute(&mut *tx)
                .await?;
        }
        let changed = if let Some(due) = due {
            sqlx::query("UPDATE zuse_rust_jobs SET state='pending',lease_token=NULL,lease_expires_at=NULL,due_at=$3,updated_at=$4,payload=payload||jsonb_build_object($5::text,true) WHERE job_id=$1 AND lease_token=$2 AND state='running' AND lease_expires_at>$4")
                .bind(&job.id).bind(&job.lease_token).bind(due).bind(now).bind(DISPATCHED_FLAG).execute(&mut *tx).await?
        } else {
            sqlx::query("UPDATE zuse_rust_jobs SET updated_at=$3 WHERE job_id=$1 AND lease_token=$2 AND state='running' AND lease_expires_at>$3")
                .bind(&job.id).bind(&job.lease_token).bind(now).execute(&mut *tx).await?
        };
        if changed.rows_affected() != 1 {
            return Err(ApiError::conflict("cloud_allocation_fenced"));
        }
        tx.commit().await?;
        Ok(())
    }
}

/// Runtime enrollment is the event an in-computer start waits for: make that
/// allocation's deferred job due now (its update NOTIFYs the worker).
pub(crate) async fn wake_guest_started_allocation(
    tx: &mut Transaction<'_, Postgres>,
    workspace: &str,
    generation: i64,
    now: i64,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE zuse_rust_jobs SET due_at=$3,updated_at=$3 WHERE kind=$4 AND resource_id=$1 AND resource_generation=$2 AND state='pending' AND due_at>$3 AND payload->>$5='true'")
        .bind(workspace).bind(generation).bind(now).bind(JOB_KIND).bind(DISPATCHED_FLAG)
        .execute(&mut **tx).await?;
    Ok(())
}

#[cfg(test)]
mod tests;
