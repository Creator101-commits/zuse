//! "Open via SSH" in an app: the editors and terminals on this computer that
//! can open a cloud computer's repository over SSH, and how each launches.
//!
//! Every app connects to the managed `zuse-<workspace>` host (see
//! `cloud_ssh`), whose entry carries the user, key, pinned host key and proxy,
//! so no port or identity is passed here. Editors take a remote URL (macOS)
//! or their CLI's remote flag (Linux, Windows); terminals run a launcher
//! script that execs `ssh` into the repository.
use std::path::{Path, PathBuf};

use super::open_in::{TargetDefinition, cached_icon_png, executable_on_path, home_path};
use crate::cloud_ssh::{Prepared, quote};

/// An app that can open a cloud computer over SSH.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SshApp {
    Cursor,
    Zed,
    VsCode,
    VsCodeInsiders,
    Windsurf,
    Terminal,
    Ghostty,
    Iterm,
    Warp,
    WezTerm,
    Kitty,
    Alacritty,
}

/// Where the app runs; decides detection and launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Platform {
    Mac,
    Linux,
    Windows,
}

impl Platform {
    fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

impl SshApp {
    /// Menu order: editors, then terminals.
    const ALL: [SshApp; 12] = [
        Self::Cursor,
        Self::Zed,
        Self::VsCode,
        Self::VsCodeInsiders,
        Self::Windsurf,
        Self::Terminal,
        Self::Ghostty,
        Self::Iterm,
        Self::Warp,
        Self::WezTerm,
        Self::Kitty,
        Self::Alacritty,
    ];

    pub(super) fn id(self) -> &'static str {
        match self {
            Self::Cursor => "cursor",
            Self::Zed => "zed",
            Self::VsCode => "vscode",
            Self::VsCodeInsiders => "vscode-insiders",
            Self::Windsurf => "windsurf",
            Self::Terminal => "terminal",
            Self::Ghostty => "ghostty",
            Self::Iterm => "iterm",
            Self::Warp => "warp",
            Self::WezTerm => "wezterm",
            Self::Kitty => "kitty",
            Self::Alacritty => "alacritty",
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Cursor => "Cursor",
            Self::Zed => "Zed",
            Self::VsCode => "VS Code",
            Self::VsCodeInsiders => "VS Code Insiders",
            Self::Windsurf => "Windsurf",
            Self::Terminal => "Terminal",
            Self::Ghostty => "Ghostty",
            Self::Iterm => "iTerm2",
            Self::Warp => "Warp",
            Self::WezTerm => "WezTerm",
            Self::Kitty => "Kitty",
            Self::Alacritty => "Alacritty",
        }
    }

    pub(super) fn is_terminal(self) -> bool {
        !matches!(
            self,
            Self::Cursor | Self::Zed | Self::VsCode | Self::VsCodeInsiders | Self::Windsurf
        )
    }

