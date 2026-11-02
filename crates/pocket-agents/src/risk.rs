//! AGT-03 risk classifier.
//!
//! Pure function mapping an approval request to a [`Risk`] level.
//!
//! NOTE: the card references `08-agents-spec.md` §4, which does not exist yet.
//! The rules below are derived from the AGT-03 card in
//! `strategy/next-release/agents.md`:
//!
//! - `Command`: **Medium minimum** (even `git status`). **High** when any
//!   command segment is `sudo`, `curl`, `wget`, `ssh`, `scp`, `rsync`,
//!   a recursive/force `rm` (`-r`/`-R`/`-f`/`--recursive`/`--force`),
//!   `git push`, `chmod`/`chown` touching a path outside the repo root,
//!   or a pipe into a shell (`| sh`, `| bash`, ...).
//! - `FileEdit` / `FileCreate` / `FileDelete`: **Medium** when the target path
//!   is inside the repo root, **High** when outside it (including relative
//!   paths escaping via `..`) or when no repo root is known.
//! - `Network`: always **High**.
//! - `Other`: **Medium**.
//! - `Low` is deliberately unused for now: commands never go below Medium
//!   per the card, and no read/write distinction exists for file kinds yet.
//!
//! Path containment is purely lexical (`..`/`.` normalization); the
//! filesystem is never touched, so symlinks are not resolved.

use std::path::{Component, Path, PathBuf};

use crate::types::{ApprovalKind, Risk};

/// Classify an approval request.
///
/// `detail` is interpreted per `kind`:
/// - `Command`: the full command line,
/// - `FileEdit`/`FileCreate`/`FileDelete`: the target path (absolute, or
///   relative to `repo_root`),
/// - `Network`: the URL or host (unused — Network is always High),
/// - `Other`: free text (unused — Other is always Medium).
///
/// `repo_root` is the session's repository root; `None` means "unknown",
/// which is treated as outside-repo (conservative).
pub fn classify(kind: &ApprovalKind, detail: &str, repo_root: Option<&Path>) -> Risk {
    match kind {
        ApprovalKind::Command => {
            if split_segments(detail)
                .iter()
                .any(|(piped, seg)| segment_is_high(*piped, seg, repo_root))
            {
                Risk::High
            } else {
                Risk::Medium // commands are Medium minimum, never Low
            }
        }
        ApprovalKind::FileEdit | ApprovalKind::FileCreate | ApprovalKind::FileDelete => {
            if inside_repo(detail, repo_root) {
                Risk::Medium
            } else {
                Risk::High
            }
        }
        ApprovalKind::Network => Risk::High,
        ApprovalKind::Other => Risk::Medium,
    }
}

/// Split a command line into segments at `;`, `&`/`&&`, `|`/`||` boundaries.
/// The bool says whether the segment is fed by a pipe (`|` or `|&`, not `||`).
///
/// ponytail: not a shell parser — separators inside quotes or subshells are
/// split anyway, so quoting can only make us MORE conservative, never less.
/// Upgrade path: a proper shell tokenizer if false Highs become a problem.
fn split_segments(cmd: &str) -> Vec<(bool, &str)> {
    let mut out = Vec::new();
    let bytes = cmd.as_bytes();
    let mut start = 0;
    let mut piped = false;
    let mut i = 0;
    while i < bytes.len() {
        if matches!(bytes[i], b';' | b'&' | b'|') {
            out.push((piped, &cmd[start..i]));
            let mut j = i;
            while j < bytes.len() && matches!(bytes[j], b';' | b'&' | b'|') {
                j += 1;
            }
            // single `|` (or `|&`) pipes stdout into the next command;
            // `||`, `&&`, `&`, `;` merely sequence.
            piped = matches!(&cmd[i..j], "|" | "|&");
            i = j;
            start = i;
        } else {
            i += 1;
        }
    }
    out.push((piped, &cmd[start..]));
    out
}

