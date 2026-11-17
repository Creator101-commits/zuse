use super::*;
use axum::{
    Json,
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use zuse_cloud::{
    crypto::MasterKey,
    plugins::{PluginConfig, PluginControl, egress::Policy},
};

#[derive(Clone, Default)]
struct Provider {
    origin: String,
    calls: Arc<AtomicUsize>,
    tokens: Arc<AtomicUsize>,
    reject_refresh: Arc<AtomicBool>,
    fail_calls: Arc<AtomicBool>,
    short_lived: Arc<AtomicBool>,
    transient_refresh: Arc<AtomicBool>,
    invalid_init: Arc<AtomicBool>,
    invalid_output: Arc<AtomicUsize>,
    invalid_output_schema: Arc<AtomicBool>,
    looping_catalog: Arc<AtomicBool>,
    catalog_size: Arc<AtomicUsize>,
    catalog_bytes: Arc<AtomicUsize>,
    requests: Arc<AtomicUsize>,
    revocations: Arc<AtomicUsize>,
    require_refresh: Arc<AtomicBool>,
    omit_refresh_token: Arc<AtomicBool>,
    block_refresh: Arc<AtomicBool>,
    refresh_started: Arc<tokio::sync::Notify>,
    release_refresh: Arc<tokio::sync::Notify>,
    streams: Arc<
        std::sync::Mutex<
            std::collections::HashMap<String, tokio::sync::mpsc::UnboundedSender<String>>,
        >,
    >,
}
async fn rpc(
    State(provider): State<Provider>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Response {
    provider.requests.fetch_add(1, Ordering::SeqCst);
    let authorization = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    if provider.reject_refresh.load(Ordering::SeqCst)
        || provider.require_refresh.load(Ordering::SeqCst)
        || !(authorization == "Bearer fixture-secret"
            || authorization.starts_with("Bearer fixture-secret-"))
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if input.get("id").is_none() {
        return StatusCode::ACCEPTED.into_response();
    }
    let result = match input["method"].as_str().unwrap_or_default() {
        "initialize" => {
            if provider.invalid_init.load(Ordering::SeqCst) {
                return Json(json!({"jsonrpc":"2.0","id":input["id"],"result":{}})).into_response();
            }
            json!({"protocolVersion":"2025-06-18", "capabilities":{"tools":{}}, "serverInfo":{"name":"fixture", "version":"1"}})
        }
        "tools/list" => {
            let count = provider.catalog_size.load(Ordering::SeqCst).max(1);
            let tools:Vec<_>=(0..count).map(|i|json!({"name":if count==1{"echo".to_owned()}else{format!("t{i:04}")},"description":if provider.catalog_bytes.load(Ordering::SeqCst)>0 {"x".repeat(provider.catalog_bytes.load(Ordering::SeqCst))}else{"Echo test".into()},"inputSchema":{"type":"object","required":["n"],"properties":{"n":{"type":"integer"}}},"outputSchema":{"type":"object","required":["value"],"properties":{"value":{"type":"string"}}},"annotations":{"readOnlyHint":false}})).collect();
            let mut tools = tools;
            if provider.invalid_output_schema.load(Ordering::SeqCst) {
                tools[0]["outputSchema"] =
                    json!({"type":"object","properties":{"value":{"$ref":"#/missing"}}});
            }
            let mut page = json!({"tools":tools});
            if provider.looping_catalog.load(Ordering::SeqCst) {
                page["nextCursor"] = json!("same");
            }
            page
        }
        "tools/call" => {
            provider.calls.fetch_add(1, Ordering::SeqCst);
            if provider.fail_calls.load(Ordering::SeqCst) {
                return StatusCode::BAD_GATEWAY.into_response();
            }
            if provider.invalid_output.load(Ordering::SeqCst) == 1 {
                json!({"content":"invalid-content","structuredContent":{"value":23}})
            } else if provider.invalid_output.load(Ordering::SeqCst) == 2 {
                json!({"content":[{"type":"text","text":"executed"}],"structuredContent":{"value":23}})
            } else {
                json!({"content":[{"type":"text","text":"executed"}],"structuredContent":{"value":"executed"},"isError":false})
            }
        }
        _ => json!({}),
    };
    Json(json!({"jsonrpc":"2.0", "id":input["id"], "result":result})).into_response()
}
async fn resource(State(p): State<Provider>) -> Json<Value> {
    Json(
        json!({"resource": format!("{}/mcp",p.origin), "authorization_servers":[p.origin], "scopes_supported":["read"]}),
    )
}
async fn metadata(State(p): State<Provider>) -> Json<Value> {
    Json(
        json!({"issuer":p.origin,"authorization_endpoint":format!("{}/authorize",p.origin),"token_endpoint":format!("{}/token",p.origin),"registration_endpoint":format!("{}/register",p.origin),"code_challenge_methods_supported":["S256"],"token_endpoint_auth_methods_supported":["none"],"authorization_response_iss_parameter_supported":true,"revocation_endpoint":format!("{}/revoke",p.origin)}),
    )
}
async fn token(State(p): State<Provider>) -> Response {
    let issued = p.tokens.fetch_add(1, Ordering::SeqCst);
    if p.transient_refresh.load(Ordering::SeqCst) {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    if p.reject_refresh.load(Ordering::SeqCst) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"invalid_grant"})),
        )
            .into_response();
    }
    if issued > 0 && p.block_refresh.load(Ordering::SeqCst) {
        p.refresh_started.notify_one();
        p.release_refresh.notified().await;
    }
    p.require_refresh.store(false, Ordering::SeqCst);
    let mut value = json!({"access_token":format!("fixture-secret-{issued}"),"token_type":"Bearer","expires_in":if p.omit_refresh_token.load(Ordering::SeqCst)||p.short_lived.load(Ordering::SeqCst){30}else if issued==0{1}else{3600}});
    if !p.omit_refresh_token.load(Ordering::SeqCst) {
        value["refresh_token"] = json!(format!("refresh-secret-{issued}"));
    }
    Json(value).into_response()
}
async fn revoke(State(p): State<Provider>) -> Json<Value> {
    p.revocations.fetch_add(1, Ordering::SeqCst);
    Json(json!({}))
}
struct SseLifetime {
    provider: Provider,
    id: String,
}
impl Drop for SseLifetime {
    fn drop(&mut self) {
        self.provider.streams.lock().unwrap().remove(&self.id);
    }
}
async fn legacy_sse(State(provider): State<Provider>, headers: HeaderMap) -> Response {
    if headers.get("authorization").and_then(|h| h.to_str().ok()) != Some("Bearer fixture-secret") {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let id = Uuid::new_v4().to_string();
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    sender
        .send(format!(
            "event: endpoint\ndata: {}/sse/messages?sid={id}\n\n",
            provider.origin
        ))
        .unwrap();
    provider.streams.lock().unwrap().insert(id.clone(), sender);
    let lifetime = SseLifetime { provider, id };
    let stream = futures::stream::unfold(
        (receiver, lifetime),
        |(mut receiver, lifetime)| async move {
            receiver.recv().await.map(|event| {
                (
                    Ok::<_, std::convert::Infallible>(event),
                    (receiver, lifetime),
                )
            })
        },
    );
    let mut response = Response::new(Body::from_stream(stream));
    response
        .headers_mut()
        .insert("content-type", "text/event-stream".parse().unwrap());
    response
}
async fn legacy_message(
    State(provider): State<Provider>,
    axum::extract::Query(query): axum::extract::Query<std::collections::HashMap<String, String>>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Response {
    let response = rpc(State(provider.clone()), headers, Json(input.clone())).await;
    if input.get("id").is_some() && response.status() == StatusCode::OK {
        let bytes = to_bytes(response.into_body(), 2_000_000).await.unwrap();
        if let Some(sender) = query
            .get("sid")
            .and_then(|id| provider.streams.lock().unwrap().get(id).cloned())
        {
            let _ = sender.send(format!(
                "event: message\ndata: {}\n\n",
                String::from_utf8_lossy(&bytes)
            ));
        }
    }
    StatusCode::ACCEPTED.into_response()
}
async fn send(
    app: &Router,
    path: &str,
    actor: &str,
    scope: &str,
    input: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("authorization", format!("Bearer {actor}"))
                .header("x-zuse-workspace", scope)
                .header("content-type", "application/json")
                .header("accept", "application/json, text/event-stream")
                .body(Body::from(input.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 2_000_000).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
#[ignore = "requires isolated localhost PostgreSQL"]
async fn native_plugins_store_credentials_execute_and_isolate_tenants() {
    credentials_execute_and_isolate(false).await;
}

/// The same flows, including OAuth renewal and revocation, while serve and the
/// executor keep rows in memory.
#[tokio::test]
#[ignore = "requires isolated localhost PostgreSQL"]
async fn native_plugins_store_credentials_execute_and_isolate_tenants_with_cached_rows() {
    credentials_execute_and_isolate(true).await;
}

async fn credentials_execute_and_isolate(listen: bool) {
    let (admin, pool, schema) = fixture().await;
    sqlx::raw_sql(zuse_cloud::plugins::MIGRATION)
        .execute(&pool)
        .await
        .unwrap();
    // Migration is safe to apply twice.
    sqlx::raw_sql(zuse_cloud::plugins::MIGRATION)
        .execute(&pool)
        .await
        .unwrap();
    PluginControl::migrate(&pool, &MasterKey::parse(&B64.encode([73; 32])).unwrap())
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let p = Provider {
        origin: format!("http://{}", listener.local_addr().unwrap()),
        calls: Arc::new(AtomicUsize::new(0)),
        tokens: Arc::new(AtomicUsize::new(0)),
        reject_refresh: Arc::new(AtomicBool::new(false)),
        fail_calls: Arc::new(AtomicBool::new(false)),
        ..Default::default()
    };
    let upstream = Router::new()
        .route("/mcp", post(rpc))
        .route("/sse", get(legacy_sse))
        .route("/sse/messages", post(legacy_message))
        .route("/.well-known/oauth-protected-resource/mcp", get(resource))
        .route("/.well-known/oauth-authorization-server", get(metadata))
        .route(
            "/register",
            post(|| async {
                Json(json!({"client_id":"fixture-client","token_endpoint_auth_method":"none"}))
            }),
        )
        .route("/token", post(token))
        .route("/revoke", post(revoke))
        .with_state(p.clone());
    let task = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let host = PluginControl::from_pool(
        pool.clone(),
        PluginConfig {
            key: MasterKey::parse(&B64.encode([73; 32])).unwrap(),
            public_origin: "https://api.test.example".parse().unwrap(),
            app_origin: "https://app.test.example".parse().unwrap(),
            policy: Policy::Loopback,
        },
    )
    .unwrap();
    if listen {
        let wakeups = zuse_cloud::db::wake::Wakeups::listen_with(schema_connection(&schema).await)
            .await
            .unwrap();
        host.attach_wakeups(wakeups).await;
    }
    let signing = ed25519_dalek::SigningKey::from_bytes(&[74; 32]);
    let jwk = json!({"kty":"OKP","crv":"Ed25519","d":B64.encode([74;32]),"x":B64.encode(signing.verifying_key().as_bytes())});
    let runtime = Arc::new(zuse_cloud::runtime::RuntimeControl::from_pool(
        pool.clone(),
        MasterKey::parse(&B64.encode([75; 32])).unwrap(),
        zuse_cloud::tickets::TicketSigner::from_jwk(&jwk.to_string(), "https://api.test.example")
            .unwrap(),
    ));
    let app = router(
        AppState {
            store: Store::from_pool(pool.clone()).with_plugin_control(host.clone()),
            identity: Arc::new(TestIdentity),
            organizations_enabled: true,
            organization_ids: Arc::new(vec!["org-team".into()]),
            runtime: Some(runtime),
            gateway: Arc::new(zuse_cloud::gateway::Hub::default()),
        },
        vec![],
    );
    let tenant = "personal:user-alice";
    let add = json!({"action":"addServer","tenantId":tenant,"requestId":Uuid::new_v4().to_string(),"name":"Fixture","url":format!("{}/mcp",p.origin),"apiKey":{"header":"Authorization","value":"Bearer fixture-secret"}});
    let (status, attempt) = send(&app, "/v1/plugins", "alice", "personal", add.clone()).await;
    assert_eq!(status, StatusCode::OK, "{attempt}");
    assert_eq!(attempt["state"], "connected", "{attempt}");
    assert_eq!(
        send(&app, "/v1/plugins", "alice", "personal", add).await.1,
        attempt,
        "idempotent create"
    );
    let connection = attempt["connectionId"].as_str().unwrap();
    let sealed: String = sqlx::query_scalar(
        "SELECT executor_ref FROM zuse_plugin_connections WHERE tenant_id=$1 AND connection_id=$2",
    )
    .bind(tenant)
    .bind(connection)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!sealed.contains("fixture-secret"));
    let (status, found) = send(
        &app,
        "/v1/plugins/tools",
        "alice",
        "personal",
        json!({"action":"search","query":"echo"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    let address = found[0]["address"].as_str().unwrap();
    let call = json!({"action":"call","address":address,"arguments":{"n":1}});
    let (status, result) = send(&app, "/v1/plugins/tools", "alice", "personal", call.clone()).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["content"][0]["text"], "executed");
    assert_ne!(
        send(&app, "/v1/plugins/tools", "bob", "personal", call.clone())
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(p.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        send(
            &app,
            "/v1/plugins",
            "bob",
            "personal",
            json!({"action":"list","tenantId":tenant})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(send(&app,"/v1/plugins","alice","personal",json!({"action":"setEnabled","tenantId":tenant,"connectionId":connection,"enabled":false})).await.0,StatusCode::OK);
    assert_ne!(
        send(&app, "/v1/plugins/tools", "alice", "personal", call)
            .await
            .0,
        StatusCode::OK
    );
    let org_path = "/v1/organization-workspaces/org-team/v1/plugins";
    assert_eq!(
        send(
            &app,
            org_path,
            "member",
            "organization:org-team",
            json!({"action":"list","tenantId":"organization:org-team"})
        )
        .await
        .1["canManage"],
        false
    );
    assert_eq!(send(&app,org_path,"member","organization:org-team",json!({"action":"disconnect","tenantId":"organization:org-team","connectionId":connection})).await.0,StatusCode::FORBIDDEN);
    assert_eq!(
        send(
            &app,
            org_path,
            "admin",
            "personal",
            json!({"action":"list"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(
            &app,
            org_path,
            "admin",
            "organization:org-team",
            json!({"action":"list","tenantId":tenant})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );

    // No supplied key makes the fixture challenge OAuth; redeem the callback ticket once.
    let (status,oauth)=send(&app,"/v1/plugins","alice","personal",json!({"action":"addServer","tenantId":tenant,"requestId":Uuid::new_v4().to_string(),"name":"OAuth fixture","url":format!("{}/mcp",p.origin),"returnTo":{"kind":"desktop","port":8976}})).await;
    assert_eq!(status, StatusCode::OK, "{oauth}");
    assert_eq!(oauth["state"], "pending", "{oauth}");
    let auth: reqwest::Url = oauth["authorizationUrl"].as_str().unwrap().parse().unwrap();
    let state = auth
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    assert!(
        auth.query_pairs()
            .any(|(k, v)| k == "code_challenge_method" && v == "S256")
    );
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("state", &state)
        .append_pair("code", "fixture-code")
        .append_pair("iss", &p.origin)
        .finish();
    let target = host.callback(&query).await.unwrap();
    assert_eq!(target.port(), Some(8976));
    assert!(host.callback(&query).await.is_err(), "callback replay");
    let ticket = target
        .query_pairs()
        .find(|(k, _)| k == "plugin_ticket")
        .unwrap()
        .1
        .into_owned();
    let complete = json!({"action":"complete","tenantId":tenant,"ticket":ticket});
    assert_ne!(
        send(&app, "/v1/plugins", "bob", "personal", complete.clone())
            .await
            .0,
        StatusCode::OK
    );
    let (status, result) = send(&app, "/v1/plugins", "alice", "personal", complete.clone()).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["state"], "connected", "{result}");
    assert_ne!(
        send(&app, "/v1/plugins", "alice", "personal", complete)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(p.tokens.load(Ordering::SeqCst), 1, "code exchanged once");
    let oauth_connection = result["connectionId"].as_str().unwrap();
    let (status, result) = send(
        &app,
        "/v1/plugins/tools",
        "alice",
        "personal",
        json!({"action":"search","query":"echo"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(
        p.tokens.load(Ordering::SeqCst),
        2,
        "SDK renewed short-lived grant"
    );
    let credentials: Vec<Vec<u8>> =
        sqlx::query_scalar("SELECT encrypted FROM executor_oauth_grants")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(!credentials.is_empty());
    assert!(
        credentials
            .iter()
            .all(|bytes| !String::from_utf8_lossy(bytes).contains("fixture-secret"))
    );

    // Runtime authentication derives its owner from the fenced workspace, ignoring tenant headers.
    use sha2::{Digest, Sha256};
    let runtime_hash: String = Sha256::digest(b"runtime-fixture")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let at = chrono::Utc::now().timestamp_millis();
    sqlx::query("INSERT INTO zuse_rust_control_owners VALUES('workspace','ws-alice','rust',2,$1)")
        .bind(at)
        .execute(&pool)
        .await
        .unwrap();
    let config = json!({"runtimeGeneration":2,"runtimeRpcProtocol":"zuse-rpc-v1","runtimeCredentialExpiresAtMs":at+60_000,"runtimeBootstrapReceipt":{"workspaceId":"ws-alice","generation":2,"gatewayEpoch":2}});
    sqlx::query("UPDATE api_cloud_workspaces SET runtime_credential_hash=$1,request_config=$2,state='ready',desired_state='ready' WHERE workspace_id='ws-alice'").bind(runtime_hash).bind(&config).execute(&pool).await.unwrap();
    let runtime_path = "/v1/plugins/runtime/ws-alice/tools";
    let (status, list) = send(
        &app,
        runtime_path,
        "runtime-fixture",
        "organization:org-team",
        json!({"action":"list"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(
        list.as_array().unwrap().len(),
        1,
        "Only Alice's enabled OAuth connection"
    );
    assert_eq!(list[0]["connectionId"], oauth_connection);
    assert_eq!(
        send(
            &app,
            runtime_path,
            "alice",
            "personal",
            json!({"action":"list"})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(
            &app,
            "/v1/plugins/runtime/ws-bob/tools",
            "runtime-fixture",
            "personal",
            json!({"action":"list"})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("UPDATE api_cloud_workspaces SET request_config=jsonb_set(request_config,'{runtimeCredentialExpiresAtMs}',$1) WHERE workspace_id='ws-alice'").bind(json!(at-1)).execute(&pool).await.unwrap();
    assert_eq!(
        send(
            &app,
            runtime_path,
            "runtime-fixture",
            "personal",
            json!({"action":"list"})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("UPDATE api_cloud_workspaces SET request_config=$1,desired_state='deleted' WHERE workspace_id='ws-alice'").bind(config).execute(&pool).await.unwrap();
    assert_eq!(
        send(
            &app,
            runtime_path,
            "runtime-fixture",
            "personal",
            json!({"action":"list"})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );

    // A failure after the provider receives a mutation must not replay the call.
    p.fail_calls.store(true, Ordering::SeqCst);
    let calls = p.calls.load(Ordering::SeqCst);
    let address = format!(
        "tools.{}.user.{oauth_connection}.echo",
        oauth["pluginId"].as_str().unwrap()
    );
    let (status, result) = send(
        &app,
        "/v1/plugins/tools",
        "alice",
        "personal",
        json!({"action":"call","address":address,"arguments":{"n":1}}),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "{result}");
    assert_eq!(
        p.calls.load(Ordering::SeqCst),
        calls + 1,
        "no mutation retries"
    );
    p.fail_calls.store(false, Ordering::SeqCst);

    p.reject_refresh.store(true, Ordering::SeqCst);
    assert_ne!(
        send(
            &app,
            "/v1/plugins/tools",
            "alice",
            "personal",
            json!({"action":"call","address":address,"arguments":{"n":1}})
        )
        .await
        .0,
        StatusCode::OK
    );
    let state: String = sqlx::query_scalar(
        "SELECT state FROM zuse_plugin_connections WHERE tenant_id=$1 AND connection_id=$2",
    )
    .bind(tenant)
    .bind(oauth_connection)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, "needs-auth");
    let (_,pending)=send(&app,"/v1/plugins","alice","personal",json!({"action":"connect","tenantId":tenant,"pluginId":oauth["pluginId"],"requestId":Uuid::new_v4().to_string(),"label":"Expires"})).await;
    assert_eq!(pending["state"], "pending", "{pending}");
    sqlx::query("UPDATE zuse_plugin_attempts SET expires_at=$1 WHERE attempt_id=$2")
        .bind(at - 1)
        .bind(pending["id"].as_str().unwrap())
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(host.sweep().await.unwrap(), 1);
    let (_, cancelled) = send(
        &app,
        "/v1/plugins",
        "alice",
        "personal",
        json!({"action":"poll","tenantId":tenant,"attemptId":pending["id"]}),
    )
    .await;
    assert_eq!(cancelled["state"], "cancelled");
    task.abort();
    pool.close().await;
    sqlx::raw_sql(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
}

fn configuration() -> PluginConfig {
    PluginConfig {
        key: MasterKey::parse(&B64.encode([73; 32])).unwrap(),
        public_origin: "https://api.test.example".parse().unwrap(),
        app_origin: "https://app.test.example".parse().unwrap(),
        policy: Policy::Loopback,
    }
}
fn application(pool: &PgPool, host: Arc<PluginControl>) -> Router {
    router(
        AppState {
            store: Store::from_pool(pool.clone()).with_plugin_control(host),
            identity: Arc::new(TestIdentity),
            organizations_enabled: false,
            organization_ids: Arc::new(vec![]),
            runtime: None,
            gateway: Arc::new(zuse_cloud::gateway::Hub::default()),
        },
        vec!["https://app.test.example".parse().unwrap()],
    )
}
async fn oauth_connection(app: &Router, p: &Provider, host: &Arc<PluginControl>) -> Value {
    let (_,pending)=send(app,"/v1/plugins","alice","personal",json!({"action":"addServer","tenantId":"personal:user-alice","requestId":Uuid::new_v4().to_string(),"name":"OAuth","url":format!("{}/mcp",p.origin)})).await;
    assert_eq!(pending["state"], "pending", "{pending}");
    let auth: reqwest::Url = pending["authorizationUrl"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        !auth.query_pairs().any(|(key, _)| key == "scope"),
        "undeclared scopes are not broadened"
    );
    let state = auth
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("state", &state)
        .append_pair("code", "fixture-code")
        .append_pair("iss", &p.origin)
        .finish();
    let target = host.callback(&query).await.unwrap();
    let ticket = target
        .query_pairs()
        .find(|(k, _)| k == "plugin_ticket")
        .unwrap()
        .1
        .into_owned();
    let (_, result) = send(
        app,
        "/v1/plugins",
        "alice",
        "personal",
        json!({"action":"complete","tenantId":"personal:user-alice","ticket":ticket}),
    )
    .await;
    assert_eq!(result["state"], "connected", "{result}");
    result
}

#[tokio::test]
#[ignore = "requires isolated localhost PostgreSQL and Bun"]
async fn native_plugins_executor_validates_protocol_schemas_and_lifecycle() {
    let (admin, pool, schema) = fixture().await;
    PluginControl::migrate(&pool, &configuration().key)
        .await
        .unwrap();
    // Exercise the actual SDK migration twice, rather than only the metadata SQL.
    PluginControl::migrate(&pool, &configuration().key)
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let p = Provider {
        origin: format!("http://{}", listener.local_addr().unwrap()),
        ..Default::default()
    };
    let upstream = Router::new()
        .route("/mcp", post(rpc))
        .route("/sse", get(legacy_sse))
        .route("/sse/messages", post(legacy_message))
        .route("/.well-known/oauth-protected-resource/mcp", get(resource))
        .route("/.well-known/oauth-authorization-server", get(metadata))
        .route(
            "/register",
            post(|| async {
                Json(json!({"client_id":"fixture","token_endpoint_auth_method":"none"}))
            }),
        )
        .route("/token", post(token))
        .route("/revoke", post(revoke))
        .with_state(p.clone());
    let task = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let host = PluginControl::from_pool(pool.clone(), configuration()).unwrap();
    let app = application(&pool, host.clone());
    let (_,connection)=send(&app,"/v1/plugins","alice","personal",json!({"action":"addServer","tenantId":"personal:user-alice","requestId":Uuid::new_v4().to_string(),"name":"Validation","url":format!("{}/mcp",p.origin),"apiKey":{"header":"Authorization","value":"Bearer fixture-secret"}})).await;
    assert_eq!(connection["state"], "connected", "{connection}");
    let id = connection["connectionId"].as_str().unwrap();
    let plugin = connection["pluginId"].as_str().unwrap();
    let address = format!("tools.{plugin}.user.{id}.echo");
    let (status, descriptor) = send(
        &app,
        "/v1/plugins/tools",
        "alice",
        "personal",
        json!({"action":"schema","address":address}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{descriptor}");
    assert!(descriptor["app"].is_string());
    assert!(descriptor["deployment"].is_string());
    assert_eq!(descriptor["readOnly"], false);
    let before = p.calls.load(Ordering::SeqCst);
    for arguments in [json!({"n":"wrong"}), json!({})] {
        assert_ne!(
            send(
                &app,
                "/v1/plugins/tools",
                "alice",
                "personal",
                json!({"action":"call","address":address,"arguments":arguments})
            )
            .await
            .0,
            StatusCode::OK
        );
    }
    assert_eq!(
        p.calls.load(Ordering::SeqCst),
        before,
        "invalid input never reaches provider"
    );
    let hidden = format!("tools.{plugin}.user.{id}.not-listed");
    assert_ne!(
        send(
            &app,
            "/v1/plugins/tools",
            "alice",
            "personal",
            json!({"action":"call","address":hidden,"arguments":{"n":1}})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        p.calls.load(Ordering::SeqCst),
        before,
        "unlisted tools never execute"
    );
    for mode in [1, 2] {
        p.invalid_output.store(mode, Ordering::SeqCst);
        assert_ne!(
            send(
                &app,
                "/v1/plugins/tools",
                "alice",
                "personal",
                json!({"action":"call","address":address,"arguments":{"n":1}})
            )
            .await
            .0,
            StatusCode::OK
        );
        assert_eq!(
            p.calls.load(Ordering::SeqCst),
            before + mode,
            "invalid output does not replay mutation"
        );
    }
    p.invalid_output.store(0, Ordering::SeqCst);
    p.invalid_output_schema.store(true, Ordering::SeqCst);
    let calls = p.calls.load(Ordering::SeqCst);
    assert_ne!(
        send(
            &app,
            "/v1/plugins/tools",
            "alice",
            "personal",
            json!({"action":"call","address":address,"arguments":{"n":1}})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        p.calls.load(Ordering::SeqCst),
        calls,
        "unusable output schema is rejected before mutation"
    );
    p.invalid_output_schema.store(false, Ordering::SeqCst);
    p.looping_catalog.store(true, Ordering::SeqCst);
    assert_ne!(
        send(
            &app,
            "/v1/plugins/tools",
            "alice",
            "personal",
            json!({"action":"schema","address":address})
        )
        .await
        .0,
        StatusCode::OK
    );
    p.looping_catalog.store(false, Ordering::SeqCst);
    p.invalid_init.store(true, Ordering::SeqCst);
    assert_ne!(
        send(
            &app,
            "/v1/plugins/tools",
            "alice",
            "personal",
            json!({"action":"schema","address":address})
        )
        .await
        .0,
        StatusCode::OK
    );
    p.invalid_init.store(false, Ordering::SeqCst);
    p.catalog_size.store(1001, Ordering::SeqCst);
    let (status, found) = send(
        &app,
        "/v1/plugins/tools",
        "alice",
        "personal",
        json!({"action":"search","query":"t1000"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(
        found.as_array().unwrap().len(),
        1,
        "discovery is not silently truncated at 1000 tools"
    );
    p.catalog_size.store(0, Ordering::SeqCst);
    p.catalog_bytes.store(9 * 1024 * 1024, Ordering::SeqCst);
    assert_ne!(
        send(
            &app,
            "/v1/plugins/tools",
            "alice",
            "personal",
            json!({"action":"schema","address":address})
        )
        .await
        .0,
        StatusCode::OK,
        "oversized catalog fails within the aggregate budget"
    );
    p.catalog_bytes.store(0, Ordering::SeqCst);
    let old:String=sqlx::query_scalar("SELECT sealed FROM zuse_plugin_servers WHERE tenant_id='personal:user-alice' AND server_id=$1").bind(plugin).fetch_one(&pool).await.unwrap();
    let key = configuration().key;
    let aad = format!("zuse-plugins:v1:server:personal:user-alice:{plugin}");
    let mut changed: Value =
        serde_json::from_slice(&key.open(aad.as_bytes(), &old).unwrap()).unwrap();
    changed["endpoint"] = json!("http://127.0.0.1:1/steal");
    let sealed = key
        .seal(aad.as_bytes(), &serde_json::to_vec(&changed).unwrap())
        .unwrap();
    sqlx::query("UPDATE zuse_plugin_servers SET sealed=$1 WHERE tenant_id='personal:user-alice' AND server_id=$2").bind(sealed).bind(plugin).execute(&pool).await.unwrap();
    let requests = p.requests.load(Ordering::SeqCst);
    assert_eq!(
        send(
            &app,
            "/v1/plugins/tools",
            "alice",
            "personal",
            json!({"action":"call","address":address,"arguments":{"n":1}})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        p.requests.load(Ordering::SeqCst),
        requests,
        "changed definition gets no credentials or requests"
    );
    sqlx::query("UPDATE zuse_plugin_servers SET sealed=$1 WHERE tenant_id='personal:user-alice' AND server_id=$2").bind(old).bind(plugin).execute(&pool).await.unwrap();
    assert_eq!(send(&app,"/v1/plugins","alice","personal",json!({"action":"rename","tenantId":"personal:user-alice","connectionId":id,"label":"Renamed"})).await.0,StatusCode::OK);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/plugins/tools")
                .header("authorization", "Bearer alice")
                .header("origin", "https://evil.example")
                .header("content-type", "application/json")
                .body(Body::from("{\"action\":\"list\"}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/v1/plugins/personal:user-alice/mcp")
                .header("origin", "https://app.test.example")
                .header("access-control-request-method", "POST")
                .header(
                    "access-control-request-headers",
                    "authorization,content-type,mcp-protocol-version",
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.headers()["access-control-allow-origin"],
        "https://app.test.example"
    );
    assert!(
        response.headers()["access-control-allow-headers"]
            .to_str()
            .unwrap()
            .contains("mcp-protocol-version")
    );
    let (status, result) = send(
        &app,
        "/v1/plugins/personal:user-alice/mcp",
        "alice",
        "personal",
        json!({"jsonrpc":"WRONG","id":{},"method":"tools/list"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{result}");
    let (status, result) = send(
        &app,
        "/v1/plugins/personal:user-alice/mcp",
        "alice",
        "personal",
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["result"]["tools"].as_array().unwrap().len(), 4);
    // A restarted worker uses the same SDK app/deployment and sealed source store.
    let restarted = PluginControl::from_pool(pool.clone(), configuration()).unwrap();
    let restarted_app = application(&pool, restarted.clone());
    let (_, after) = send(
        &restarted_app,
        "/v1/plugins/tools",
        "alice",
        "personal",
        json!({"action":"schema","address":address}),
    )
    .await;
    assert_eq!(after["app"], descriptor["app"]);
    assert_eq!(after["deployment"], descriptor["deployment"]);
    drop(restarted_app);
    drop(restarted);
    let (_,legacy)=send(&app,"/v1/plugins","alice","personal",json!({"action":"addServer","tenantId":"personal:user-alice","requestId":Uuid::new_v4().to_string(),"name":"Legacy SSE","url":format!("{}/sse",p.origin),"apiKey":{"header":"Authorization","value":"Bearer fixture-secret"}})).await;
    assert_eq!(legacy["state"], "connected", "{legacy}");
    let legacy_address = format!(
        "tools.{}.user.{}.echo",
        legacy["pluginId"].as_str().unwrap(),
        legacy["connectionId"].as_str().unwrap()
    );
    let (status, result) = send(
        &app,
        "/v1/plugins/tools",
        "alice",
        "personal",
        json!({"action":"call","address":legacy_address,"arguments":{"n":1}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !p.streams.lock().unwrap().is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    // An early renewal outage retains the still-valid token and leaves the worker usable.
    p.short_lived.store(true, Ordering::SeqCst);
    let short = oauth_connection(&app, &p, &host).await;
    p.short_lived.store(false, Ordering::SeqCst);
    p.transient_refresh.store(true, Ordering::SeqCst);
    let short_address = format!(
        "tools.{}.user.{}.echo",
        short["pluginId"].as_str().unwrap(),
        short["connectionId"].as_str().unwrap()
    );
    let (status, result) = send(
        &app,
        "/v1/plugins/tools",
        "alice",
        "personal",
        json!({"action":"call","address":short_address,"arguments":{"n":1}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    p.transient_refresh.store(false, Ordering::SeqCst);
    let (status, result) = send(
        &app,
        "/v1/plugins/tools",
        "alice",
        "personal",
        json!({"action":"call","address":short_address,"arguments":{"n":1}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    // Nonrenewable grants stay usable through their actual expiry.
    p.omit_refresh_token.store(true, Ordering::SeqCst);
    let unexpired = oauth_connection(&app, &p, &host).await;
    p.omit_refresh_token.store(false, Ordering::SeqCst);
    let unexpired_address = format!(
        "tools.{}.user.{}.echo",
        unexpired["pluginId"].as_str().unwrap(),
        unexpired["connectionId"].as_str().unwrap()
    );
    let tokens = p.tokens.load(Ordering::SeqCst);
    let (status, result) = send(
        &app,
        "/v1/plugins/tools",
        "alice",
        "personal",
        json!({"action":"call","address":unexpired_address,"arguments":{"n":1}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(p.tokens.load(Ordering::SeqCst), tokens);
    // Lifecycle serialization spans token renewal and the actual provider mutation.
    let oauth = oauth_connection(&app, &p, &host).await;
    let oauth_address = format!(
        "tools.{}.user.{}.echo",
        oauth["pluginId"].as_str().unwrap(),
        oauth["connectionId"].as_str().unwrap()
    );
    p.require_refresh.store(true, Ordering::SeqCst);
    p.block_refresh.store(true, Ordering::SeqCst);
    let caller_app = app.clone();
    let caller = tokio::spawn(async move {
        send(
            &caller_app,
            "/v1/plugins/tools",
            "alice",
            "personal",
            json!({"action":"call","address":oauth_address,"arguments":{"n":1}}),
        )
        .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        p.refresh_started.notified(),
    )
    .await
    .unwrap();
    let before_cancelled_call = p.calls.load(Ordering::SeqCst);
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    let deleting_app = application(
        &pool,
        PluginControl::from_pool(pool.clone(), configuration()).unwrap(),
    );
    let oauth_id = oauth["connectionId"].clone();
    let deleting = tokio::spawn(async move {
        send(
            &deleting_app,
            "/v1/plugins",
            "alice",
            "personal",
            json!({"action":"disconnect","tenantId":"personal:user-alice","connectionId":oauth_id}),
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        !deleting.is_finished(),
        "disconnect cannot report success ahead of the in-flight call"
    );
    p.release_refresh.notify_one();
    assert_eq!(deleting.await.unwrap().0, StatusCode::OK);
    assert_eq!(
        p.calls.load(Ordering::SeqCst),
        before_cancelled_call + 1,
        "the cancelled caller cannot drop its worker fence"
    );
    let before = p.calls.load(Ordering::SeqCst);
    let removed_address = format!(
        "tools.{}.user.{}.echo",
        oauth["pluginId"].as_str().unwrap(),
        oauth["connectionId"].as_str().unwrap()
    );
    assert_ne!(
        send(
            &app,
            "/v1/plugins/tools",
            "alice",
            "personal",
            json!({"action":"call","address":removed_address,"arguments":{"n":1}})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        p.calls.load(Ordering::SeqCst),
        before,
        "no invocation can start after disconnect success"
    );
    assert!(
        p.revocations.load(Ordering::SeqCst) > 0,
        "disconnect attempts provider-side revocation"
    );
    drop(app);
    drop(host);
    task.abort();
    pool.close().await;
    sqlx::raw_sql(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires isolated localhost PostgreSQL"]
async fn native_plugins_cached_views_follow_changes_and_tenants() {
    let (admin, pool, schema) = fixture().await;
    PluginControl::migrate(&pool, &configuration().key)
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let p = Provider {
        origin: format!("http://{}", listener.local_addr().unwrap()),
        ..Default::default()
    };
    let upstream = Router::new().route("/mcp", post(rpc)).with_state(p.clone());
    let task = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let host = PluginControl::from_pool(pool.clone(), configuration()).unwrap();
    // A dedicated LISTEN connection on the test schema, as `serve` opens one.
    let scope = schema.clone();
    let listen = PgPoolOptions::new()
        .max_connections(1)
        .after_connect(move |connection, _| {
            let scope = scope.clone();
            Box::pin(async move {
                sqlx::query(&format!("SET search_path = {scope}"))
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&std::env::var("ZUSE_CLOUD_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let wakeups = zuse_cloud::db::wake::Wakeups::listen_with(listen)
        .await
        .unwrap();
    host.attach_wakeups(wakeups.clone()).await;
    let app = application(&pool, host.clone());
    let tenant = "personal:user-alice";
    let (_, attempt) = send(&app, "/v1/plugins", "alice", "personal", json!({"action":"addServer","tenantId":tenant,"requestId":Uuid::new_v4().to_string(),"name":"Cached","url":format!("{}/mcp",p.origin),"apiKey":{"header":"Authorization","value":"Bearer fixture-secret"}})).await;
    assert_eq!(attempt["state"], "connected", "{attempt}");
    let connection = attempt["connectionId"].as_str().unwrap().to_owned();
    let (status, found) = send(
        &app,
        "/v1/plugins/tools",
        "alice",
        "personal",
        json!({"action":"search","query":"echo"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    let call = json!({"action":"call","address":found[0]["address"],"arguments":{"n":1}});
    let calls = |actor: &'static str, scope: &'static str| {
        let app = app.clone();
        let call = call.clone();
        async move { send(&app, "/v1/plugins/tools", actor, scope, call).await.0 }
    };
    assert_eq!(calls("alice", "personal").await, StatusCode::OK);
    // A cached view never serves another tenant.
    assert_ne!(calls("bob", "personal").await, StatusCode::OK);
    assert_ne!(
        calls("admin", "organization:org-team").await,
        StatusCode::OK
    );
    assert_eq!(calls("alice", "personal").await, StatusCode::OK);

    // A change made by another process arrives as NOTIFY and drops the view.
    sqlx::query("UPDATE zuse_plugin_connections SET enabled=false WHERE connection_id=$1")
        .bind(&connection)
        .execute(&schema_connection(&schema).await)
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while calls("alice", "personal").await == StatusCode::OK {
        assert!(
            tokio::time::Instant::now() < deadline,
            "notification did not invalidate the view"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // This process's own writes take effect on the very next call.
    let enable = |enabled: bool| json!({"action":"setEnabled","tenantId":tenant,"connectionId":connection,"enabled":enabled});
    assert_eq!(
        send(&app, "/v1/plugins", "alice", "personal", enable(true))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(calls("alice", "personal").await, StatusCode::OK);
    assert_eq!(
        send(&app, "/v1/plugins", "alice", "personal", enable(false))
            .await
            .0,
        StatusCode::OK
    );
    assert_ne!(calls("alice", "personal").await, StatusCode::OK);
    let mut notifications = wakeups.subscribe();
    assert_eq!(
        send(&app, "/v1/plugins", "alice", "personal", enable(true))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(calls("alice", "personal").await, StatusCode::OK);
    // Let this write's own NOTIFY arrive (it drops the view once more).
    while tokio::time::timeout(std::time::Duration::from_secs(5), notifications.recv())
        .await
        .expect("own write notifies")
        .unwrap()
        != zuse_cloud::db::wake::Work::Plugins
    {}

    // Views really are served from memory: a change that sends no NOTIFY is
    // not seen until notifications may have been missed (listener reconnect).
    assert_eq!(calls("alice", "personal").await, StatusCode::OK);
    let direct = schema_connection(&schema).await;
    sqlx::raw_sql("ALTER TABLE zuse_plugin_connections DISABLE TRIGGER zuse_cloud_plugin_connections_wake; UPDATE zuse_plugin_connections SET enabled=false; ALTER TABLE zuse_plugin_connections ENABLE TRIGGER zuse_cloud_plugin_connections_wake")
        .execute(&direct)
        .await
        .unwrap();
    assert_eq!(calls("alice", "personal").await, StatusCode::OK);
    wakeups.notify(zuse_cloud::db::wake::Work::All);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while calls("alice", "personal").await == StatusCode::OK {
        assert!(
            tokio::time::Instant::now() < deadline,
            "reconnect kept views"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let mut notifications = wakeups.subscribe();
    assert_eq!(
        send(&app, "/v1/plugins", "alice", "personal", enable(true))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(calls("alice", "personal").await, StatusCode::OK);

    // The executor keeps the SDK rows a call reads, too: a change that sends
    // no NOTIFY stays unseen until serve forwards a new epoch.
    while tokio::time::timeout(std::time::Duration::from_secs(5), notifications.recv())
        .await
        .expect("own write notifies")
        .unwrap()
        != zuse_cloud::db::wake::Work::Plugins
    {}
    assert_eq!(calls("alice", "personal").await, StatusCode::OK);
    sqlx::raw_sql("ALTER TABLE executor_installations DISABLE TRIGGER zuse_cloud_executor_wake; UPDATE executor_installations SET enabled=false; ALTER TABLE executor_installations ENABLE TRIGGER zuse_cloud_executor_wake")
        .execute(&direct)
        .await
        .unwrap();
    assert_eq!(
        calls("alice", "personal").await,
        StatusCode::OK,
        "served from the executor's kept rows"
    );
    wakeups.notify(zuse_cloud::db::wake::Work::All);
    assert_ne!(
        calls("alice", "personal").await,
        StatusCode::OK,
        "a new epoch drops the executor's rows"
    );
    // With its trigger, an SDK row change reaches the executor via serve.
    sqlx::query("UPDATE executor_installations SET enabled=true")
        .execute(&direct)
        .await
        .unwrap();
    let started = tokio::time::Instant::now();
    while calls("alice", "personal").await != StatusCode::OK {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "SDK change did not reach the executor"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    eprintln!(
        "sdk_row_change_visible_ms={}",
        started.elapsed().as_millis()
    );

    // A second serve process (own executor, own LISTEN) changes the connection:
    // this process rejects the next calls within notification latency.
    let other = PluginControl::from_pool(pool.clone(), configuration()).unwrap();
    let other_wakeups =
        zuse_cloud::db::wake::Wakeups::listen_with(schema_connection(&schema).await)
            .await
            .unwrap();
    other.attach_wakeups(other_wakeups).await;
    let other_app = application(&pool, other.clone());
    assert_eq!(calls("alice", "personal").await, StatusCode::OK);
    assert_eq!(
        send(
            &other_app,
            "/v1/plugins",
            "alice",
            "personal",
            enable(false)
        )
        .await
        .0,
        StatusCode::OK
    );
    let started = tokio::time::Instant::now();
    while calls("alice", "personal").await == StatusCode::OK {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "another process's change was not seen"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    eprintln!(
        "cross_process_disable_visible_ms={}",
        started.elapsed().as_millis()
    );
    assert_eq!(
        send(&other_app, "/v1/plugins", "alice", "personal", enable(true))
            .await
            .0,
        StatusCode::OK
    );
    let started = tokio::time::Instant::now();
    while calls("alice", "personal").await != StatusCode::OK {
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    drop(other_app);
    drop(other);

    // Disconnecting revokes at once: the very next call is rejected.
    assert_eq!(calls("alice", "personal").await, StatusCode::OK);
    assert_eq!(
        send(
            &app,
            "/v1/plugins",
            "alice",
            "personal",
            json!({"action":"disconnect","tenantId":tenant,"connectionId":connection})
        )
        .await
        .0,
        StatusCode::OK
    );
    let started = tokio::time::Instant::now();
    assert_ne!(calls("alice", "personal").await, StatusCode::OK);
    eprintln!(
        "disconnect_next_call_rejected_ms={}",
        started.elapsed().as_millis()
    );
    task.abort();
    drop(direct);
    pool.close().await;
    sqlx::raw_sql(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
}

/// A separate connection on the test schema, standing in for another process.
async fn schema_connection(schema: &str) -> PgPool {
    let scope = schema.to_owned();
    PgPoolOptions::new()
        .max_connections(1)
        .after_connect(move |connection, _| {
            let scope = scope.clone();
            Box::pin(async move {
                sqlx::query(&format!("SET search_path = {scope}"))
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&std::env::var("ZUSE_CLOUD_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap()
}