    /// The URL scheme of a VS Code-family editor.
    fn vscode_scheme(self) -> Option<&'static str> {
        match self {
            Self::Cursor => Some("cursor"),
            Self::VsCode => Some("vscode"),
            Self::VsCodeInsiders => Some("vscode-insiders"),
            Self::Windsurf => Some("windsurf"),
            _ => None,
        }
    }

    /// macOS app bundles, `~` meaning the home directory.
    fn mac_bundles(self) -> &'static [&'static str] {
        match self {
            Self::Cursor => &["/Applications/Cursor.app", "~/Applications/Cursor.app"],
            Self::Zed => &["/Applications/Zed.app", "~/Applications/Zed.app"],
            Self::VsCode => &[
                "/Applications/Visual Studio Code.app",
                "~/Applications/Visual Studio Code.app",
            ],
            Self::VsCodeInsiders => &[
                "/Applications/Visual Studio Code - Insiders.app",
                "~/Applications/Visual Studio Code - Insiders.app",
            ],
            Self::Windsurf => &["/Applications/Windsurf.app", "~/Applications/Windsurf.app"],
            Self::Terminal => &[
                "/System/Applications/Utilities/Terminal.app",
                "/Applications/Utilities/Terminal.app",
            ],
            Self::Ghostty => &["/Applications/Ghostty.app", "~/Applications/Ghostty.app"],
            Self::Iterm => &["/Applications/iTerm.app", "~/Applications/iTerm.app"],
            Self::Warp => &["/Applications/Warp.app", "~/Applications/Warp.app"],
            Self::WezTerm => &["/Applications/WezTerm.app", "~/Applications/WezTerm.app"],
            Self::Kitty => &["/Applications/kitty.app", "~/Applications/kitty.app"],
            Self::Alacritty => &[
                "/Applications/Alacritty.app",
                "~/Applications/Alacritty.app",
            ],
        }
    }

    /// Icon file names to try in the bundle's resources.
    fn icon_names(self) -> &'static [&'static str] {
        match self {
            Self::VsCode | Self::VsCodeInsiders => &["Code", "Code - Insiders"],
            Self::Terminal => &["Terminal"],
            Self::Iterm => &["AppIcon", "iTerm2"],
            Self::Kitty => &["kitty"],
            _ => &[],
        }
    }

    /// The commands to look for on PATH (Linux, Windows); empty when the app
    /// isn't offered there.
    fn commands(self, platform: Platform) -> &'static [&'static str] {
        match (self, platform) {
            (_, Platform::Mac) => &[],
            (Self::Cursor, _) => &["cursor"],
            (Self::VsCode, _) => &["code"],
            (Self::VsCodeInsiders, _) => &["code-insiders"],
            (Self::Windsurf, _) => &["windsurf"],
            (Self::Terminal, Platform::Windows) => &["wt"],
            (Self::Zed, Platform::Linux) => &["zed", "zeditor"],
            (Self::Terminal, Platform::Linux) => &["x-terminal-emulator"],
            (Self::Ghostty, Platform::Linux) => &["ghostty"],
            (Self::WezTerm, Platform::Linux) => &["wezterm"],
            (Self::Kitty, Platform::Linux) => &["kitty"],
            (Self::Alacritty, Platform::Linux) => &["alacritty"],
            _ => &[],
        }
    }

    fn definition(self) -> TargetDefinition {
        TargetDefinition {
            id: self.id(),
            label: self.label(),
            app_name: Some(self.label()),
            app_paths: self.mac_bundles(),
            icon_names: self.icon_names(),
            icon_paths: &[],
        }
    }
}

/// One installed app the SSH menu lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SshTarget {
    pub(super) app: SshApp,
    /// macOS: the app bundle. Elsewhere: the executable on PATH.
    launcher: Option<PathBuf>,
    /// A PNG of the app's own icon (macOS only).
    pub(super) icon: Option<PathBuf>,
}

#[cfg(test)]
impl SshTarget {
    pub(super) fn for_test(app: SshApp) -> Self {
        Self {
            app,
            launcher: None,
            icon: None,
        }
    }
}

/// The macOS apps present, by `exists`. Terminal ships with the system, so
/// it is always offered.
fn mac_select(exists: &dyn Fn(&Path) -> bool) -> Vec<(SshApp, Option<PathBuf>)> {
    SshApp::ALL
        .iter()
        .filter_map(|&app| {
            let bundle = app
                .mac_bundles()
                .iter()
                .filter_map(|path| home_path(path))
                .find(|path| exists(path));
            (bundle.is_some() || app == SshApp::Terminal).then_some((app, bundle))
        })
        .collect()
}

/// The Linux or Windows apps whose command resolves on `path_var`.
fn path_select(platform: Platform, path_var: Option<&std::ffi::OsStr>) -> Vec<SshTarget> {
    SshApp::ALL
        .iter()
        .filter_map(|&app| {
            let launcher = app
                .commands(platform)
                .iter()
                .find_map(|command| executable_on_path(command, path_var))?;
            Some(SshTarget {
                app,
                launcher: Some(launcher),
                icon: None,
            })
        })
        .collect()
}