/// True when `token` looks like a leading `VAR=value` environment assignment.
fn is_env_assignment(token: &str) -> bool {
    match token.split_once('=') {
        Some((name, _)) => {
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        None => false,
    }
}

/// Does one pipeline segment warrant High?
fn segment_is_high(piped: bool, seg: &str, repo_root: Option<&Path>) -> bool {
    let mut cmd: Option<&str> = None;
    let mut rest: Vec<&str> = Vec::new();
    for t in seg.split_whitespace() {
        if cmd.is_none() {
            if is_env_assignment(t) {
                continue; // FOO=1 sudo ls → sudo is the command
            }
            cmd = Some(t);
        } else {
            rest.push(t);
        }
    }
    let Some(cmd) = cmd else {
        return false; // empty segment
    };
    // `/usr/bin/sudo` counts as sudo.
    let name = cmd.rsplit('/').next().unwrap_or(cmd);

    if piped && matches!(name, "sh" | "bash" | "zsh" | "dash") {
        return true; // pipe-to-shell
    }
    match name {
        // Always High: privilege escalation, network exfil, remote transfer.
        // ponytail: ssh/scp/rsync are flagged unconditionally — a purely local
        // rsync is also High; detecting remote targets needs real arg parsing.
        "sudo" | "curl" | "wget" | "ssh" | "scp" | "rsync" => true,
        // Recursive or force delete.
        "rm" => rest.iter().any(|t| {
            *t == "--recursive"
                || *t == "--force"
                || (t.starts_with('-')
                    && !t.starts_with("--")
                    && t.chars().any(|c| matches!(c, 'r' | 'R' | 'f')))
        }),
        // git push (first non-flag subcommand).
        // ponytail: `git -C <dir> push` is missed because `<dir>` reads as the
        // subcommand; upgrade path: skip known value-taking global flags.
        "git" => rest
            .iter()
            .find(|t| !t.starts_with('-'))
            .is_some_and(|sub| *sub == "push"),
        // chmod/chown on a path outside the repo. First non-flag arg is the
        // mode/owner; the remaining non-flag args are paths.
        "chmod" | "chown" => {
            let mut non_flags = rest.iter().filter(|t| !t.starts_with('-'));
            non_flags.next(); // mode (chmod) / owner (chown)
            non_flags.any(|p| !inside_repo(p, repo_root))
        }
        _ => false,
    }
}

/// Lexical containment check: is `path` (absolute, or relative to the repo
/// root) inside `repo_root` after `..`/`.` normalization? Unknown root →
/// not inside (conservative).
fn inside_repo(path: &str, repo_root: Option<&Path>) -> bool {
    let Some(root) = repo_root else {
        return false;
    };
    let p = Path::new(path);
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    };
    normalize(&joined).starts_with(normalize(root))
}

