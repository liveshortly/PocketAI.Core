//! Agent detection (AGT-05): which coding agents are installed, resolved via the
//! user's *login shell* PATH (nvm/homebrew paths live in `.zprofile`, not in the
//! LaunchAgent environment).
//!
//! ACP adapters are pinned in [`ACP_ADAPTERS`] and launched with `npx -y` at session
//! start — that is AGT-06's job. Detection only records whether `npx` exists.

use serde::Serialize;
use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// (agent, pinned npm adapter) for agents that don't speak ACP natively. Verified on npm
/// 2026-11-02: the `@zed-industries/*` names are deprecated, renamed to these.
pub const ACP_ADAPTERS: &[(&str, &str)] = &[
    ("claude", "@agentclientprotocol/claude-agent-acp@0.100.0"),
    ("codex", "@agentclientprotocol/codex-acp@2.5.0"),
];

/// (name, install hint shown when missing).
const AGENTS: &[(&str, &str)] = &[
    ("claude", "npm install -g @anthropic-ai/claude-code"),
    ("codex", "npm install -g @openai/codex"),
    ("opencode", "npm install -g opencode-ai"),
    ("goose", "brew install block/goose/goose"),
    ("node", "brew install node"),
    // npx ships with Node; needed to launch the pinned ACP adapters (AGT-06).
    ("npx", "brew install node (npx is included with Node)"),
];

#[derive(Serialize, Debug, PartialEq)]
pub struct AgentInfo {
    pub name: String,
    pub available: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub hint: Option<String>,
}

/// Detect all known agents using the real shell. Convenience wrapper for `main`.
/// Also returns where PATH came from: `"login-shell"` or `"fallback"` (the bridge
/// process's own PATH, used when the shell fails or times out).
pub fn detect() -> (Vec<AgentInfo>, &'static str) {
    let (path, source) = match shell_path("/bin/zsh", Duration::from_secs(5)) {
        Some(p) => (p, "login-shell"),
        None => (std::env::var("PATH").unwrap_or_default(), "fallback"),
    };
    (detect_agents(|names| resolve_in(&path, names), shell_version), source)
}

/// Core detection, with the shell-outs injected so tests use fakes:
/// `resolve` maps names → paths (login-shell `command -v` in production),
/// `version_of` runs `<path> --version` (None when it fails or times out).
pub fn detect_agents(
    resolve: impl Fn(&[&str]) -> HashMap<String, String>,
    version_of: impl Fn(&str) -> Option<String>,
) -> Vec<AgentInfo> {
    let names: Vec<&str> = AGENTS.iter().map(|(n, _)| *n).collect();
    let found = resolve(&names);
    AGENTS
        .iter()
        .map(|(name, hint)| match found.get(*name) {
            Some(path) => AgentInfo {
                name: (*name).to_string(),
                available: true,
                version: version_of(path),
                path: Some(path.clone()),
                hint: None,
            },
            None => AgentInfo {
                name: (*name).to_string(),
                available: false,
                path: None,
                version: None,
                hint: Some((*hint).to_string()),
            },
        })
        .collect()
}

const PATH_START: &str = "__POCKET_PATH_START__";
const PATH_END: &str = "__POCKET_PATH_END__";

/// PATH as an interactive login zsh sees it: `-i` sources `.zshrc` too (nvm lives
/// there), `-l` sources `.zprofile` (homebrew). Printed between markers so rc-file noise
/// (banners, echo, instant prompts) can't corrupt it; `timeout` + kill so a hanging rc
/// file can't wedge `status`. None on failure/timeout → caller falls back.
/// ponytail: zsh only (macOS default); honour $SHELL if bash/fish users show up.
fn shell_path(shell: &str, timeout: Duration) -> Option<String> {
    let script = format!(r#"printf '%s%s%s' {PATH_START} "$PATH" {PATH_END}"#);
    let out = run_with_timeout(Command::new(shell).args(["-ilc", &script]), timeout)?;
    let rest = &out[out.find(PATH_START)? + PATH_START.len()..];
    let path = &rest[..rest.find(PATH_END)?];
    (!path.is_empty()).then(|| path.to_string())
}

/// `command -v` equivalent over an explicit PATH: first executable file per name.
/// Done in Rust rather than in the shell so aliases/functions can't masquerade as paths.
fn resolve_in(path: &str, names: &[&str]) -> HashMap<String, String> {
    use std::os::unix::fs::PermissionsExt;
    names
        .iter()
        .filter_map(|name| {
            std::env::split_paths(path)
                .map(|dir| dir.join(name))
                .find(|p| {
                    std::fs::metadata(p)
                        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                        .unwrap_or(false)
                })
                .map(|p| (name.to_string(), p.to_string_lossy().into_owned()))
        })
        .collect()
}

/// `<path> --version`, first stdout line.
fn shell_version(path: &str) -> Option<String> {
    let s = run_with_timeout(Command::new(path).arg("--version"), Duration::from_secs(5))?;
    let line = s.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_string())
}

