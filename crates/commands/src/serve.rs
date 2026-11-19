//! `zuse serve`: the browser client and a local engine in one process.
use crate::{cli::ServeArgs, remote};
use anyhow::Context;
use serde_json::json;
use std::path::PathBuf;
use zuse_engine::{Engine, EngineConfig, EngineProfile, InstanceLock};
use zuse_i18n::{Localizer, messages::cli as m};
use zuse_protocol::HarnessId;
use zuse_serve::{BrowserServer, TlsConfig};

pub fn main(args: ServeArgs, ipc_port: Option<u16>, l10n: &Localizer) -> anyhow::Result<()> {
    let ipc_port = ipc_port.or_else(|| std::env::var("ZUSE_IPC_PORT").ok()?.parse().ok());
    tokio::runtime::Builder::new_multi_thread()
        .thread_stack_size(zuse_engine::RUNTIME_STACK_SIZE)
        .enable_all()
        .build()?
        .block_on(serve(args, ipc_port, l10n))
}

async fn serve(
    args: ServeArgs,
    requested_port: Option<u16>,
    l10n: &Localizer,
) -> anyhow::Result<()> {
    let relay = match (&args.relay_origin, &args.relay_host_token_file) {
        (Some(origin), Some(path)) => {
            Some((zuse_serve::relay::validate_origin(origin)?, path.clone()))
        }
        (None, None) => None,
        _ => anyhow::bail!(m::relay_requires_credentials(l10n)),
    };
    let shutdown = tokio_util::sync::CancellationToken::new();
    let tls = match (&args.tls_cert, &args.tls_key) {
        (Some(certificate), Some(key)) => Some(TlsConfig::from_pem_files(certificate, key)?),
        (None, None) => None,
        _ => anyhow::bail!(m::https_requires_keys(l10n)),
    };
    anyhow::ensure!(
        args.listen.ip().is_loopback() || tls.is_some() || args.public_origin.is_some(),
        m::https_required(l10n)
    );
    let data_dir = match &args.data_dir {
        Some(path) => path.clone(),
        None => crate::paths::default_data_dir().context(m::missing_home(l10n))?,
    };
    std::fs::create_dir_all(&data_dir).context(m::create_data_failed(l10n))?;
    let data_dir = std::fs::canonicalize(data_dir).context(m::resolve_data_failed(l10n))?;
    // Decide profile ownership before ports, auth initialization or store opens.
    let lock = match InstanceLock::acquire(&data_dir) {
        Ok(lock) => lock,
        Err(error) if InstanceLock::holder(&data_dir).is_some() => {
            let port = requested_port
                .or_else(|| recorded_ipc_port(&data_dir))
                .unwrap_or(crate::DEFAULT_IPC_PORT);
            return reuse_engine(&args, &data_dir, port, l10n)
                .await.with_context(|| format!("{error}. Could not enable browser access on that engine; use its --ipc-port, or choose a separate --data-dir"));
        }
        Err(error) => return Err(error.into()),
    };
    // Bind before starting engine workers, so port conflicts leave no live runtime.
    let ipc = bind_ipc(requested_port, crate::DEFAULT_IPC_PORT)
        .await
        .context(m::bind_failed(l10n))?;
    let ipc_port = ipc.local_addr()?.port();
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    let address = listener.local_addr()?;
    let token = match args.token {
        Some(token) => token,
        None => BrowserServer::load_or_create_token(&data_dir)?,
    };
    let assets = args
        .assets
        .or_else(zuse_engine::browser_access::discover_assets)
        .unwrap_or_else(|| {
            eprintln!("{}", m::assets_missing(l10n));
            PathBuf::from("dist/web")
        });

    let config = EngineConfig {
        data_dir: data_dir.clone(),
        edge_url: "http://127.0.0.1:27655".into(),
        edge_token: None,
        ipc_port,
        default_harness: HarnessId::ClaudeCode,
        org_id: None,
        workos_client_id: None,
    };
    let auth = Engine::build_auth(&config).await;
    let runtime =
        Engine::assemble_runtime_with_lock(&config, auth, EngineProfile::local(&data_dir)?, lock)
            .await?;
    let _port_record = match IpcPortRecord::publish(&data_dir, ipc_port) {
        Ok(record) => record,
        Err(error) => {
            runtime.shutdown().await;
            return Err(error).context("Could not publish this profile's RPC port");
        }
    };
    // Agent-origin attribution and plugin tools must dial this engine's real port.
    runtime.core().sessions.set_ipc_port(ipc_port);
    let browser = match BrowserServer::new(
        runtime.core().rpc_service(),
        token.clone(),
        args.public_origin.clone(),
    ) {
        Ok(browser) => browser
            .with_managed_authority(std::sync::Arc::new(runtime.core().managed_host.control()))
            .with_pairings(
                runtime.core().pairings.clone(),
                runtime.core().device_id.clone(),
            )
            .with_pairing_slots(runtime.core().browser_access.control().pairing_slots())
            .with_paired_sessions(runtime.core().browser_access.control().paired_sessions())
            .with_advertised_routes(
                runtime
                    .core()
                    .browser_access
                    .control()
                    .advertised_routes()
                    .subscribe(),
            )
            .with_shutdown_token(shutdown.clone()),
        Err(error) => {
            runtime.shutdown().await;
            return Err(error);
        }
    };
    let ipc_task = tokio::spawn(zuse_rpc::serve_ws_listener(
        ipc,
        runtime.core().rpc_service(),
    ));
    let public_origin = args.public_origin.is_some();
    let origin = args.public_origin.unwrap_or_else(|| {
        format!(
            "{}://{address}",
            if tls.is_some() { "https" } else { "http" }
        )
    });
    runtime
        .core()
        .browser_access
        .control()
        .register_external(address, origin.clone(), tls.is_none())
        .await;
    println!(
        "{}",
        remote::plain(m::listening(l10n, ipc_port.to_string()))
    );
    println!(
        "{}",
        remote::plain(m::open(l10n, format!("{origin}/#token={token}")))
    );
    if args.lan {
        // The LAN and Tailscale listeners serve the same app as this one.
        runtime
            .core()
            .direct_access
            .control()
            .use_app_assets(assets.clone());
        match runtime
            .core()
            .direct_access
            .control()
            .start(
                zuse_engine::direct_access::Start {
                    port: args.lan_port,
                    tailscale_serve: args.tailscale_serve,
                    // `--lan` offers every private network this host is on.
                    lan: Some(true),
                    tailnet: Some(true),
                },
                runtime.core().rpc_service(),
            )
            .await
        {
            Ok(status) => {
                for route in status.routes {
                    println!("{}: {}", route.kind.label(), route.origin);
                }
            }
            Err(error) => {
                ipc_task.abort();
                runtime.shutdown().await;
                return Err(error);
            }
        }
    }
    let relayed = relay.is_some();
    if let Some((origin, host_token_file)) = relay {
        // Explicit command-line configuration replaces saved startup settings.
        runtime.core().relay_access.control().stop().await;
        if let Err(error) = runtime
            .core()
            .relay_access
            .control()
            .start(
                zuse_engine::relay_access::Start {
                    origin,
                    host_token_file,
                },
                runtime.core().rpc_service(),
            )
            .await
        {
            ipc_task.abort();
            runtime.shutdown().await;
            return Err(error);
        }
    }
    let managed = runtime.core().managed_host.control().status();
    let reachable =
        args.lan || relayed || public_origin || (managed.enabled && managed.account_matches);
    // Linking needs the listener below to be serving, so this cannot run first.
    let remote_setup = (reachable || args.account).then(|| {
        let client = zuse_rpc::memory_client(runtime.core().rpc_service());
        let l10n = Localizer::new(l10n.locale());
        let account = args.account;
        tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
            if account
                && let Err(error) = async {
                    remote::sign_in(&client, &l10n).await?;
                    remote::link(&client, None, &l10n).await
                }
                .await
            {
                eprintln!("{error}");
            }
            if let Err(error) = remote::invite(&client, None, false, false, &l10n).await {
                eprintln!("{error}");
            }
        }))
    });
    let result = match tls {
        Some(tls) => browser.serve_tls(listener, assets, tls).await,
        None => browser.serve(listener, assets).await,
    };
    drop(remote_setup);
    shutdown.cancel();
    ipc_task.abort();
    runtime.shutdown().await;
    result
}

