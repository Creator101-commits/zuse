//! The standalone command updates its browser assets and binary as one package.
use std::path::{Path, PathBuf};

use anyhow::{Context as _, ensure};

use super::{FileMeta, Manifest, ReleaseSource};

/// Only recognize the explicit layout and marker written by install-cli.py.
pub fn install_root(exe: &Path) -> Option<PathBuf> {
    if !matches!(std::env::consts::OS, "linux" | "macos") {
        return None;
    }
    let exe = exe.canonicalize().ok()?;
    let bin = exe.parent()?;
    let package = bin.parent()?;
    let packages = package.parent()?;
    let root = packages.parent()?;
    (bin.file_name()? == "bin"
        && packages.file_name()? == "packages"
        && root.join(".installed-by-zuse-cli").is_file()
        && package.join("install.py").is_file())
    .then(|| root.to_owned())
}

pub fn artifact(version: &str) -> String {
    let (os, arch) = super::platform_key();
    format!("zuse-cli-{version}-{os}-{arch}.tar.gz")
}

/// The legacy SHA256SUMS stays compatible with old desktop updaters. Standalone
/// archives have their own checksums in SHA256SUMS.cli.
async fn manifest(edge_url: &str, release: &Manifest, file: &str) -> anyhow::Result<Manifest> {
    let mut release = release.clone();
    if !release.files.contains_key(file)
        && super::release_source(edge_url)? == ReleaseSource::GitHub
    {
        let url = format!(
            "{}/download/v{}/SHA256SUMS.cli",
            super::RELEASES_PAGE,
            release.version
        );
        let response = super::fetch_release_metadata(&super::http_client()?, &url).await?;
        ensure!(
            response.status.is_success(),
            "fetching CLI checksums: HTTP {}",
            response.status
        );
        let checksums = std::str::from_utf8(&response.body).context("reading CLI checksums")?;
        let mut matches = checksums.lines().filter_map(|line| {
            let (digest, name) = line.trim().split_once(char::is_whitespace)?;
            (name.trim_start().trim_start_matches('*') == file).then_some(digest)
        });
        let digest = matches
            .next()
            .context("release has no checksum for this CLI platform")?;
        ensure!(
            matches.next().is_none(),
            "release lists duplicate CLI checksums"
        );
        release.files.insert(
            file.to_owned(),
            FileMeta {
                sha256: Some(digest.to_owned()),
            },
        );
    }
    let digest = release
        .files
        .get(file)
        .and_then(|entry| entry.sha256.as_deref());
    ensure!(
        digest.is_some_and(
            |digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        ),
        "release has no valid checksum for {file}"
    );
    Ok(release)
}