/// Run `cmd`, returning stdout if it exits 0 within `timeout`, else kill → None. Polls
/// `try_wait` — chosen over plain `.output()` so one hung binary can't wedge `status`
/// forever, and over a waiter thread because polling is std-only and simpler (no tokio,
/// per crate rules). ponytail: stdout is read after exit, so output > the ~64 KB pipe
/// buffer would deadlock the child into the kill path — fine for `--version` / PATH.
fn run_with_timeout(cmd: &mut Command, timeout: Duration) -> Option<String> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return None, // exited nonzero
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return None,
        }
    }
    let mut s = String::new();
    child.stdout.take()?.read_to_string(&mut s).ok()?;
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_resolve(found: &[(&str, &str)]) -> impl Fn(&[&str]) -> HashMap<String, String> {
        let map: HashMap<String, String> = found
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |_names| map.clone()
    }

    #[test]
    fn available_agent_has_path_and_version() {
        let agents = detect_agents(fake_resolve(&[("claude", "/opt/bin/claude")]), |path| {
            assert_eq!(path, "/opt/bin/claude");
            Some("2.1.0".into())
        });
        let claude = agents.iter().find(|a| a.name == "claude").unwrap();
        assert!(claude.available);
        assert_eq!(claude.path.as_deref(), Some("/opt/bin/claude"));
        assert_eq!(claude.version.as_deref(), Some("2.1.0"));
        assert!(claude.hint.is_none());
    }

    #[test]
    fn missing_agent_has_hint() {
        let agents = detect_agents(fake_resolve(&[]), |_| None);
        assert_eq!(agents.len(), 6); // every known agent is always listed
        let goose = agents.iter().find(|a| a.name == "goose").unwrap();
        assert!(!goose.available);
        assert!(goose.path.is_none());
        assert!(goose.version.is_none());
        assert_eq!(goose.hint.as_deref(), Some("brew install block/goose/goose"));
    }

    #[test]
    fn version_failure_still_available_with_null_version() {
        let agents = detect_agents(fake_resolve(&[("codex", "/usr/local/bin/codex")]), |_| None);
        let codex = agents.iter().find(|a| a.name == "codex").unwrap();
        assert!(codex.available);
        assert_eq!(codex.path.as_deref(), Some("/usr/local/bin/codex"));
        assert!(codex.version.is_none());
        assert!(codex.hint.is_none());
    }

    fn fake_shell(dir: &std::path::Path, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join("fake-shell");
        std::fs::write(&bin, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin.to_str().unwrap().to_string()
    }

    #[test]
    fn shell_path_normal() {
        let dir = tempfile::tempdir().unwrap();
        let sh = fake_shell(dir.path(), &format!("printf '{PATH_START}/a:/b{PATH_END}'"));
        assert_eq!(shell_path(&sh, Duration::from_secs(5)).as_deref(), Some("/a:/b"));
    }

    #[test]
    fn shell_path_ignores_rc_noise_around_markers() {
        let dir = tempfile::tempdir().unwrap();
        let sh = fake_shell(
            dir.path(),
            &format!("echo 'Welcome! PATH=/evil'\nprintf 'x{PATH_START}/nvm/bin:/usr/bin{PATH_END}y'\necho bye"),
        );
        assert_eq!(shell_path(&sh, Duration::from_secs(5)).as_deref(), Some("/nvm/bin:/usr/bin"));
    }

    #[test]
    fn shell_path_times_out_and_missing_markers_are_none() {
        let dir = tempfile::tempdir().unwrap();
        let sh = fake_shell(dir.path(), &format!("sleep 10; printf '{PATH_START}/a{PATH_END}'"));
        let t = Instant::now();
        assert!(shell_path(&sh, Duration::from_millis(300)).is_none());
        assert!(t.elapsed() < Duration::from_secs(5));
        let sh = fake_shell(dir.path(), "echo no markers here");
        assert!(shell_path(&sh, Duration::from_secs(5)).is_none());
        assert!(shell_path("/nonexistent/shell", Duration::from_secs(5)).is_none());
    }

    #[test]
    fn resolve_in_finds_first_executable_only() {
        use std::os::unix::fs::PermissionsExt;
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        std::fs::write(a.path().join("claude"), "").unwrap(); // not executable → skipped
        for (dir, name) in [(&b, "claude"), (&a, "npx"), (&b, "npx")] {
            let p = dir.path().join(name);
            std::fs::write(&p, "").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = std::env::join_paths([a.path(), b.path()]).unwrap();
        let map = resolve_in(path.to_str().unwrap(), &["claude", "npx", "goose"]);
        assert_eq!(map["claude"], b.path().join("claude").to_str().unwrap());
        assert_eq!(map["npx"], a.path().join("npx").to_str().unwrap());
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn shell_version_returns_first_line() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("fake-agent");
        std::fs::write(&bin, "#!/bin/sh\necho ' fake 1.2.3 '\necho second line\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(shell_version(bin.to_str().unwrap()).as_deref(), Some("fake 1.2.3"));
        assert!(shell_version("/usr/bin/true").is_none()); // exits 0, prints nothing
        assert!(shell_version("/usr/bin/false").is_none()); // exits nonzero
    }

    #[test]
    fn shell_version_nonexistent_binary_is_none() {
        assert!(shell_version("/nonexistent/binary").is_none());
    }
}
