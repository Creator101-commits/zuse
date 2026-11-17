//! Migration 0047 against a real PostgreSQL: the runtime worker hears
//! `idle_sleep` only when a shared-credit hold turns on, after commit.
use crate::db::wake::{Wakeups, Work};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::time::Duration;

const STUB: &str = "CREATE TABLE api_cloud_billing_periods(period_id text PRIMARY KEY, account_id text NOT NULL, shared_credit_account boolean NOT NULL DEFAULT false, shared_credit_hold boolean NOT NULL DEFAULT false, updated_at bigint NOT NULL DEFAULT 0);";

fn scoped(schema: &str, max: u32) -> PgPoolOptions {
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
async fn billing_hold_trigger_notifies_idle_sleep_only_when_the_hold_turns_on() {
    let url = std::env::var("ZUSE_CLOUD_TEST_DATABASE_URL").unwrap();
    assert!(matches!(
        reqwest::Url::parse(&url).unwrap().host_str(),
        Some("localhost" | "127.0.0.1" | "::1")
    ));
    let admin = PgPool::connect(&url).await.unwrap();
    let schema = format!("billing_wake_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let pool = scoped(&schema, 2).connect(&url).await.unwrap();
    // Only the notify function from 0043; its other triggers need unrelated tables.
    let wakeups = include_str!("../../migrations/0043_work_wakeups.sql");
    let function = &wakeups[..wakeups.find("END $body$;").unwrap() + "END $body$;".len()];
    sqlx::raw_sql(function).execute(&pool).await.unwrap();
    sqlx::raw_sql(STUB).execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../../migrations/0047_billing_wakeups.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let listen = scoped(&schema, 1).connect(&url).await.unwrap();
    let wakeups = Wakeups::listen_with(listen).await.unwrap();
    let mut receiver = wakeups.subscribe();

    // New periods never notify, even if created already held.
    sqlx::query("INSERT INTO api_cloud_billing_periods(period_id,account_id,shared_credit_account) VALUES('p1','user-a',true),('p0','user-a',false)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO api_cloud_billing_periods(period_id,account_id,shared_credit_account,shared_credit_hold) VALUES('p2','user-b',true,true)")
        .execute(&pool)
        .await
        .unwrap();
    quiet(&mut receiver).await;

    // The hold statement the billing control runs (compute.rs).
    let hold = |account: &'static str, value: bool| {
        sqlx::query("UPDATE api_cloud_billing_periods SET shared_credit_hold=$2 WHERE account_id=$1 AND shared_credit_account AND shared_credit_hold IS DISTINCT FROM $2")
            .bind(account)
            .bind(value)
    };
    // Off -> on notifies once, and only after commit.
    let mut tx = pool.begin().await.unwrap();
    hold("user-a", true).execute(&mut *tx).await.unwrap();
    quiet(&mut receiver).await;
    tx.commit().await.unwrap();
    assert_eq!(next(&mut receiver).await, Some(Work::IdleSleep));
    quiet(&mut receiver).await;

    // Recomputing an unchanged hold, even with a plain rewrite, stays silent.
    assert_eq!(
        hold("user-a", true)
            .execute(&pool)
            .await
            .unwrap()
            .rows_affected(),
        0
    );
    sqlx::query(
        "UPDATE api_cloud_billing_periods SET shared_credit_hold=true WHERE period_id='p1'",
    )
    .execute(&pool)
    .await
    .unwrap();
    // Other columns and releasing a hold stay silent too.
    sqlx::query("UPDATE api_cloud_billing_periods SET updated_at=5 WHERE account_id='user-a'")
        .execute(&pool)
        .await
        .unwrap();
    hold("user-a", false).execute(&pool).await.unwrap();
    quiet(&mut receiver).await;

    // A rolled-back hold is never announced; a later one is.
    let mut tx = pool.begin().await.unwrap();
    hold("user-a", true).execute(&mut *tx).await.unwrap();
    tx.rollback().await.unwrap();
    quiet(&mut receiver).await;
    hold("user-a", true).execute(&pool).await.unwrap();
    assert_eq!(next(&mut receiver).await, Some(Work::IdleSleep));

    drop(receiver);
    drop(wakeups);
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
}