/// Verify the archive and executable before the installer atomically activates
/// the complete package. Previous versions remain available to running servers.
pub async fn update(edge_url: &str, release: &Manifest, root: &Path) -> anyhow::Result<()> {
    super::require_managed_update_platform()?;
    ensure!(
        super::parse_version(&release.version).is_some(),
        "invalid release version"
    );
    let file = artifact(&release.version);
    let release = manifest(edge_url, release, &file).await?;
    // Match the bootstrap installer's interpreter selection, including Macs
    // whose default python3 is older than the installed versioned Python.
    let mut python = None;
    for candidate in ["python3", "python3.13", "python3.12", "python3.11"] {
        if tokio::process::Command::new(candidate)
            .args(["-c", "import sys; sys.exit(sys.version_info < (3, 11))"])
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
            .is_ok_and(|output| output.status.success())
        {
            python = Some(candidate);
            break;
        }
    }
    let python = python.context("Python 3.11 or newer is required for CLI updates")?;
    let stage = tempfile::tempdir_in(root)?;
    let tarball = stage.path().join(&file);
    super::download_release_file(edge_url, &release, &file, &tarball).await?;
    let unpacked = stage.path().join("unpacked");
    std::fs::create_dir(&unpacked)?;
    super::run(
        "tar",
        &[
            "-xzf",
            &tarball.to_string_lossy(),
            "-C",
            &unpacked.to_string_lossy(),
        ],
    )?;
    let package = unpacked.join(file.trim_end_matches(".tar.gz"));
    ensure!(
        package.join("share/zuse/web/index.html").is_file(),
        "CLI archive has no browser app"
    );
    super::verify_staged_binary(&package.join("bin/zuse"), &release.version).await?;
    let output = tokio::process::Command::new(python)
        .arg(package.join("install.py"))
        .arg("--root")
        .arg(root)
        .arg("--no-launchers")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .context("running the CLI package installer (Python 3.11+ required)")?;
    ensure!(
        output.status.success(),
        "CLI installation failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn recognition_requires_the_managed_layout_and_marker() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let package = root.join("packages/version");
        std::fs::create_dir_all(package.join("bin")).unwrap();
        std::fs::write(package.join("install.py"), "").unwrap();
        let exe = package.join("bin/zuse");
        std::fs::write(&exe, "").unwrap();
        assert_eq!(install_root(&exe), None);
        std::fs::write(root.join(".installed-by-zuse-cli"), "1").unwrap();
        let canonical = root.canonicalize().unwrap();
        assert_eq!(install_root(&exe), Some(canonical.clone()));
        std::os::unix::fs::symlink(&package, root.join("current")).unwrap();
        let launcher = root.join("zuse");
        std::os::unix::fs::symlink(root.join("current/bin/zuse"), &launcher).unwrap();
        assert_eq!(install_root(&launcher), Some(canonical.clone()));
        assert_eq!(
            install_root(&root.join("current/bin/zuse")),
            Some(canonical)
        );
        assert_eq!(install_root(&root.join("other/version/bin/zuse")), None);
    }

    #[tokio::test]
    async fn missing_checksums_fail_before_downloading_or_changing_the_install() {
        let temporary = tempfile::tempdir().unwrap();
        let release = Manifest {
            version: "1.2.3".into(),
            ..Default::default()
        };
        let error = update("http://127.0.0.1:1", &release, temporary.path())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("checksum"));
        assert_eq!(std::fs::read_dir(temporary.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn update_verifies_and_activates_the_binary_and_browser_together() {
        use sha2::{Digest, Sha256};
        use std::os::unix::fs::{PermissionsExt, symlink};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let temporary = tempfile::tempdir().unwrap();
        let file = artifact("1.2.3");
        let package = temporary.path().join(file.trim_end_matches(".tar.gz"));
        std::fs::create_dir_all(package.join("bin")).unwrap();
        std::fs::create_dir_all(package.join("share/zuse/web")).unwrap();
        std::fs::write(package.join("bin/zuse"), "#!/bin/sh\necho 'zuse 1.2.3'\n").unwrap();
        std::fs::set_permissions(
            package.join("bin/zuse"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        symlink("zuse", package.join("bin/zuse-server")).unwrap();
        std::fs::write(package.join("share/zuse/web/index.html"), "new browser").unwrap();
        std::fs::write(
            package.join("install.py"),
            include_str!("../../../scripts/install-cli.py"),
        )
        .unwrap();
        let files: std::collections::BTreeMap<_, _> =
            ["bin/zuse", "share/zuse/web/index.html", "install.py"]
                .into_iter()
                .map(|name| {
                    (
                        name,
                        format!(
                            "{:x}",
                            Sha256::digest(std::fs::read(package.join(name)).unwrap())
                        ),
                    )
                })
                .collect();
        let (os, arch) = super::super::platform_key();
        std::fs::write(
            package.join("manifest.json"),
            serde_json::to_vec(&serde_json::json!({
                "format": 1, "kind": "cli", "version": "1.2.3", "platform": os, "arch": arch,
                "files": files, "links": {"bin/zuse-server": "zuse"},
            }))
            .unwrap(),
        )
        .unwrap();
        let tarball = temporary.path().join("archive.tar.gz");
        super::super::run(
            "tar",
            &[
                "-czf",
                &tarball.to_string_lossy(),
                "-C",
                &temporary.path().to_string_lossy(),
                package.file_name().unwrap().to_str().unwrap(),
            ],
        )
        .unwrap();
        let body = std::fs::read(tarball).unwrap();
        let digest = format!("{:x}", Sha256::digest(&body));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                socket.read(&mut request).await.unwrap();
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                socket.write_all(&body).await.unwrap();
            }
        });
        let root = temporary.path().join("cli");
        std::fs::create_dir(&root).unwrap();
        let mut release = Manifest {
            version: "1.2.3".into(),
            files: [(
                file.clone(),
                FileMeta {
                    sha256: Some("0".repeat(64)),
                },
            )]
            .into(),
        };
        assert!(
            update(&base, &release, &root)
                .await
                .unwrap_err()
                .to_string()
                .contains("checksum mismatch")
        );
        assert!(!root.join("current").exists());
        release.files.get_mut(&file).unwrap().sha256 = Some(digest);
        update(&base, &release, &root).await.unwrap();
        server.await.unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("current/share/zuse/web/index.html")).unwrap(),
            "new browser"
        );
        let exe = std::fs::canonicalize(root.join("current/bin/zuse")).unwrap();
        assert_eq!(install_root(&exe), Some(root.canonicalize().unwrap()));
    }
}
