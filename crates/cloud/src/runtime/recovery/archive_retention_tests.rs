//! Migration 0050 against a real PostgreSQL: archiving schedules the
//! computer's deletion at a persisted due time the worker sleeps until,
//! unarchiving cancels it, and admission never leaves a past due time behind
//! (which would wake the worker every round).
use super::*;
use crate::{
    crypto::MasterKey,
    db::wake::{Wakeups, Work},
    tickets::TicketSigner,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sqlx::postgres::PgPoolOptions;

const NOTIFY_FUNCTION: &str = "CREATE OR REPLACE FUNCTION zuse_cloud_notify_work() RETURNS trigger LANGUAGE plpgsql AS $body$ BEGIN PERFORM pg_notify(left('zuse_cloud_work:' || TG_TABLE_SCHEMA, 63), TG_ARGV[0]); RETURN NULL; END $body$;";
/// Other worker sources read by the one due-time statement.
const DUE_SOURCES: &str = "
CREATE TABLE zuse_rust_task_command_drains(workspace_id text, session_id text, next_attempt_at bigint NOT NULL);
CREATE TABLE zuse_rust_computer_recreations(operation_id text PRIMARY KEY, phase text NOT NULL, due_at bigint NOT NULL);
CREATE TABLE cloud_machine_images(image_id text PRIMARY KEY, state text NOT NULL, next_attempt_at bigint NOT NULL, lease_expires_at bigint);
";
const MIGRATION_0050: &str = include_str!("../../../migrations/0050_archive_machine_deletion.sql");

async fn scoped_pool(url: &str, schema: &str, max: u32) -> PgPool {
    let scope = schema.to_owned();
    PgPoolOptions::new()
        .max_connections(max)
        .after_connect(move |c, _| {
            let scope = scope.clone();
            Box::pin(async move {
                sqlx::query(&format!("SET search_path={scope}"))
                    .execute(c)
                    .await?;
                Ok(())
            })
        })
        .connect(url)
        .await
        .unwrap()
}

/// `ws-alice` is an archived, retained computer owned by the Rust backend.
async fn fixture(with_0050: bool) -> (PgPool, String, String) {
    let url = std::env::var("ZUSE_CLOUD_TEST_DATABASE_URL").unwrap();
    assert!(matches!(
        reqwest::Url::parse(&url).unwrap().host_str(),
        Some("localhost" | "127.0.0.1" | "::1")
    ));
    let admin = PgPool::connect(&url).await.unwrap();
    let schema = format!("archive_due_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let pool = scoped_pool(&url, &schema, 4).await;
    for sql in [
        include_str!("../../../tests/fixtures/legacy-catalog.sql"),
        jobs::MIGRATION,
        MIGRATION,
        NOTIFY_FUNCTION,
        DUE_SOURCES,
    ] {
        sqlx::raw_sql(sql).execute(&pool).await.unwrap();
    }
    if with_0050 {
        sqlx::raw_sql(MIGRATION_0050).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO zuse_rust_control_owners VALUES('workspace','ws-alice','rust',2,1)")
        .execute(&pool)
        .await
        .unwrap();
    (pool, schema, url)
}

fn control(pool: &PgPool, recovery: bool) -> RuntimeControl {
    let signing = ed25519_dalek::SigningKey::from_bytes(&[71; 32]);
    let jwk = json!({"kty":"OKP","crv":"Ed25519","d":URL_SAFE_NO_PAD.encode([71;32]),"x":URL_SAFE_NO_PAD.encode(signing.verifying_key().as_bytes())});
    RuntimeControl::from_pool(
        pool.clone(),
        MasterKey::parse(&URL_SAFE_NO_PAD.encode([72; 32])).unwrap(),
        TicketSigner::from_jwk(&jwk.to_string(), "https://api.example.test").unwrap(),
    )
    .with_recovery_policy(recovery, vec![])
}

async fn next(receiver: &mut tokio::sync::broadcast::Receiver<Work>) -> Option<Work> {
    tokio::time::timeout(Duration::from_millis(1500), receiver.recv())
        .await
        .ok()
        .and_then(Result::ok)
}

async fn due_at(pool: &PgPool) -> Option<i64> {
    sqlx::query_scalar(
        "SELECT machine_delete_due_at FROM api_cloud_workspaces WHERE workspace_id='ws-alice'",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn archive(pool: &PgPool, at: i64) {
    sqlx::query("UPDATE api_cloud_workspaces SET state='archived',desired_state='archived',archive_requested_at=$1,archive_delete_at=$2,revision=revision+1 WHERE workspace_id='ws-alice'")
        .bind(at)
        .bind(at + zuse_protocol::cloud::ARCHIVE_RETENTION_MS)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires explicit isolated localhost PostgreSQL"]
async fn archive_schedules_machine_deletion_and_unarchive_cancels_it() {
    let (pool, schema, url) = fixture(true).await;
    let now = chrono::Utc::now().timestamp_millis();
    let listen = scoped_pool(&url, &schema, 1).await;
    let wakeups = Wakeups::listen_with(listen).await.unwrap();
    let mut receiver = wakeups.subscribe();

    // The fixture's archive (requested at 100) is backfilled to seven days.
    assert_eq!(
        due_at(&pool).await,
        Some(100 + zuse_protocol::cloud::ARCHIVE_RETENTION_MS)
    );

    // Unarchive (every lifecycle path clears the deadline) cancels deletion.
    sqlx::query("UPDATE api_cloud_workspaces SET state='paused',desired_state='paused',archive_requested_at=NULL,archive_delete_at=NULL WHERE workspace_id='ws-alice'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(due_at(&pool).await, None);
    let enabled = control(&pool, true);
    assert_eq!(enabled.worker_due().await.unwrap().archive_retention, None);

    // Archive: due at archived_at + 7 days, and the worker hears about it.
    archive(&pool, now).await;
    assert_eq!(next(&mut receiver).await, Some(Work::ArchiveRetention));
    let deadline = now + zuse_protocol::cloud::ARCHIVE_RETENTION_MS;
    assert_eq!(deadline - now, 7 * 24 * 60 * 60 * 1000);
    assert_eq!(due_at(&pool).await, Some(deadline));
    assert_eq!(
        enabled.worker_due().await.unwrap().archive_retention,
        Some(deadline)
    );
    // Not due yet: admission leaves it alone.
    enabled.recovery_admit_due(now).await.unwrap();
    assert_eq!(due_at(&pool).await, Some(deadline));

    // Recovery disabled, or a computer not owned by this backend: never due.
    assert_eq!(
        control(&pool, false)
            .worker_due()
            .await
            .unwrap()
            .archive_retention,
        None
    );
    sqlx::query("UPDATE zuse_rust_control_owners SET backend='legacy'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(enabled.worker_due().await.unwrap().archive_retention, None);
    sqlx::query("UPDATE zuse_rust_control_owners SET backend='rust'")
        .execute(&pool)
        .await
        .unwrap();

    // Unrelated workspace writes keep the due time and stay silent.
    sqlx::query("UPDATE api_cloud_workspaces SET status_code='archived',revision=revision+1 WHERE workspace_id='ws-alice'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(due_at(&pool).await, Some(deadline));
    assert_eq!(next(&mut receiver).await, None);

    // Unarchive cancels the pending deletion.
    sqlx::query("UPDATE api_cloud_workspaces SET state='paused',desired_state='paused',archive_requested_at=NULL,archive_delete_at=NULL WHERE workspace_id='ws-alice'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(due_at(&pool).await, None);
    assert_eq!(enabled.worker_due().await.unwrap().archive_retention, None);
    enabled.recovery_admit_due(deadline + 1).await.unwrap();
    let no_operation: bool =
        sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM zuse_rust_recovery_operations)")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(no_operation);
}

#[tokio::test]
#[ignore = "requires explicit isolated localhost PostgreSQL"]
async fn due_admission_backs_off_instead_of_waking_every_round() {
    let (pool, _, _) = fixture(true).await;
    let now = chrono::Utc::now().timestamp_millis();
    let archived_at = now - zuse_protocol::cloud::ARCHIVE_RETENTION_MS - 1000;
    archive(&pool, archived_at).await;
    let enabled = control(&pool, true);
    assert_eq!(
        enabled.worker_due().await.unwrap().archive_retention,
        Some(now - 1000)
    );
    // This fixture cannot start a backup (no archive store): the attempt
    // reports its error and the computer is looked at again in an hour.
    let error = enabled.recovery_admit_due(now).await.unwrap_err();
    assert!(error.code.starts_with("cloud_"), "{}", error.code);
    let retry = due_at(&pool).await.unwrap();
    assert!(retry >= now + ARCHIVE_ADMISSION_RETRY_MS);
    let due = enabled.worker_due().await.unwrap();
    assert_eq!(due.archive_retention, Some(retry));
    assert!(!super::super::due::should_run(
        Some(&due),
        |d| d.archive_retention,
        now + 1
    ));
    // Re-archiving later starts a new window from the new request.
    archive(&pool, now).await;
    assert_eq!(
        due_at(&pool).await,
        Some(now + zuse_protocol::cloud::ARCHIVE_RETENTION_MS)
    );

    // A started backup excludes the computer until it ends; a cancelled one
    // (still archived) becomes due again.
    archive(&pool, archived_at).await;
    sqlx::query("INSERT INTO zuse_rust_recovery_operations(operation_id,workspace_id,generation,source_machine_id,storage_incarnation_id,binding,phase,expires_at,created_at,updated_at) VALUES('op','ws-alice',2,'machine-alice','storage','{}','uploading',$1,$1,$1)")
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(enabled.worker_due().await.unwrap().archive_retention, None);
    sqlx::query("UPDATE zuse_rust_recovery_operations SET phase='cancelled'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        enabled.worker_due().await.unwrap().archive_retention,
        Some(now - 1000)
    );
    // A retired computer (machine already deleted) is never due again.
    sqlx::query(
        "UPDATE api_cloud_workspaces SET provider_sandbox_id=NULL WHERE workspace_id='ws-alice'",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(enabled.worker_due().await.unwrap().archive_retention, None);
}

#[tokio::test]
#[ignore = "requires explicit isolated localhost PostgreSQL"]
async fn before_migration_0050_the_original_retention_scan_still_admits() {
    let (pool, _, _) = fixture(false).await;
    let enabled = control(&pool, true);
    assert_eq!(enabled.worker_due().await.unwrap().archive_retention, None);
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query("UPDATE api_cloud_workspaces SET archive_requested_at=$1,archive_delete_at=$2 WHERE workspace_id='ws-alice'")
        .bind(now - zuse_protocol::cloud::ARCHIVE_RETENTION_MS - 1)
        .bind(now - 1)
        .execute(&pool)
        .await
        .unwrap();
    // The legacy scan reaches begin_recovery, which reports the fixture's
    // missing archive store.
    assert!(enabled.recovery_admit_due(now).await.is_err());
    sqlx::query("UPDATE api_cloud_workspaces SET archive_requested_at=$1,archive_delete_at=$2 WHERE workspace_id='ws-alice'")
        .bind(now)
        .bind(now + zuse_protocol::cloud::ARCHIVE_RETENTION_MS)
        .execute(&pool)
        .await
        .unwrap();
    enabled.recovery_admit_due(now).await.unwrap();
}
