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
pub fn detect() -> Vec<AgentInfo> {
    detect_agents(shell_resolve, shell_version)
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

/// Production resolver: ONE `/bin/zsh -lc` invocation echoing `name=path` per found
/// command (missing commands print nothing), rather than parsing bare `command -v a b c`
/// output where you can't tell which paths belong to which names.
fn shell_resolve(names: &[&str]) -> HashMap<String, String> {
    let script = format!(
        r#"for c in {}; do p=$(command -v "$c") && echo "$c=$p"; done; true"#,
        names.join(" ")
    );
    match Command::new("/bin/zsh").args(["-lc", &script]).output() {
        Ok(out) => parse_resolved(&String::from_utf8_lossy(&out.stdout)),
        Err(_) => HashMap::new(), // no zsh → nothing found; everything gets a hint
    }
}

fn parse_resolved(stdout: &str) -> HashMap<String, String> {
    stdout
        .lines()
        .filter_map(|l| l.split_once('='))
        // command -v can print non-paths for aliases/functions; keep absolute paths only.
        .filter(|(_, v)| v.starts_with('/'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

/// `<path> --version`, first stdout line. Timeout: poll `try_wait` for up to 5 s then
/// kill — chosen over plain `.output()` so one hung binary can't wedge `status` forever,
/// and over a waiter thread because polling is std-only and simpler (no tokio, per crate
/// rules). ponytail: stdout is read after exit, so output > the ~64 KB pipe buffer would
/// deadlock the child into the kill path — fine for `--version`.
fn shell_version(path: &str) -> Option<String> {
    let mut child = Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(5);
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
    let line = s.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_string())
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

    #[test]
    fn parse_resolved_skips_noise_and_non_paths() {
        let map = parse_resolved("claude=/opt/bin/claude\nnoise line\nnpx=alias npx\n");
        assert_eq!(map.get("claude").map(String::as_str), Some("/opt/bin/claude"));
        assert!(!map.contains_key("npx"));
        assert_eq!(map.len(), 1);
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
