//! `pocket-bridge` — Mac bridge binary skeleton (AGT-04).
//!
//! Subcommands: `serve | status | pair | install-launch-agent | uninstall`.
//! `serve` and `pair` are stubs until AGT-06/07.

mod detect;

pub mod acp;

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const LAUNCH_AGENT_LABEL: &str = "app.getpocketai.bridge";
const USAGE: &str =
    "usage: pocket-bridge <serve | status | pair | install-launch-agent | uninstall [--purge]>";

#[derive(Serialize, Deserialize, Debug, Default, PartialEq)]
#[serde(default)]
struct Config {
    /// Port `serve` listens on (AGT-07). None until pairing configures it.
    port: Option<u16>,
}

/// Base dir: `$POCKET_BRIDGE_DIR` override (tests), else
/// `~/Library/Application Support/PocketAI Bridge`.
fn base_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("POCKET_BRIDGE_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    PathBuf::from(std::env::var("HOME").expect("HOME not set"))
        .join("Library/Application Support/PocketAI Bridge")
}

fn config_path(base: &Path) -> PathBuf {
    base.join("config.json")
}

fn logs_dir(base: &Path) -> PathBuf {
    base.join("logs")
}

/// Load `config.json`, creating base dir, `logs/`, and a default config on first run.
fn load_or_create_config(base: &Path) -> Result<Config, String> {
    std::fs::create_dir_all(logs_dir(base)).map_err(|e| format!("create {base:?}: {e}"))?;
    let path = config_path(base);
    if path.exists() {
        let bytes = std::fs::read(&path).map_err(|e| format!("read {path:?}: {e}"))?;
        serde_json::from_slice(&bytes).map_err(|e| format!("parse {path:?}: {e}"))
    } else {
        let cfg = Config::default();
        let json = serde_json::to_string_pretty(&cfg).expect("serialize default config");
        std::fs::write(&path, json).map_err(|e| format!("write {path:?}: {e}"))?;
        Ok(cfg)
    }
}

/// Append one line to `logs/bridge.log`; best-effort.
fn log_line(base: &Path, line: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(logs_dir(base).join("bridge.log"))
    {
        let _ = writeln!(f, "{line}");
    }
}

fn status_json(
    base: &Path,
    cfg: &Config,
    agents: &[detect::AgentInfo],
    path_source: &str,
) -> serde_json::Value {
    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "config_path": config_path(base),
        "port": cfg.port,
        "agents": agents,
        "path_source": path_source,
        "acp_adapters": detect::ACP_ADAPTERS.iter().copied().collect::<std::collections::HashMap<_, _>>(),
    })
}

fn plist_contents(exe: &Path, base: &Path) -> String {
    let log = logs_dir(base).join("bridge.log");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LAUNCH_AGENT_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
        <string>serve</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
</dict>
</plist>
"#,
        exe = exe.display(),
        log = log.display(),
    )
}

fn plist_path(launch_agents_dir: &Path) -> PathBuf {
    launch_agents_dir.join(format!("{LAUNCH_AGENT_LABEL}.plist"))
}

fn launchctl(args: &[&str]) {
    // Best-effort: `unload` fails when not loaded, `load` when already loaded —
    // both fine for idempotency.
    let _ = Command::new("launchctl").args(args).status();
}

/// Write the plist (overwrite = idempotent) and load it unless `dry_run`.
fn install_launch_agent(
    launch_agents_dir: &Path,
    base: &Path,
    exe: &Path,
    dry_run: bool,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(launch_agents_dir)
        .map_err(|e| format!("create {launch_agents_dir:?}: {e}"))?;
    let path = plist_path(launch_agents_dir);
    std::fs::write(&path, plist_contents(exe, base)).map_err(|e| format!("write {path:?}: {e}"))?;
    if !dry_run {
        launchctl(&["unload", "-w", &path.to_string_lossy()]);
        launchctl(&["load", "-w", &path.to_string_lossy()]);
    }
    Ok(path)
}

/// Unload + remove the plist; `purge` also removes the config/logs dir. Idempotent.
fn uninstall(launch_agents_dir: &Path, base: &Path, purge: bool, dry_run: bool) -> Result<(), String> {
    let path = plist_path(launch_agents_dir);
    if path.exists() {
        if !dry_run {
            launchctl(&["unload", "-w", &path.to_string_lossy()]);
        }
        std::fs::remove_file(&path).map_err(|e| format!("remove {path:?}: {e}"))?;
    }
    if purge && base.exists() {
        std::fs::remove_dir_all(base).map_err(|e| format!("remove {base:?}: {e}"))?;
    }
    Ok(())
}

fn dry_run() -> bool {
    std::env::var("POCKET_BRIDGE_DRY_RUN").is_ok_and(|v| !v.is_empty() && v != "0")
}

