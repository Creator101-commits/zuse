//! Plugin change triggers (migration 0046) against a real PostgreSQL. Every
//! committed change to connections or custom servers must reach listeners of
//! the writer's own schema, since cached plugin views depend on it.
use super::{EXECUTOR_WAKEUPS_MIGRATION, MIGRATION, WAKEUPS_MIGRATION};
use crate::db::wake::{Wakeups, Work};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::time::Duration;

async fn schema_pool(url: &str, admin: &PgPool, max: u32) -> (PgPool, String) {
    let schema = format!("plugin_wake_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(admin)
        .await
        .unwrap();
    (pool_for(url, &schema, max).await, schema)
}

async fn pool_for(url: &str, schema: &str, max: u32) -> PgPool {
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
async fn plugin_change_triggers_notify_listeners_after_commit() {
    let url = std::env::var("ZUSE_CLOUD_TEST_DATABASE_URL").unwrap();
    assert!(matches!(
        reqwest::Url::parse(&url).unwrap().host_str(),
        Some("localhost" | "127.0.0.1" | "::1")
    ));
    let admin = PgPool::connect(&url).await.unwrap();

    // Bootstrap before `plugin-migrate`: no plugin tables, nothing to install.
    let (bare, bare_schema) = schema_pool(&url, &admin, 1).await;
    sqlx::raw_sql(WAKEUPS_MIGRATION)
        .execute(&bare)
        .await
        .unwrap();
    let triggers: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid WHERE c.relnamespace=current_schema()::regnamespace AND t.tgname LIKE 'zuse_cloud_plugin_%'",
    )
    .fetch_one(&bare)
    .await
    .unwrap();
    assert_eq!(triggers, 0);

    // `plugin-migrate` order: tables, then triggers; repeating is harmless and
    // the notify function is created when 0043 has not run in this schema.
    let (pool, schema) = schema_pool(&url, &admin, 2).await;
    sqlx::raw_sql(MIGRATION).execute(&pool).await.unwrap();
    sqlx::raw_sql(WAKEUPS_MIGRATION)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(WAKEUPS_MIGRATION)
        .execute(&pool)
        .await
        .unwrap();
    let listen = pool_for(&url, &schema, 1).await;
    let wakeups = Wakeups::listen_with(listen.clone()).await.unwrap();
    let mut receiver = wakeups.subscribe();

    sqlx::query("INSERT INTO zuse_plugin_servers(tenant_id,server_id,sealed,created_at) VALUES('personal:a','custom-00000000000000000000000000000001','x',1)")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(next(&mut receiver).await, Some(Work::Plugins));
    let connection = "c00000000000000000000000000000001";
    sqlx::query("INSERT INTO zuse_plugin_connections(tenant_id,connection_id,subject,plugin_id,label,owner,state,created_at,updated_at) VALUES('personal:a',$1,'a','custom-00000000000000000000000000000001','Tools','user','connecting',1,1)")
        .bind(connection)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(next(&mut receiver).await, Some(Work::Plugins));
    sqlx::query("UPDATE zuse_plugin_connections SET enabled=false WHERE connection_id=$1")
        .bind(connection)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(next(&mut receiver).await, Some(Work::Plugins));

    // Only after commit; several changes in one transaction arrive once.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE zuse_plugin_connections SET state='connected' WHERE connection_id=$1")
        .bind(connection)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE zuse_plugin_servers SET sealed='y'")
        .execute(&mut *tx)
        .await
        .unwrap();
    quiet(&mut receiver).await;
    tx.commit().await.unwrap();
    assert_eq!(next(&mut receiver).await, Some(Work::Plugins));
    quiet(&mut receiver).await;

    // Attempts are never cached, so they stay silent.
    sqlx::query("INSERT INTO zuse_plugin_attempts(tenant_id,attempt_id,subject,plugin_id,state,sealed,expires_at,created_at,updated_at) VALUES('personal:a','c00000000000000000000000000000002','a','p','pending','x',1,1,1)")
        .execute(&pool)
        .await
        .unwrap();
    quiet(&mut receiver).await;

    // A rolled-back change sends nothing.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("DELETE FROM zuse_plugin_connections")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    quiet(&mut receiver).await;

    // Another schema's plugin changes never reach this listener.
    let (other, other_schema) = schema_pool(&url, &admin, 1).await;
    sqlx::raw_sql(MIGRATION).execute(&other).await.unwrap();
    sqlx::raw_sql(WAKEUPS_MIGRATION)
        .execute(&other)
        .await
        .unwrap();
    sqlx::query("INSERT INTO zuse_plugin_servers(tenant_id,server_id,sealed,created_at) VALUES('personal:a','custom-00000000000000000000000000000001','x',1)")
        .execute(&other)
        .await
        .unwrap();
    quiet(&mut receiver).await;

    sqlx::query("DELETE FROM zuse_plugin_connections")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(next(&mut receiver).await, Some(Work::Plugins));
    sqlx::query("DELETE FROM zuse_plugin_servers")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(next(&mut receiver).await, Some(Work::Plugins));

    // 0048: every SDK table (created by the SDK migration) notifies the same way.
    sqlx::raw_sql("CREATE TABLE executor_accounts(id text PRIMARY KEY, v int); CREATE TABLE executor_oauth_grants(id text PRIMARY KEY, status text); CREATE TABLE other_rows(id text)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(EXECUTOR_WAKEUPS_MIGRATION)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(EXECUTOR_WAKEUPS_MIGRATION)
        .execute(&pool)
        .await
        .unwrap();
    for statement in [
        "INSERT INTO executor_accounts VALUES('a',1)",
        "UPDATE executor_accounts SET v=2",
        "INSERT INTO executor_oauth_grants VALUES('a','ready_1')",
        "UPDATE executor_oauth_grants SET status='refresh_2'",
        "DELETE FROM executor_accounts",
    ] {
        sqlx::query(statement).execute(&pool).await.unwrap();
        assert_eq!(
            next(&mut receiver).await,
            Some(Work::Plugins),
            "{statement}"
        );
    }
    sqlx::query("INSERT INTO other_rows VALUES('x')")
        .execute(&pool)
        .await
        .unwrap();
    quiet(&mut receiver).await;

    drop(wakeups);
    for (pool, schema) in [(bare, bare_schema), (other, other_schema), (pool, schema)] {
        pool.close().await;
        sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&admin)
            .await
            .unwrap();
    }
    // The listener task keeps its connection until the process exits.
    drop(listen);
}