/// The apps on this computer that can open a cloud computer over SSH, in
/// menu order. Blocking: run it off the UI thread.
pub(super) fn detect(cache_dir: &Path) -> Vec<SshTarget> {
    match Platform::current() {
        Platform::Mac => mac_select(&|path| path.exists())
            .into_iter()
            .map(|(app, bundle)| SshTarget {
                icon: cached_icon_png(&app.definition(), bundle.as_deref(), cache_dir),
                app,
                launcher: bundle,
            })
            .collect(),
        platform => path_select(platform, std::env::var_os("PATH").as_deref()),
    }
}

/// Percent-encode a remote path for a URL, keeping its slashes.
fn encode_path(path: &str) -> String {
    let path = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn apple_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn yaml_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// How one app is launched.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Launch {
    /// Hand a URL to the system's URL handler.
    Url(String),
    /// Run a program.
    Program { program: PathBuf, args: Vec<String> },
    /// The system terminal, running a shell line (`cloud_ssh::open_terminal`).
    SystemTerminal(String),
    /// macOS: run an AppleScript.
    AppleScript(String),
    /// Warp: write a launch configuration to `path`, then open `url`.
    Warp {
        path: PathBuf,
        config: String,
        url: String,
    },
}

/// How `target` opens `prepared` on `platform`. Terminals run `script` (the
/// launcher from `cloud_ssh::write_launcher`). `None` when the app can't
/// open an SSH session there.
pub(super) fn plan(
    target: &SshTarget,
    prepared: &Prepared,
    script: &Path,
    platform: Platform,
) -> Option<Launch> {
    let host = &prepared.host;
    let path = encode_path(&prepared.path);
    let script = script.to_string_lossy().into_owned();
    let mac = platform == Platform::Mac;
    // `open -na <bundle> --args …`: a new instance of the app, with arguments.
    let mac_open = |args: &[&str]| {
        let bundle = target.launcher.as_ref()?;
        let mut all = vec![
            "-na".to_owned(),
            bundle.to_string_lossy().into_owned(),
            "--args".to_owned(),
        ];
        all.extend(args.iter().map(|arg| (*arg).to_owned()));
        Some(Launch::Program {
            program: "open".into(),
            args: all,
        })
    };
    let program = |args: &[&str]| {
        Some(Launch::Program {
            program: target.launcher.clone()?,
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        })
    };
    let app = target.app;
    if let Some(scheme) = app.vscode_scheme() {
        return if mac {
            Some(Launch::Url(format!(
                "{scheme}://vscode-remote/ssh-remote+{host}{path}"
            )))
        } else {
            program(&["--remote", &format!("ssh-remote+{host}"), &prepared.path])
        };
    }
    match (app, platform) {
        (SshApp::Zed, Platform::Mac) => Some(Launch::Url(format!(
            "zed://ssh/{}@{host}{path}",
            prepared.user
        ))),
        (SshApp::Zed, Platform::Linux) => {
            program(&[&format!("ssh://{}@{host}{path}", prepared.user)])
        }
        (SshApp::Terminal, Platform::Windows) => program(&["ssh", host]),
        (SshApp::Terminal, _) => Some(Launch::SystemTerminal(crate::cloud_ssh::terminal_command(
            prepared,
        ))),
        (SshApp::Iterm, Platform::Mac) => Some(Launch::AppleScript(format!(
            "tell application \"iTerm\"\n\
             activate\n\
             set zuseWindow to (create window with default profile)\n\
             tell current session of zuseWindow to write text \"{}\"\n\
             end tell",
            apple_string(&crate::cloud_ssh::terminal_command(prepared))
        ))),
        (SshApp::Warp, Platform::Mac) => {
            let config_path = Path::new(&script).with_extension("yaml");
            let config = format!(
                "---\nname: {name}\nwindows:\n  - tabs:\n      - title: {name}\n        layout:\n          commands:\n            - exec: {exec}\n",
                name = yaml_string(host),
                exec = yaml_string(&quote(&script)),
            );
            let url = format!(
                "warp://launch/{}",
                encode_path(&config_path.to_string_lossy())
                    .trim_start_matches('/')
                    .to_owned()
            );
            Some(Launch::Warp {
                path: config_path,
                config,
                url,
            })
        }
        (SshApp::Ghostty, Platform::Mac) => mac_open(&["-e", &script]),
        (SshApp::Ghostty, Platform::Linux) => program(&["-e", &script]),
        (SshApp::WezTerm, Platform::Mac) => mac_open(&["start", "--", &script]),
        (SshApp::WezTerm, Platform::Linux) => program(&["start", "--", &script]),
        (SshApp::Kitty, Platform::Mac) => mac_open(&[&script]),
        (SshApp::Kitty, Platform::Linux) => program(&[&script]),
        (SshApp::Alacritty, Platform::Mac) => mac_open(&["-e", &script]),
        (SshApp::Alacritty, Platform::Linux) => program(&["-e", &script]),
        _ => None,
    }
}