const IPC_RECORD: &str = "serve-ipc.json";

struct IpcPortRecord {
    path: PathBuf,
    value: Vec<u8>,
}

impl IpcPortRecord {
    fn publish(profile: &std::path::Path, port: u16) -> anyhow::Result<Self> {
        use std::io::Write;
        let path = profile.join(IPC_RECORD);
        let value = serde_json::to_vec(&json!({"pid": std::process::id(), "port": port}))?;
        let mut pending = tempfile::NamedTempFile::new_in(profile)?;
        pending.write_all(&value)?;
        pending.as_file().sync_all()?;
        pending.persist(&path)?;
        Ok(Self { path, value })
    }
}

impl Drop for IpcPortRecord {
    fn drop(&mut self) {
        if std::fs::read(&self.path).ok().as_deref() == Some(self.value.as_slice()) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn recorded_ipc_port(profile: &std::path::Path) -> Option<u16> {
    let record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(profile.join(IPC_RECORD)).ok()?).ok()?;
    let pid = record["pid"].as_u64()?.to_string();
    // A file left after a crash must never redirect a later profile owner.
    if InstanceLock::holder(profile)?.trim() != pid {
        return None;
    }
    u16::try_from(record["port"].as_u64()?)
        .ok()
        .filter(|port| *port != 0)
}

async fn bind_ipc(
    requested: Option<u16>,
    default: u16,
) -> std::io::Result<tokio::net::TcpListener> {
    let address = std::net::Ipv4Addr::LOCALHOST;
    match tokio::net::TcpListener::bind((address, requested.unwrap_or(default))).await {
        Err(error) if requested.is_none() && error.kind() == std::io::ErrorKind::AddrInUse => {
            tokio::net::TcpListener::bind((address, 0)).await
        }
        result => result,
    }
}

async fn reuse_engine(
    args: &ServeArgs,
    profile: &std::path::Path,
    port: u16,
    l10n: &Localizer,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !args.lan && !args.account && args.relay_origin.is_none() && args.token.is_none(),
        "This profile is already running. Use `zuse serve` to open its browser, configure remote access in Settings, or stop it before supplying remote access/token flags"
    );
    let client = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let client = zuse_rpc::connect_ws(&format!("ws://127.0.0.1:{port}")).await?;
        let info = client
            .call(zuse_rpc::methods::ENGINE_INFO, json!({}))
            .await?;
        let expected = std::fs::read_to_string(profile.join("device-id"))?;
        anyhow::ensure!(
            !expected.trim().is_empty() && Some(expected.trim()) == info["deviceId"].as_str(),
            "The engine on port {port} belongs to another profile"
        );
        Ok::<_, anyhow::Error>(client)
    })
    .await
    .context("Timed out identifying the running engine")??;
    let device = std::fs::read_to_string(profile.join("device-id"))?;
    let target = json!({"targetDeviceId": device.trim()});
    let status = client
        .call(zuse_rpc::methods::BROWSER_ACCESS_STATUS, target.clone())
        .await?;
    if status["running"] != true {
        let absolute = |path: &Option<PathBuf>| -> std::io::Result<Option<PathBuf>> {
            path.as_ref().map(std::path::absolute).transpose()
        };
        let assets = args
            .assets
            .clone()
            .or_else(zuse_engine::browser_access::discover_assets);
        client
            .call(
                zuse_rpc::methods::START_BROWSER_ACCESS,
                json!({
                    "targetDeviceId": device.trim(), "listen": args.listen,
                    "publicOrigin": args.public_origin, "tlsCert": absolute(&args.tls_cert)?,
                    "tlsKey": absolute(&args.tls_key)?, "assets": absolute(&assets)?,
                }),
            )
            .await?;
    } else {
        anyhow::ensure!(
            args.assets.is_none()
                && args.public_origin.is_none()
                && args.tls_cert.is_none()
                && (args.listen == "127.0.0.1:27656".parse::<std::net::SocketAddr>()?
                    || status["listen"].as_str() == Some(args.listen.to_string().as_str())),
            "Browser access is already running with different settings; open it with `zuse serve` or change its settings in the app"
        );
    }
    eprintln!(
        "Using the running Zuse engine on port {port}. Browser access stays available while that app or service is running."
    );
    remote::invite(&client, None, false, false, l10n).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_discovery_requires_the_current_profile_lock_owner() {
        let profile = tempfile::tempdir().unwrap();
        let _lock = InstanceLock::acquire(profile.path()).unwrap();
        let record = IpcPortRecord::publish(profile.path(), 31000).unwrap();
        assert_eq!(recorded_ipc_port(profile.path()), Some(31000));
        drop(record);
        assert!(recorded_ipc_port(profile.path()).is_none());
        std::fs::write(
            profile.path().join(IPC_RECORD),
            serde_json::to_vec(&json!({"pid": u64::MAX, "port": 31000})).unwrap(),
        )
        .unwrap();
        assert!(recorded_ipc_port(profile.path()).is_none());
    }

    #[tokio::test]
    async fn an_explicit_busy_rpc_port_is_not_silently_changed() {
        let owner = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let busy = owner.local_addr().unwrap().port();
        let error = bind_ipc(Some(busy), busy).await.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AddrInUse);
        assert_ne!(
            bind_ipc(Some(0), busy)
                .await
                .unwrap()
                .local_addr()
                .unwrap()
                .port(),
            0
        );
        let automatic = bind_ipc(None, busy).await.unwrap();
        assert_ne!(automatic.local_addr().unwrap().port(), busy);
    }
}