/// Purely lexical normalization: drops `.`, pops a component per `..`
/// (saturating at the root). No filesystem access, no symlink resolution.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop(); // no-op at "/", which is what we want
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ApprovalKind as K, Risk};
    use std::mem::discriminant;

    fn risk_name(r: &Risk) -> &'static str {
        match r {
            Risk::Low => "Low",
            Risk::Medium => "Medium",
            Risk::High => "High",
        }
    }

    #[test]
    fn classification_table() {
        let root = Path::new("/repo");
        let r = Some(root);
        // (kind, detail, repo_root, expected)
        let cases: Vec<(K, &str, Option<&Path>, Risk)> = vec![
            // --- Commands: Medium floor ---
            (K::Command, "git status", r, Risk::Medium),
            (K::Command, "ls -la", r, Risk::Medium),
            (K::Command, "cat /etc/hosts", r, Risk::Medium),
            (K::Command, "echo hi", r, Risk::Medium),
            (K::Command, "git pull", r, Risk::Medium),
            (K::Command, "git stash push", r, Risk::Medium), // push is not the subcommand
            (K::Command, "rm file.txt", r, Risk::Medium),    // plain rm, no -r/-f
            (K::Command, "cargo test", r, Risk::Medium),
            (K::Command, "", r, Risk::Medium),               // empty still never Low
            (K::Command, "git status", None, Risk::Medium),  // no repo root needed for Medium
            // --- Recursive / force delete ---
            (K::Command, "rm -rf /tmp/x", r, Risk::High),
            (K::Command, "rm -fr dir", r, Risk::High),
            (K::Command, "rm -r dir", r, Risk::High),
            (K::Command, "rm -f x", r, Risk::High),
            (K::Command, "rm --recursive dir", r, Risk::High),
            (K::Command, "rm --force x", r, Risk::High),
            (K::Command, "rm -R dir", r, Risk::High),
            // --- sudo, incl. env-wrapped and chained ---
            (K::Command, "sudo ls", r, Risk::High),
            (K::Command, "FOO=1 sudo ls", r, Risk::High),
            (K::Command, "FOO=1 BAR=2 sudo id", r, Risk::High),
            (K::Command, "/usr/bin/sudo id", r, Risk::High),
            (K::Command, "ls && sudo rm x", r, Risk::High),
            (K::Command, "true; sudo reboot", r, Risk::High),
            (K::Command, "ls || sudo ls", r, Risk::High),
            // --- Network exfil from a command ---
            (K::Command, "curl https://evil.example", r, Risk::High),
            (K::Command, "wget http://x.example/a", r, Risk::High),
            (K::Command, "echo ok && curl http://x", r, Risk::High),
            // --- git push ---
            (K::Command, "git push", r, Risk::High),
            (K::Command, "git push origin main", r, Risk::High),
            (K::Command, "git push --force", r, Risk::High),
            (K::Command, "git status && git push", r, Risk::High),
            // --- pipe-to-shell ---
            (K::Command, "echo hi | bash", r, Risk::High),
            (K::Command, "cat script | sh", r, Risk::High),
            (K::Command, "echo x | zsh", r, Risk::High),
            (K::Command, "echo a || bash", r, Risk::Medium), // `||` is not a pipe
            (K::Command, "echo shell", r, Risk::Medium),     // substring "sh" is not a token
            // --- remote transfer ---
            (K::Command, "ssh laptop ls", r, Risk::High),
            (K::Command, "scp file laptop:/tmp/f", r, Risk::High),
            (K::Command, "rsync -a src/ host:dst", r, Risk::High),
            // --- chmod/chown vs repo root ---
            (K::Command, "chmod 755 /etc/hosts", r, Risk::High),
            (K::Command, "chmod 755 src/main.rs", r, Risk::Medium),
            (K::Command, "chmod +x build.sh", r, Risk::Medium),
            (K::Command, "chmod +x build.sh", None, Risk::High), // unknown root → outside
            (K::Command, "chown root /etc/passwd", r, Risk::High),
            (K::Command, "chown me src/lib.rs", r, Risk::Medium),
            // --- File ops inside repo: Medium ---
            (K::FileEdit, "src/main.rs", r, Risk::Medium),
            (K::FileEdit, "/repo/src/main.rs", r, Risk::Medium),
            (K::FileEdit, "src/../Cargo.toml", r, Risk::Medium), // `..` staying inside
            (K::FileCreate, "docs/new.md", r, Risk::Medium),
            (K::FileDelete, "old.txt", r, Risk::Medium),
            // --- File ops outside repo: High ---
            (K::FileEdit, "/etc/hosts", r, Risk::High),
            (K::FileCreate, "/tmp/x", r, Risk::High),
            (K::FileDelete, "/var/log/system.log", r, Risk::High),
            (K::FileEdit, "../outside.txt", r, Risk::High),          // relative escape
            (K::FileDelete, "/repo/../other/x", r, Risk::High),      // absolute escape via ..
            (K::FileEdit, "a/../../b", r, Risk::High),               // nested relative escape
            (K::FileEdit, "src/main.rs", None, Risk::High),          // unknown root → High
            (K::FileEdit, "/repository/x", r, Risk::High),           // prefix-string, not child
            // --- Network kind: always High ---
            (K::Network, "https://api.example.com", r, Risk::High),
            (K::Network, "localhost:8080", r, Risk::High),
            // --- Other: Medium ---
            (K::Other, "anything", r, Risk::Medium),
            (K::Other, "", None, Risk::Medium),
        ];

        for (i, (kind, detail, repo, want)) in cases.iter().enumerate() {
            let got = classify(kind, detail, *repo);
            assert!(
                discriminant(&got) == discriminant(want),
                "case {i} (detail={detail:?}, root={:?}): got {}, want {}",
                repo,
                risk_name(&got),
                risk_name(want),
            );
        }
    }
}