/// Run `command` to completion; for launchers (`open`, `osascript`,
/// `xdg-open`) that exit once the app has the request.
fn run_to_exit(mut command: std::process::Command) -> Result<(), String> {
    let output = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(if detail.is_empty() {
        format!("exited with {}", output.status)
    } else {
        detail
    })
}

fn open_url(url: &str) -> Result<(), String> {
    let mut command = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else {
        std::process::Command::new("xdg-open")
    };
    command.arg(url);
    run_to_exit(command)
}

/// Open the computer's repository in `target`. Blocking.
pub(super) fn open(target: &SshTarget, prepared: &Prepared) -> Result<(), String> {
    let platform = Platform::current();
    let script = if target.app.is_terminal() {
        crate::cloud_ssh::write_launcher(prepared)?
    } else {
        PathBuf::new()
    };
    let launch = plan(target, prepared, &script, platform)
        .ok_or_else(|| "it can't open SSH sessions on this system".to_owned())?;
    match launch {
        Launch::Url(url) => open_url(&url),
        Launch::SystemTerminal(command) => crate::cloud_ssh::open_terminal(&command),
        Launch::AppleScript(script) => {
            let mut command = std::process::Command::new("osascript");
            command.args(["-e", &script]);
            run_to_exit(command)
        }
        Launch::Warp { path, config, url } => {
            std::fs::write(&path, config).map_err(|error| error.to_string())?;
            open_url(&url)
        }
        Launch::Program { program, args } => {
            let waits = program == Path::new("open");
            let mut command = std::process::Command::new(&program);
            command.args(&args);
            if waits {
                return run_to_exit(command);
            }
            // Editors and terminals launched directly keep running; a
            // detached thread reaps them.
            let mut child = command
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|error| error.to_string())?;
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepared(path: &str) -> Prepared {
        Prepared {
            host: "zuse-w1".into(),
            user: "zuse".into(),
            path: path.into(),
        }
    }

    fn target(app: SshApp, launcher: Option<&str>) -> SshTarget {
        SshTarget {
            app,
            launcher: launcher.map(PathBuf::from),
            icon: None,
        }
    }

    fn url(app: SshApp, path: &str) -> String {
        match plan(
            &target(app, Some("/Applications/X.app")),
            &prepared(path),
            Path::new("/s"),
            Platform::Mac,
        ) {
            Some(Launch::Url(url)) => url,
            other => panic!("{app:?}: {other:?}"),
        }
    }

    fn program(
        app: SshApp,
        launcher: &str,
        script: &str,
        platform: Platform,
    ) -> (String, Vec<String>) {
        match plan(
            &target(app, Some(launcher)),
            &prepared("/home/zuse/my repo"),
            Path::new(script),
            platform,
        ) {
            Some(Launch::Program { program, args }) => {
                (program.to_string_lossy().into_owned(), args)
            }
            other => panic!("{app:?}: {other:?}"),
        }
    }

    #[test]
    fn editors_open_the_repository_by_remote_url_on_macos() {
        let path = "/home/zuse/my repo";
        assert_eq!(
            url(SshApp::VsCode, path),
            "vscode://vscode-remote/ssh-remote+zuse-w1/home/zuse/my%20repo"
        );
        assert_eq!(
            url(SshApp::VsCodeInsiders, path),
            "vscode-insiders://vscode-remote/ssh-remote+zuse-w1/home/zuse/my%20repo"
        );
        assert_eq!(
            url(SshApp::Cursor, "/home/zuse/workspace"),
            "cursor://vscode-remote/ssh-remote+zuse-w1/home/zuse/workspace"
        );
        assert_eq!(
            url(SshApp::Windsurf, "/w/a#b?c'd"),
            "windsurf://vscode-remote/ssh-remote+zuse-w1/w/a%23b%3Fc%27d"
        );
        assert_eq!(
            url(SshApp::Zed, path),
            "zed://ssh/zuse@zuse-w1/home/zuse/my%20repo"
        );
        // A relative path still reads as absolute on the computer.
        assert_eq!(url(SshApp::Zed, "w"), "zed://ssh/zuse@zuse-w1/w");
    }

    #[test]
    fn editors_use_their_cli_remote_flag_elsewhere() {
        for platform in [Platform::Linux, Platform::Windows] {
            let (program_path, args) = program(SshApp::VsCode, "/usr/bin/code", "/s", platform);
            assert_eq!(program_path, "/usr/bin/code");
            // A separate argument: spaces need no quoting.
            assert_eq!(
                args,
                ["--remote", "ssh-remote+zuse-w1", "/home/zuse/my repo"]
            );
        }
        let (_, args) = program(SshApp::Zed, "/usr/bin/zed", "/s", Platform::Linux);
        assert_eq!(args, ["ssh://zuse@zuse-w1/home/zuse/my%20repo"]);
        // Zed has no Windows CLI here.
        assert!(
            plan(
                &target(SshApp::Zed, Some("zed")),
                &prepared("/w"),
                Path::new("/s"),
                Platform::Windows
            )
            .is_none()
        );
        // Without a resolved launcher there is nothing to run.
        assert!(
            plan(
                &target(SshApp::Cursor, None),
                &prepared("/w"),
                Path::new("/s"),
                Platform::Linux
            )
            .is_none()
        );
    }

    #[test]
    fn terminals_run_the_launcher_script() {
        let script = "/Users/a b/.zuse/ssh/open/zuse-w1.command";
        let bundle = "/Applications/Ghostty.app";
        let (program_path, args) = program(SshApp::Ghostty, bundle, script, Platform::Mac);
        assert_eq!(program_path, "open");
        assert_eq!(args, ["-na", bundle, "--args", "-e", script]);
        let (_, args) = program(SshApp::WezTerm, "/A/WezTerm.app", script, Platform::Mac);
        assert_eq!(
            args,
            ["-na", "/A/WezTerm.app", "--args", "start", "--", script]
        );
        let (_, args) = program(SshApp::Kitty, "/A/kitty.app", script, Platform::Mac);
        assert_eq!(args, ["-na", "/A/kitty.app", "--args", script]);
        let (_, args) = program(SshApp::Alacritty, "/A/Alacritty.app", script, Platform::Mac);
        assert_eq!(args, ["-na", "/A/Alacritty.app", "--args", "-e", script]);

        let (program_path, args) =
            program(SshApp::Kitty, "/usr/bin/kitty", script, Platform::Linux);
        assert_eq!(program_path, "/usr/bin/kitty");
        assert_eq!(args, [script]);
        let (_, args) = program(SshApp::Ghostty, "/usr/bin/ghostty", script, Platform::Linux);
        assert_eq!(args, ["-e", script]);
        let (_, args) = program(SshApp::WezTerm, "/usr/bin/wezterm", script, Platform::Linux);
        assert_eq!(args, ["start", "--", script]);
        let (_, args) = program(SshApp::Terminal, "C:/wt.exe", script, Platform::Windows);
        assert_eq!(args, ["ssh", "zuse-w1"]);
    }

    #[test]
    fn system_terminal_iterm_and_warp_get_the_quoted_ssh_line() {
        let prepared = prepared("/home/zuse/my \"repo\"");
        let line = crate::cloud_ssh::terminal_command(&prepared);
        let terminal = target(SshApp::Terminal, None);
        for platform in [Platform::Mac, Platform::Linux] {
            assert_eq!(
                plan(&terminal, &prepared, Path::new("/s"), platform),
                Some(Launch::SystemTerminal(line.clone()))
            );
        }
        let Some(Launch::AppleScript(script)) = plan(
            &target(SshApp::Iterm, Some("/Applications/iTerm.app")),
            &prepared,
            Path::new("/s"),
            Platform::Mac,
        ) else {
            panic!("iTerm2 runs an AppleScript");
        };
        assert!(script.starts_with("tell application \"iTerm\"\n"));
        assert!(script.contains(&format!("write text \"{}\"", apple_string(&line))));
        // The repository's quotes survive both the shell and AppleScript.
        assert!(script.contains(r#"my \"repo\""#));

        let Some(Launch::Warp { path, config, url }) = plan(
            &target(SshApp::Warp, Some("/Applications/Warp.app")),
            &prepared,
            Path::new("/Users/a b/.zuse/ssh/open/zuse-w1.command"),
            Platform::Mac,
        ) else {
            panic!("Warp opens a launch configuration");
        };
        assert_eq!(path, Path::new("/Users/a b/.zuse/ssh/open/zuse-w1.yaml"));
        assert_eq!(url, "warp://launch/Users/a%20b/.zuse/ssh/open/zuse-w1.yaml");
        assert!(config.contains("- exec: '''/Users/a b/.zuse/ssh/open/zuse-w1.command'''\n"));
        // macOS-only apps aren't offered elsewhere.
        for app in [SshApp::Iterm, SshApp::Warp] {
            assert!(
                plan(
                    &target(app, Some("x")),
                    &prepared,
                    Path::new("/s"),
                    Platform::Linux
                )
                .is_none()
            );
        }
    }

    #[test]
    fn macos_lists_installed_apps_in_menu_order_and_always_terminal() {
        let installed = [
            "/Applications/Zed.app",
            "/Applications/iTerm.app",
            "/Applications/Cursor.app",
        ];
        let apps: Vec<_> = mac_select(&|path| installed.iter().any(|i| Path::new(i) == path))
            .into_iter()
            .map(|(app, _)| app)
            .collect();
        assert_eq!(
            apps,
            [SshApp::Cursor, SshApp::Zed, SshApp::Terminal, SshApp::Iterm]
        );
        let apps: Vec<_> = mac_select(&|_| false)
            .into_iter()
            .map(|(app, bundle)| (app, bundle.is_none()))
            .collect();
        assert_eq!(apps, [(SshApp::Terminal, true)]);
    }

    #[test]
    fn path_detection_offers_only_what_resolves() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["code", "zeditor", "kitty", "wt"] {
            let bin = dir.path().join(name);
            std::fs::write(&bin, "#!/bin/sh\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        let path = std::env::join_paths([dir.path()]).unwrap();
        let apps = |platform| -> Vec<SshApp> {
            path_select(platform, Some(path.as_os_str()))
                .into_iter()
                .map(|target| target.app)
                .collect()
        };
        assert_eq!(
            apps(Platform::Linux),
            [SshApp::Zed, SshApp::VsCode, SshApp::Kitty]
        );
        if cfg!(unix) {
            assert_eq!(apps(Platform::Windows), [SshApp::VsCode, SshApp::Terminal]);
        }
        assert!(path_select(Platform::Linux, None).is_empty());
    }
}
