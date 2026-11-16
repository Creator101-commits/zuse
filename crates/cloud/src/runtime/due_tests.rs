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
