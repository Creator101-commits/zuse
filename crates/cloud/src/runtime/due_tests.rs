//! Wakeup triggers (migration 0043) and the worker due-time query against a
//! real PostgreSQL. NOTIFY must arrive for new and retried work only, after
//! commit, and only on the writer's own schema channel.
use super::*;
use crate::{
    crypto::MasterKey,
    db::wake::{Wakeups, Work},
    tickets::TicketSigner,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::json;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{sync::Arc, time::Duration};

const STUB_TABLES: &str = "
CREATE TABLE zuse_rust_task_command_drains(workspace_id text, session_id text, revision bigint, next_attempt_at bigint NOT NULL, PRIMARY KEY(workspace_id, session_id));
CREATE TABLE zuse_rust_computer_recreations(operation_id text PRIMARY KEY, phase text NOT NULL, due_at bigint NOT NULL);
CREATE TABLE cloud_machine_images(image_id text PRIMARY KEY, state text NOT NULL, next_attempt_at bigint NOT NULL, lease_id text, lease_expires_at bigint);
CREATE TABLE zuse_rust_mailboxes(workspace_id text PRIMARY KEY, sequence bigint NOT NULL DEFAULT 0, revision bigint NOT NULL DEFAULT 0);
CREATE TABLE zuse_rust_account_event_outbox(outbox_id bigserial PRIMARY KEY, owner_id text NOT NULL, kind text NOT NULL, resource_id text NOT NULL, created_at bigint NOT NULL);
";

async fn schema_pool(url: &str, admin: &PgPool, prefix: &str, max: u32) -> (PgPool, String) {
    let schema = format!("{prefix}_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(admin)
        .await
        .unwrap();
    let scope = schema.clone();
    let pool = PgPoolOptions::new()
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
        .unwrap();
    (pool, schema)
}

async fn install(pool: &PgPool) {
    sqlx::raw_sql(crate::jobs::MIGRATION)
        .execute(pool)
        .await
        .unwrap();
    sqlx::raw_sql(STUB_TABLES).execute(pool).await.unwrap();
    sqlx::raw_sql(include_str!("../../migrations/0043_work_wakeups.sql"))
        .execute(pool)
        .await
        .unwrap();
}

async fn next(receiver: &mut tokio::sync::broadcast::Receiver<Work>) -> Option<Work> {
    tokio::time::timeout(Duration::from_millis(1500), receiver.recv())
        .await
        .ok()
        .map(Result::unwrap)
}

async fn quiet(receiver: &mut tokio::sync::broadcast::Receiver<Work>) {
    let got = tokio::time::timeout(Duration::from_millis(300), receiver.recv()).await;
    assert!(got.is_err(), "unexpected wakeup {got:?}");
}

#[tokio::test]
#[ignore = "requires explicit isolated localhost PostgreSQL"]
async fn work_triggers_notify_listeners_and_due_query_tracks_next_work() {
    let url = std::env::var("ZUSE_CLOUD_TEST_DATABASE_URL").unwrap();
    assert!(matches!(
        reqwest::Url::parse(&url).unwrap().host_str(),
        Some("localhost" | "127.0.0.1" | "::1")
    ));
    let admin = PgPool::connect(&url).await.unwrap();
    let (pool, _) = schema_pool(&url, &admin, "work_wake", 4).await;
    let (other, _) = schema_pool(&url, &admin, "work_wake_other", 2).await;
    install(&pool).await;
    install(&other).await;
    let (listen_pool, _) = {
        // The listener must see the same search_path as the writer.
        let schema: String = sqlx::query_scalar("SELECT current_schema()")
            .fetch_one(&pool)
            .await
            .unwrap();
        let scope = schema.clone();
        let listen = PgPoolOptions::new()
            .max_connections(1)
            .after_connect(move |c, _| {
                let scope = scope.clone();
                Box::pin(async move {
                    sqlx::query(&format!("SET search_path={scope}"))
                        .execute(c)
                        .await?;
                    Ok(())
                })
            })
            .connect(&url)
            .await
            .unwrap();
        (listen, schema)
    };
    let wakeups: Arc<Wakeups> = Wakeups::listen_with(listen_pool).await.unwrap();
    assert!(wakeups.enabled());
    let mut receiver = wakeups.subscribe();

    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query("INSERT INTO zuse_rust_control_owners VALUES('workspace','ws','rust',1,$1)")
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();
    // Enqueue: notified once, and only after commit.
    let mut tx = pool.begin().await.unwrap();
    crate::jobs::enqueue(&mut tx, "k1", lifecycle::JOB_KIND, "ws", 1, &json!({}), now)
        .await
        .unwrap();
    quiet(&mut receiver).await;
    tx.commit().await.unwrap();
    assert_eq!(next(&mut receiver).await, Some(Work::Jobs));
    quiet(&mut receiver).await;

    // Claiming never wakes anyone (no self-wake loop); retrying does.
    let job = crate::jobs::claim(&pool, lifecycle::JOB_KIND, "workspace", now, 60_000)
        .await
        .unwrap()
        .unwrap();
    quiet(&mut receiver).await;
    assert!(
        crate::jobs::retry(&pool, &job, now, "test_retry")
            .await
            .unwrap()
    );
    assert_eq!(next(&mut receiver).await, Some(Work::Jobs));

    // Another schema on the same database stays silent.
    let mut tx = other.begin().await.unwrap();
    crate::jobs::enqueue(&mut tx, "k2", lifecycle::JOB_KIND, "ws", 1, &json!({}), now)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    sqlx::query("INSERT INTO zuse_rust_account_event_outbox(owner_id,kind,resource_id,created_at) VALUES('o','workspace','ws',1)")
        .execute(&other)
        .await
        .unwrap();
    quiet(&mut receiver).await;

    // Several outbox rows in one transaction arrive as one notification.
    sqlx::query("INSERT INTO zuse_rust_account_event_outbox(owner_id,kind,resource_id,created_at) VALUES('o','workspace','a',1),('o','workspace','b',1)")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(next(&mut receiver).await, Some(Work::AccountEvents));
    quiet(&mut receiver).await;
    sqlx::query("INSERT INTO zuse_rust_task_command_drains VALUES('ws','s',1,$1)")
        .bind(now + 5_000)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(next(&mut receiver).await, Some(Work::TaskDrain));
    sqlx::query("INSERT INTO zuse_rust_computer_recreations VALUES('r','ready',$1)")
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();
    quiet(&mut receiver).await;
    sqlx::query(
        "INSERT INTO cloud_machine_images(image_id,state,next_attempt_at) VALUES('i','queued',$1)",
    )
    .bind(now + 9_000)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(next(&mut receiver).await, Some(Work::MachineImages));
    // Mailbox revisions wake command long-polls in the API process.
    sqlx::query("INSERT INTO zuse_rust_mailboxes(workspace_id) VALUES('ws')")
        .execute(&pool)
        .await
        .unwrap();
    quiet(&mut receiver).await;
    sqlx::query("UPDATE zuse_rust_mailboxes SET revision=revision+1 WHERE workspace_id='ws'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(next(&mut receiver).await, Some(Work::Mailbox));

    // The due query: the retried job's backoff, the drain and the image.
    let key = URL_SAFE_NO_PAD.encode([48; 32]);
    let signing = ed25519_dalek::SigningKey::from_bytes(&[49; 32]);
    let jwk = json!({"kty":"OKP","crv":"Ed25519","d":URL_SAFE_NO_PAD.encode([49;32]),"x":URL_SAFE_NO_PAD.encode(signing.verifying_key().as_bytes())});
    let control = RuntimeControl::from_pool(
        pool.clone(),
        MasterKey::parse(&key).unwrap(),
        TicketSigner::from_jwk(&jwk.to_string(), "https://api.example.test").unwrap(),
    );
    let due = control.worker_due().await.unwrap();
    let retried_at: i64 =
        sqlx::query_scalar("SELECT due_at FROM zuse_rust_jobs WHERE idempotency_key='k1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(retried_at > now);
    assert_eq!(due.lifecycle, Some(retried_at));
    assert_eq!(due.task_drain, Some(now + 5_000));
    assert_eq!(due.recreation, None, "finished recreations are not due");
    assert_eq!(due.user_images, Some(now + 9_000));
    assert_eq!(due.allocation, None);
    assert_eq!(due.earliest(), Some((now + 5_000).min(retried_at)));

    // A claimed job is due again only when its lease expires.
    sqlx::query("UPDATE zuse_rust_jobs SET due_at=$1 WHERE idempotency_key='k1'")
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();
    let job = crate::jobs::claim(&pool, lifecycle::JOB_KIND, "workspace", now, 60_000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        control.worker_due().await.unwrap().lifecycle,
        Some(now + 60_000)
    );
    // A job whose owner moved to another generation is not claimable: not due.
    crate::jobs::finish(&pool, &job, "workspace", now)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    crate::jobs::enqueue(&mut tx, "k3", deletion::JOB_KIND, "ws", 7, &json!({}), now)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let due = control.worker_due().await.unwrap();
    assert_eq!((due.lifecycle, due.deletion), (None, None));
}

struct NoDriver;
#[async_trait::async_trait]
impl crate::connections::worker::AuthDriver for NoDriver {
    async fn execute(
        &self,
        _: crate::connections::worker::AuthJob,
        _: tokio::sync::mpsc::Sender<crate::connections::worker::DeviceChallenge>,
    ) -> Result<crate::connections::worker::VerifiedConnection, &'static str> {
        panic!("nothing in this fixture is claimable by the worker");
    }
}

/// Migration 0045 and the credential worker's due query.
#[tokio::test]
#[ignore = "requires explicit isolated localhost PostgreSQL"]
async fn auth_triggers_wake_only_for_claimable_work_and_due_query_tracks_deadlines() {
    let url = std::env::var("ZUSE_CLOUD_TEST_DATABASE_URL").unwrap();
    assert!(matches!(
        reqwest::Url::parse(&url).unwrap().host_str(),
        Some("localhost" | "127.0.0.1" | "::1")
    ));
    let admin = PgPool::connect(&url).await.unwrap();
    let (pool, schema) = schema_pool(&url, &admin, "auth_wake", 4).await;
    install(&pool).await;
    for sql in [
        include_str!("../../migrations/0009_computers.sql"),
        include_str!("../../migrations/0024_credential_delivery_journal.sql"),
        include_str!("../../migrations/0045_auth_wakeups.sql"),
        // Idempotent: a second run replaces the triggers and keeps the indexes.
        include_str!("../../migrations/0045_auth_wakeups.sql"),
    ] {
        sqlx::raw_sql(sql).execute(&pool).await.unwrap();
    }
    let listen = PgPoolOptions::new()
        .max_connections(1)
        .after_connect(move |c, _| {
            let scope = schema.clone();
            Box::pin(async move {
                sqlx::query(&format!("SET search_path={scope}"))
                    .execute(c)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    let wakeups = Wakeups::listen_with(listen).await.unwrap();
    let mut receiver = wakeups.subscribe();
    let key = URL_SAFE_NO_PAD.encode([48; 32]);
    let control = crate::connections::ConnectionControl::from_pool(
        pool.clone(),
        MasterKey::parse(&key).unwrap(),
        vec!["owner".to_owned()],
    );
    assert!(control.auth_wakeups_installed().await.unwrap());
    let now = chrono::Utc::now().timestamp_millis();
    let exec = |sql: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query(sql).bind(now).execute(&pool).await.unwrap();
        }
    };
    exec("INSERT INTO cloud_agent_connections(connection_id,owner_id,idempotency_key,request_digest,agent,method,state,created_at,updated_at) VALUES('signin','owner','k1','d','codex','device-code','pending',$1,$1),('update','owner','k2','d','codex','api-key','connected',$1,$1),('quiet','owner','k3','d','codex','device-code','pending',$1,$1),('foreign','stranger','k4','d','codex','device-code','pending',$1,$1)").await;
    quiet(&mut receiver).await;
    assert_eq!(control.auth_due(now).await.unwrap().next(now), None);

    // A queued sign-in wakes the worker and is due now.
    exec("INSERT INTO cloud_auth_operations(operation_id,owner_id,actor_id,connection_id,idempotency_key,state,expires_at,created_at,updated_at) VALUES('op-signin','owner','owner','signin','i1','queued',$1+900000,$1,$1)").await;
    assert_eq!(next(&mut receiver).await, Some(Work::Auth));
    let due = control.auth_due(now).await.unwrap();
    assert!(due.claimable);
    assert_eq!(due.operation_deadline, Some(now + 900_000));
    assert_eq!(due.next(now), Some(now));

    // The worker's claim, challenge and heartbeats stay silent; the lease
    // end becomes the deadline.
    exec("UPDATE cloud_auth_operations SET state='authorizing',lease_id='worker-lease',lease_expires_at=$1+15000,updated_at=$1 WHERE operation_id='op-signin'").await;
    exec(
        "UPDATE cloud_auth_operations SET lease_expires_at=$1+15000 WHERE operation_id='op-signin'",
    )
    .await;
    exec("UPDATE cloud_auth_operations SET operation_ciphertext='challenge',updated_at=$1 WHERE operation_id='op-signin'").await;
    quiet(&mut receiver).await;
    let due = control.auth_due(now).await.unwrap();
    assert!(!due.claimable);
    assert_eq!(due.next(now), Some(now + 15_000));

    // A credential proposal waiting for verification wakes the worker.
    exec("INSERT INTO cloud_auth_operations(operation_id,owner_id,actor_id,connection_id,idempotency_key,state,expires_at,created_at,updated_at,lease_id,lease_expires_at) VALUES('auth_cfg_x','owner','owner','update','i2','authorizing',$1+120000,$1,$1,'credential-proposal-v1',$1+120000)").await;
    assert_eq!(next(&mut receiver).await, Some(Work::Auth));
    assert!(control.auth_due(now).await.unwrap().claimable);
    exec("UPDATE cloud_auth_operations SET state='error',lease_id=NULL,lease_expires_at=NULL WHERE operation_id='auth_cfg_x'").await;
    quiet(&mut receiver).await;

    // Another account's sign-in notifies but never makes this worker due.
    exec("INSERT INTO cloud_auth_operations(operation_id,owner_id,actor_id,connection_id,idempotency_key,state,expires_at,created_at,updated_at) VALUES('op-foreign','stranger','stranger','foreign','i3','queued',$1+1000,$1,$1)").await;
    assert_eq!(next(&mut receiver).await, Some(Work::Auth));
    assert_eq!(
        control.auth_due(now).await.unwrap().next(now),
        Some(now + 15_000)
    );

    // A sign-in that ended without an outcome leaves its pending connection
    // to be failed now.
    exec("UPDATE cloud_auth_operations SET state='error',lease_id=NULL,lease_expires_at=NULL WHERE operation_id='op-signin'").await;
    quiet(&mut receiver).await;
    let due = control.auth_due(now).await.unwrap();
    assert!(due.abandoned_pending);
    assert_eq!(due.next(now), Some(now));

    // A refresh dispatch wakes the worker with its lease deadline. A result
    // whose connection still matches waits for serve and is not due.
    exec("UPDATE cloud_agent_connections SET state='connected',refresh_lease_id='refresh',refresh_lease_expires_at=$1+60000 WHERE connection_id='quiet'").await;
    quiet(&mut receiver).await;
    exec("INSERT INTO zuse_rust_credential_rotations(rotation_id,owner_id,connection_id,agent,authority_epoch,source_version,lease_id,phase,started_at,lease_expires_at,updated_at) VALUES('r1','owner','quiet','codex',1,1,'refresh','dispatching',$1,$1+60000,$1)").await;
    assert_eq!(next(&mut receiver).await, Some(Work::Auth));
    assert_eq!(
        control.auth_due(now).await.unwrap().dispatch_deadline,
        Some(now + 60_000)
    );
    exec("UPDATE zuse_rust_credential_rotations SET phase='result',result_ciphertext='sealed',updated_at=$1 WHERE rotation_id='r1'").await;
    quiet(&mut receiver).await;
    // Finished journal rows are due once their retention ends.
    exec("INSERT INTO zuse_rust_credential_rotations(rotation_id,owner_id,connection_id,agent,authority_epoch,source_version,lease_id,phase,started_at,lease_expires_at,updated_at) VALUES('r0','owner','signin','codex',1,1,'old','applied',$1-1000,$1-1000,$1-1000)").await;
    assert_eq!(next(&mut receiver).await, Some(Work::Auth));
    let retention = crate::connections::due::ROTATION_RETENTION_MS;
    let due = control.auth_due(now).await.unwrap();
    assert!(!due.stale_result);
    assert_eq!(due.oldest_finished_rotation, Some(now - 1_000));

    // The worker fails the abandoned connection. Afterwards only the
    // retention deadline remains: neither the matching result nor the
    // pending connection without a sign-in keeps the worker busy.
    assert!(!control.work_once(&NoDriver).await.unwrap());
    let state: String = sqlx::query_scalar(
        "SELECT state FROM cloud_agent_connections WHERE connection_id='signin'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, "error");
    let due = control.auth_due(now).await.unwrap();
    assert!(!due.abandoned_pending && !due.claimable && !due.stale_result);
    assert_eq!(due.next(now), Some(now - 1_000 + retention + 1));

    // Disconnecting a connection that holds a refresh lease wakes the worker
    // to discard the now-stale result.
    exec("UPDATE cloud_agent_connections SET state='revoked',credential_ciphertext=NULL,version=version+1,authority_epoch=authority_epoch+1,refresh_lease_id=NULL,refresh_lease_expires_at=NULL,updated_at=$1 WHERE connection_id='quiet'").await;
    assert_eq!(next(&mut receiver).await, Some(Work::Auth));
    let due = control.auth_due(now).await.unwrap();
    assert!(due.stale_result);
    assert_eq!(due.next(now), Some(now));
    assert!(!control.work_once(&NoDriver).await.unwrap());
    let phase: String = sqlx::query_scalar(
        "SELECT phase FROM zuse_rust_credential_rotations WHERE rotation_id='r1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(phase, "superseded");
    let later = chrono::Utc::now().timestamp_millis();
    let due = control.auth_due(later).await.unwrap();
    assert!(!due.stale_result);
    assert_eq!(due.next(later), Some(now - 1_000 + retention + 1));
}