fn not_implemented(base: &Path, command: &str) -> ExitCode {
    match load_or_create_config(base) {
        Ok(_) => {
            log_line(base, &format!("{command}: invoked, not implemented (AGT-06/07)"));
            println!(
                "{}",
                serde_json::json!({
                    "error": "not implemented",
                    "command": command,
                    "hint": "implemented in a later release (AGT-06/07)",
                })
            );
            ExitCode::from(1)
        }
        Err(e) => fail(&e),
    }
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("pocket-bridge: {msg}");
    ExitCode::from(1)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let base = base_dir();
    let launch_agents_dir =
        PathBuf::from(std::env::var("HOME").expect("HOME not set")).join("Library/LaunchAgents");

    match args.first().map(String::as_str) {
        Some("status") => match load_or_create_config(&base) {
            Ok(cfg) => {
                let (agents, path_source) = detect::detect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&status_json(&base, &cfg, &agents, path_source))
                        .expect("status json")
                );
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e),
        },
        Some(cmd @ ("serve" | "pair")) => not_implemented(&base, cmd),
        Some("install-launch-agent") => {
            let exe = match std::env::current_exe() {
                Ok(p) => p,
                Err(e) => return fail(&format!("current_exe: {e}")),
            };
            match load_or_create_config(&base)
                .and_then(|_| install_launch_agent(&launch_agents_dir, &base, &exe, dry_run()))
            {
                Ok(path) => {
                    log_line(&base, &format!("install-launch-agent: {}", path.display()));
                    println!("{}", serde_json::json!({ "installed": path }));
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&e),
            }
        }
        Some("uninstall") => {
            let purge = args.iter().any(|a| a == "--purge");
            match uninstall(&launch_agents_dir, &base, purge, dry_run()) {
                Ok(()) => {
                    println!("{}", serde_json::json!({ "uninstalled": true, "purged": purge }));
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&e),
            }
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_created_on_first_run() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = load_or_create_config(dir.path()).unwrap();
        assert_eq!(cfg, Config::default());
        assert!(config_path(dir.path()).exists());
        assert!(logs_dir(dir.path()).is_dir());
    }

    #[test]
    fn config_loads_existing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path()).unwrap();
        std::fs::write(config_path(dir.path()), r#"{ "port": 7490 }"#).unwrap();
        let cfg = load_or_create_config(dir.path()).unwrap();
        assert_eq!(cfg.port, Some(7490));
    }

    #[test]
    fn config_rejects_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(config_path(dir.path()), "not json").unwrap();
        assert!(load_or_create_config(dir.path()).is_err());
    }

    #[test]
    fn status_json_shape() {
        let dir = tempfile::tempdir().unwrap();
        let agents = vec![detect::AgentInfo {
            name: "claude".into(),
            available: true,
            path: Some("/opt/bin/claude".into()),
            version: Some("2.1.0".into()),
            hint: None,
        }];
        let v = status_json(dir.path(), &Config { port: Some(7490) }, &agents, "login-shell");
        assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(v["config_path"], config_path(dir.path()).to_str().unwrap());
        assert_eq!(v["port"], 7490);
        assert_eq!(v["path_source"], "login-shell");
        assert_eq!(v["agents"][0]["name"], "claude");
        assert_eq!(v["agents"][0]["available"], true);
        assert_eq!(v["agents"][0]["version"], "2.1.0");
        assert!(v["agents"][0]["hint"].is_null());
        assert!(v["acp_adapters"]["codex"].as_str().unwrap().starts_with("@agentclientprotocol/codex-acp@"));
        // port omitted in config -> null in status
        assert!(status_json(dir.path(), &Config::default(), &[], "fallback")["port"].is_null());
    }

    #[test]
    fn plist_has_required_keys() {
        let dir = tempfile::tempdir().unwrap();
        let p = plist_contents(Path::new("/usr/local/bin/pocket-bridge"), dir.path());
        assert!(p.contains("<string>app.getpocketai.bridge</string>"));
        assert!(p.contains("<string>/usr/local/bin/pocket-bridge</string>"));
        assert!(p.contains("<string>serve</string>"));
        assert!(p.contains("<key>RunAtLoad</key>\n    <true/>"));
        assert!(p.contains("<key>SuccessfulExit</key>\n        <false/>"));
    }

    #[test]
    fn install_uninstall_round_trip_dry_run() {
        let la = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();
        let exe = Path::new("/tmp/pocket-bridge");

        let path = install_launch_agent(la.path(), base.path(), exe, true).unwrap();
        assert!(path.exists());
        assert_eq!(path, la.path().join("app.getpocketai.bridge.plist"));
        // idempotent install
        install_launch_agent(la.path(), base.path(), exe, true).unwrap();
        assert!(path.exists());

        uninstall(la.path(), base.path(), false, true).unwrap();
        assert!(!path.exists());
        // idempotent uninstall
        uninstall(la.path(), base.path(), false, true).unwrap();

        // purge removes the config dir
        load_or_create_config(base.path()).unwrap();
        assert!(base.path().exists());
        uninstall(la.path(), base.path(), true, true).unwrap();
        assert!(!base.path().exists());
    }
}
