//! AGT-13 stage 2: the Mac-side sandboxed code runner behind `POST /pocket/v1/run`.
//!
//! Code is untrusted (a stolen paired-phone token, a prompt-injected model, a
//! hostile web page the model read), so every run is:
//!  * executed under `/usr/bin/sandbox-exec -p <profile>` (Seatbelt). The tool
//!    is deprecated but present on every macOS release and is what many CLI
//!    sandboxes (Bazel, Nix, Homebrew) still use; if Apple removes it the run
//!    fails closed ("sandbox-exec is missing"). The profile is
//!    deny-by-default: reads only from system dirs and the per-run scratch
//!    dir, writes only inside the scratch dir and `/dev/null`, no network, no
//!    mach services, no reads of `/Users` or `/Volumes`, signals only within
//!    the sandbox. Python and JavaScript additionally cannot fork/spawn.
//!  * run with the scratch dir (`$TMPDIR/pocketai-run-<ulid>`, mode 0700,
//!    deleted afterwards and swept at startup) as cwd, `HOME` and `TMPDIR`,
//!  * in its own session/process group, killed with SIGKILL on timeout or
//!    output overflow, then swept for stragglers by cwd,
//!  * with rlimits (CPU, address space, file size, process count, core),
//!  * with a scrubbed environment (nothing from the app is inherited).
use std::ffi::OsString;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const MAX_CODE_BYTES: usize = 64 * 1024;
pub const OUTPUT_CAP: usize = 20 * 1024;
pub const DEFAULT_TIMEOUT_MS: u64 = 10_000;
pub const MAX_TIMEOUT_MS: u64 = 60_000;
// <sys/proc_info.h>; not exported by the libc crate.
#[cfg(target_os = "macos")]
const PROC_ALL_PIDS: u32 = 1;
#[cfg(target_os = "macos")]
const PROC_UID_ONLY: u32 = 4;
#[cfg(target_os = "macos")]
const PROC_PIDTASKINFO: libc::c_int = 4;
/// macOS accepts RLIMIT_AS but does not enforce it, so a watchdog polls the
/// run's resident size instead (the main process only, not its children).
const MEMORY_CAP: u64 = 1 << 30;
const SCRATCH_PREFIX: &str = "pocketai-run-";
const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";
const JSC: &str = "/System/Library/Frameworks/JavaScriptCore.framework/Versions/A/Helpers/jsc";

/// Deny by default; later rules win in SBPL, so the denies sit before the
/// scratch allow (which stays last in case TMPDIR is oddly under /Users).
/// Parameters: SCRATCH (canonical scratch dir), DEV (canonical developer dir,
/// for Python), EXTRA (extra read-only tree, e.g. /opt/homebrew for node).
/// The caller appends `(allow|deny) process-fork`.
const PROFILE_BASE: &str = r#"(version 1)
(deny default)
(allow process-exec)
(allow signal (target same-sandbox))
(allow sysctl-read)
(allow file-read-metadata)
(allow file-read*
  (literal "/") (subpath "/usr") (subpath "/System") (subpath "/Library/Frameworks")
  (subpath "/bin") (subpath "/sbin") (subpath "/private/etc")
  (subpath "/private/var/db/dyld") (subpath "/private/var/db/timezone")
  (subpath (param "DEV")) (subpath (param "EXTRA"))
  (literal "/dev/null") (literal "/dev/urandom") (literal "/dev/random") (literal "/dev/dtracehelper"))
(allow file-write* (literal "/dev/null"))
(deny network*)
(deny file-read* (subpath "/Users") (subpath "/Volumes") (subpath "/private/var/root"))
(allow file-read* file-write* (subpath (param "SCRATCH")))
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Python,
    JavaScript,
    Shell,
}

impl Language {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "python" => Some(Self::Python),
            "javascript" => Some(Self::JavaScript),
            "shell" => Some(Self::Shell),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::JavaScript => "javascript",
            Self::Shell => "shell",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub ok: bool,
    pub output: String,
    pub error: Option<String>,
    pub duration_ms: u64,
    pub truncated: bool,
    pub exit_code: Option<i32>,
}

pub fn clamp_timeout(ms: Option<u64>) -> u64 {
    ms.filter(|m| *m > 0)
        .unwrap_or(DEFAULT_TIMEOUT_MS)
        .clamp(100, MAX_TIMEOUT_MS)
}

/// What to exec and how the profile is parameterised.
struct Plan {
    program: PathBuf,
    args: Vec<OsString>,
    dev: PathBuf,
    extra: PathBuf,
    allow_fork: bool,
    /// File to create in the scratch dir before the run (name, contents).
    file: Option<(&'static str, String)>,
}

/// `Err` is a user-presentable "this Mac can't run that language" message.
fn plan(lang: Language, code: &str) -> Result<Plan, String> {
    let none = PathBuf::from("/var/empty/pocketai-none");
    match lang {
        Language::Shell => Ok(Plan {
            program: "/bin/zsh".into(),
            args: vec!["-f".into(), "-c".into(), code.into()],
            dev: none.clone(),
            extra: none,
            allow_fork: true,
            file: None,
        }),
        Language::Python => {
            // Never run the /usr/bin/python3 stub: without developer tools it
            // pops the "install command line tools" dialog. Resolve the real
            // interpreter from the developer dir instead.
            let missing = || "Python isn't installed on this Mac.".to_string();
            let out = Command::new("/usr/bin/xcode-select")
                .arg("-p")
                .env_clear()
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
                .map_err(|_| missing())?;
            if !out.status.success() {
                return Err(missing());
            }
            let dir = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
            let dev = std::fs::canonicalize(&dir).map_err(|_| missing())?;
            let py = std::fs::canonicalize(dir.join("usr/bin/python3")).map_err(|_| missing())?;
            if !py.starts_with(&dev) {
                return Err(missing());
            }
            Ok(Plan {
                // Run via the developer-dir symlink so Python finds its framework.
                program: dir.join("usr/bin/python3"),
                args: vec!["-I".into(), "-B".into(), "-u".into(), "main.py".into()],
                dev,
                extra: none,
                allow_fork: false,
                file: Some(("main.py", code.to_string())),
            })
        }
        Language::JavaScript => {
            let prelude = "if(typeof console==='undefined'){var console={log:print,info:print,warn:print,error:print};}";
            if Path::new(JSC).exists() {
                return Ok(Plan {
                    program: JSC.into(),
                    args: vec!["main.js".into()],
                    dev: none.clone(),
                    extra: none,
                    allow_fork: false,
                    file: Some(("main.js", format!("{prelude}{code}"))),
                });
            }
            for (node, tree) in [
                ("/opt/homebrew/bin/node", "/opt/homebrew"),
                ("/usr/local/bin/node", "/usr/local"),
            ] {
                if Path::new(node).exists() {
                    return Ok(Plan {
                        program: node.into(),
                        args: vec!["main.js".into()],
                        dev: none,
                        extra: tree.into(),
                        allow_fork: false,
                        file: Some(("main.js", code.to_string())),
                    });
                }
            }
            Err("JavaScript isn't available on this Mac.".into())
        }
    }
}

fn scratch_base() -> PathBuf {
    let t = std::env::temp_dir();
    std::fs::canonicalize(&t).unwrap_or(t)
}

/// Deletes the scratch dir (also when a script chmod'ed things unreadable).
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        remove_tree(&self.0);
    }
}

fn remove_tree(p: &Path) {
    if std::fs::remove_dir_all(p).is_ok() {
        return;
    }
    fn open_up(p: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700));
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                if e.file_type().is_ok_and(|t| t.is_dir()) {
                    open_up(&e.path());
                }
            }
        }
    }
    open_up(p);
    let _ = std::fs::remove_dir_all(p);
}

/// Startup sweep of scratch dirs a crash left behind. Only dirs older than
/// two minutes (a run lasts at most 60 s), so a server restart or a second
/// core never deletes a live run's folder.
pub fn sweep_scratch() {
    sweep_older_than(Duration::from_secs(120));
}

fn sweep_older_than(age: Duration) {
    let cutoff = std::time::SystemTime::now() - age;
    if let Ok(rd) = std::fs::read_dir(scratch_base()) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with(SCRATCH_PREFIX)
                && e.metadata().is_ok_and(|m| {
                    m.is_dir() && m.modified().is_ok_and(|t| t <= cutoff)
                })
            {
                remove_tree(&e.path());
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn proc_count_for_uid() -> usize {
    // SAFETY: size query only (null buffer).
    let n = unsafe {
        libc::proc_listpids(PROC_UID_ONLY, libc::getuid(), std::ptr::null_mut(), 0)
    };
    (n.max(0) as usize) / std::mem::size_of::<libc::pid_t>()
}

/// SIGKILLs every process whose cwd is inside `scratch` (catches children
/// that left the process group with setsid and were reparented to launchd).
#[cfg(target_os = "macos")]
fn kill_stragglers(scratch: &Path) {
    let sz = std::mem::size_of::<libc::pid_t>();
    // SAFETY: buffers are sized from the kernel's own answer; the info struct
    // is plain data that proc_pidinfo fills.
    unsafe {
        let n = libc::proc_listpids(PROC_ALL_PIDS, 0, std::ptr::null_mut(), 0);
        if n <= 0 {
            return;
        }
        let mut pids = vec![0 as libc::pid_t; n as usize / sz + 64];
        let n = libc::proc_listpids(
            PROC_ALL_PIDS,
            0,
            pids.as_mut_ptr() as *mut libc::c_void,
            (pids.len() * sz) as libc::c_int,
        );
        if n <= 0 {
            return;
        }
        for &pid in &pids[..n as usize / sz] {
            if pid <= 1 || pid == libc::getpid() {
                continue;
            }
            let mut info: libc::proc_vnodepathinfo = std::mem::zeroed();
            let r = libc::proc_pidinfo(
                pid,
                libc::PROC_PIDVNODEPATHINFO,
                0,
                &mut info as *mut _ as *mut libc::c_void,
                std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int,
            );
            if r <= 0 {
                continue;
            }
            let bytes: Vec<u8> = info
                .pvi_cdir
                .vip_path
                .iter()
                .flatten()
                .map(|c| *c as u8)
                .take_while(|b| *b != 0)
                .collect();
            if Path::new(std::str::from_utf8(&bytes).unwrap_or("")).starts_with(scratch) {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn resident_bytes(pid: i32) -> u64 {
    // SAFETY: proc_taskinfo is plain data filled by proc_pidinfo.
    unsafe {
        let mut ti: libc::proc_taskinfo = std::mem::zeroed();
        let n = libc::proc_pidinfo(
            pid,
            PROC_PIDTASKINFO,
            0,
            &mut ti as *mut _ as *mut libc::c_void,
            std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int,
        );
        if n > 0 { ti.pti_resident_size } else { 0 }
    }
}

// The runner only ever executes on the Mac (the route exists on iOS builds but
// is unreachable there: no paired phone, no sandbox-exec); these keep the iOS
// slices of the library compiling.
#[cfg(not(target_os = "macos"))]
fn proc_count_for_uid() -> usize {
    0
}
#[cfg(not(target_os = "macos"))]
fn kill_stragglers(_scratch: &Path) {}
#[cfg(not(target_os = "macos"))]
fn resident_bytes(_pid: i32) -> u64 {
    0
}

fn kill_all(pgid: i32, scratch: &Path) {
    // SAFETY: plain signal to the group we created with setsid.
    unsafe { libc::killpg(pgid, libc::SIGKILL) };
    kill_stragglers(scratch);
}

/// Runs `code` in the sandbox. `Err` = the language isn't available here.
pub fn run(lang: Language, code: &str, timeout_ms: u64) -> Result<Outcome, String> {
    let plan = plan(lang, code)?;
    if !Path::new(SANDBOX_EXEC).exists() {
        return Err("This Mac can't run code safely (sandbox-exec is missing).".into());
    }
    let timeout_ms = timeout_ms.clamp(100, MAX_TIMEOUT_MS);
    let dir = scratch_base().join(format!("{SCRATCH_PREFIX}{}", ulid::Ulid::new()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .map_err(|e| format!("Couldn't create a scratch folder: {e}"))?;
    let scratch = Scratch(dir.clone());
    if let Some((name, contents)) = &plan.file {
        std::fs::write(dir.join(name), contents)
            .map_err(|e| format!("Couldn't write the script: {e}"))?;
    }

    let mut profile = String::from(PROFILE_BASE);
    profile.push_str(if plan.allow_fork {
        "(allow process-fork)\n"
    } else {
        "(deny process-fork)\n"
    });
    let mut cmd = Command::new(SANDBOX_EXEC);
    cmd.arg("-D")
        .arg(format!("SCRATCH={}", dir.display()))
        .arg("-D")
        .arg(format!("DEV={}", plan.dev.display()))
        .arg("-D")
        .arg(format!("EXTRA={}", plan.extra.display()))
        .arg("-p")
        .arg(&profile)
        .arg(&plan.program)
        .args(&plan.args)
        .current_dir(&dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("HOME", &dir)
        .env("TMPDIR", &dir)
        .env("LANG", "en_US.UTF-8")
        .stdin(Stdio::null());

    // One pipe for stdout+stderr so their order is preserved.
    let mut fds = [0i32; 2];
    // SAFETY: fds is a 2-element array; both ends are wrapped right away.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err("Couldn't start the run.".into());
    }
    let (rd, wr) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    // SAFETY: fcntl on fds we own.
    unsafe {
        libc::fcntl(rd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
        libc::fcntl(wr.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
        let fl = libc::fcntl(rd.as_raw_fd(), libc::F_GETFL);
        libc::fcntl(rd.as_raw_fd(), libc::F_SETFL, fl | libc::O_NONBLOCK);
    }
    let wr2 = wr.try_clone().map_err(|_| "Couldn't start the run.".to_string())?;
    cmd.stdout(Stdio::from(wr2)).stderr(Stdio::from(wr));

    let cpu_secs = (timeout_ms / 1000 + 3).min(65) as libc::rlim_t;
    let nproc = (proc_count_for_uid() + 48) as libc::rlim_t;
    // SAFETY: only async-signal-safe calls (setsid, setrlimit, fcntl) run in
    // the child between fork and exec.
    unsafe {
        cmd.pre_exec(move || {
            libc::setsid();
            for (res, lim) in [
                (libc::RLIMIT_CPU, cpu_secs),
                (libc::RLIMIT_AS, 1 << 30),
                (libc::RLIMIT_FSIZE, 20 << 20),
                (libc::RLIMIT_NPROC, nproc),
                (libc::RLIMIT_CORE, 0),
            ] {
                let l = libc::rlimit {
                    rlim_cur: lim,
                    rlim_max: lim,
                };
                libc::setrlimit(res, &l); // best effort: macOS ignores some
            }
            // Nothing the app had open may leak into the child.
            for fd in 3..1024 {
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }
            Ok(())
        });
    }

    let start = Instant::now();
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Couldn't start the run: {e}"))?;
    drop(cmd); // closes our copies of the pipe's write end
    let pgid = child.id() as i32;
    let mut reader = std::fs::File::from(rd);

    let deadline = start + Duration::from_millis(timeout_ms);
    let mut buf = Vec::<u8>::new();
    let mut chunk = [0u8; 8192];
    let (mut timed_out, mut truncated, mut oom) = (false, false, false);
    let mut last_mem = start;
    let mut status = None;
    let mut take = |reader: &mut std::fs::File, buf: &mut Vec<u8>, truncated: &mut bool| -> bool {
        match reader.read(&mut chunk) {
            Ok(n) if n > 0 => {
                let room = OUTPUT_CAP.saturating_sub(buf.len());
                buf.extend_from_slice(&chunk[..n.min(room)]);
                *truncated |= n > room;
                true
            }
            _ => false,
        }
    };
    loop {
        let got = take(&mut reader, &mut buf, &mut truncated);
        if truncated {
            break;
        }
        if let Ok(Some(st)) = child.try_wait() {
            status = Some(st);
            // Take what the process wrote before it exited.
            while take(&mut reader, &mut buf, &mut truncated) {}
            break;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            break;
        }
        if last_mem.elapsed() >= Duration::from_millis(50) {
            last_mem = Instant::now();
            if resident_bytes(pgid) > MEMORY_CAP {
                oom = true;
                break;
            }
        }
        if !got {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    // Whatever happened, nothing the script started may outlive the run.
    kill_all(pgid, &dir);
    let status = status.or_else(|| child.wait().ok());
    drop(scratch);

    let exit_code = status.and_then(|s| s.code());
    let signal = status.and_then(|s| s.signal());
    let error = if timed_out {
        Some(format!("Timed out after {} s.", timeout_ms / 1000))
    } else if oom {
        Some("Used too much memory (limit 1 GB).".into())
    } else if truncated {
        Some("Output limit reached (20 KB); the run was stopped.".into())
    } else if let Some(sig) = signal {
        Some(match sig {
            libc::SIGXCPU => "Used too much CPU time.".to_string(),
            libc::SIGXFSZ => "Wrote too large a file.".to_string(),
            _ => format!("Stopped by signal {sig}."),
        })
    } else if exit_code != Some(0) {
        Some(format!("Exited with code {}.", exit_code.unwrap_or(-1)))
    } else {
        None
    };
    let mut output = String::from_utf8_lossy(&buf).into_owned();
    if truncated {
        output.push_str("\n[output truncated]");
    }
    Ok(Outcome {
        ok: error.is_none(),
        output,
        error,
        duration_ms: start.elapsed().as_millis() as u64,
        truncated,
        exit_code,
    })
}

#[cfg(test)]
mod tests {
    //! These really run under /usr/bin/sandbox-exec on this Mac: the
    //! path-escape / abuse suite from the AGT-13 release checklist. Python
    //! tests skip (with a note) on a Mac without developer tools.
    use super::*;

    /// The suite forks, floods and kills; running it in parallel would let
    /// one test eat another's process quota or scratch dir.
    fn run(lang: Language, code: &str, ms: u64) -> Result<Outcome, String> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        super::run(lang, code, ms)
    }

    fn sh(code: &str) -> Outcome {
        run(Language::Shell, code, 10_000).unwrap()
    }

    fn home() -> String {
        std::env::var("HOME").unwrap()
    }

    fn ps() -> String {
        let o = Command::new("/bin/ps").args(["-axo", "command"]).output().unwrap();
        String::from_utf8_lossy(&o.stdout).into_owned()
    }

    /// A readable secret inside the real $HOME that the sandbox must not reach.
    struct Canary(PathBuf);
    impl Canary {
        fn new(name: &str) -> Self {
            let p = PathBuf::from(home()).join(name);
            std::fs::write(&p, "TOPSECRET").unwrap();
            Canary(p)
        }
    }
    impl Drop for Canary {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn real_interpreters_run_in_the_sandbox() {
        let o = sh("echo hi");
        assert!(o.ok, "{o:?}");
        assert_eq!(o.output.trim(), "hi");
        let o = run(Language::JavaScript, "console.log(6*7); print('x')", 10_000).unwrap();
        assert!(o.ok, "{o:?}");
        assert_eq!(o.output.trim(), "42\nx");
        match run(Language::Python, "print(1+1)", 10_000) {
            Ok(o) => {
                assert!(o.ok, "{o:?}");
                assert_eq!(o.output.trim(), "2");
            }
            Err(e) => eprintln!("python test skipped: {e}"),
        }
    }

    #[test]
    fn home_and_users_are_unreadable() {
        let c = Canary::new(".pocketai-canary-read");
        let h = home();
        let o = sh(&format!(
            "cat '{}'; ls /Users; ls {h}/.ssh; cat {h}/.zshrc; ls /Volumes",
            c.0.display()
        ));
        assert!(!o.output.contains("TOPSECRET"), "{o:?}");
        assert!(o.output.matches("Operation not permitted").count() >= 4, "{o:?}");
        if let Ok(o) = run(Language::Python, &format!("print(open('{}').read())", c.0.display()), 10_000) {
            assert!(!o.output.contains("TOPSECRET") && !o.ok, "{o:?}");
        }
        let o = run(Language::JavaScript, &format!("print(readFile('{}'))", c.0.display()), 10_000).unwrap();
        assert!(!o.output.contains("TOPSECRET"), "{o:?}");
    }

    #[test]
    fn writes_outside_scratch_fail_but_scratch_works() {
        let name = format!("pocketai-escape-{}", ulid::Ulid::new());
        let tmp = format!("/tmp/{name}");
        let in_home = format!("{}/{name}", home());
        let o = sh(&format!(
            "echo a > {tmp}; echo a > {in_home}; echo a > ../{name}; echo a > ./ok; cat ok"
        ));
        assert!(!Path::new(&tmp).exists() && !Path::new(&in_home).exists());
        assert!(!scratch_base().join(&name).exists());
        assert!(o.output.trim_end().ends_with('a'), "scratch write must work: {o:?}");
        assert!(o.output.matches("not permitted").count() >= 3, "{o:?}");
    }

    #[test]
    fn symlink_and_hardlink_tricks_fail() {
        let c = Canary::new(".pocketai-canary-link");
        let o = sh(&format!(
            "ln -s '{p}' s; cat s; ln -s /Users/ u; ls u; ln '{p}' h; cat h; ln -s /tmp t; echo a > t/pocketai-link-escape",
            p = c.0.display()
        ));
        assert!(!o.output.contains("TOPSECRET"), "{o:?}");
        assert!(!Path::new("/tmp/pocketai-link-escape").exists(), "{o:?}");
    }

    #[test]
    fn network_is_denied() {
        let o = sh("/usr/bin/curl -sS -m 3 http://1.1.1.1 2>&1; echo rc=$?");
        assert!(!o.output.contains("rc=0"), "{o:?}");
        if let Ok(o) = run(
            Language::Python,
            "import socket\ntry:\n socket.create_connection(('1.1.1.1',80),3)\n print('CONNECTED')\nexcept OSError as e:\n print('denied',e)",
            10_000,
        ) {
            assert!(o.output.contains("denied") && !o.output.contains("CONNECTED"), "{o:?}");
        }
    }

    #[test]
    fn python_cannot_spawn_processes() {
        if let Ok(o) = run(Language::Python, "import subprocess\nsubprocess.run(['/bin/echo','x'])", 10_000) {
            assert!(!o.ok && !o.output.starts_with('x'), "{o:?}");
        }
    }

    #[test]
    fn env_is_scrubbed() {
        std::env::set_var("POCKETAI_TEST_SECRET", "s3cret");
        let o = sh("env");
        assert!(!o.output.contains("s3cret"), "{o:?}");
        assert!(o.output.contains("PATH=/usr/bin:/bin:/usr/sbin:/sbin"));
    }

    #[test]
    fn infinite_loop_is_killed_at_the_timeout_and_nothing_is_left() {
        let t = Instant::now();
        let o = run(Language::Shell, "while true; do :; done", 1_000).unwrap();
        assert!(!o.ok && o.error.as_deref().unwrap().starts_with("Timed out"), "{o:?}");
        assert!(t.elapsed() < Duration::from_secs(4));
        // Background children must not survive the run either.
        let o = run(Language::Shell, "sleep 300 & sleep 301 & sleep 0.2; echo started", 5_000).unwrap();
        assert!(o.ok, "{o:?}");
        std::thread::sleep(Duration::from_millis(300));
        let ps = ps();
        assert!(!ps.contains("sleep 300") && !ps.contains("sleep 301"), "leftover process");
    }

    #[test]
    fn setsid_orphans_are_swept_by_cwd() {
        let o = run(
            Language::Shell,
            "/usr/bin/perl -e 'use POSIX; if(fork()==0){POSIX::setsid(); exec(\"/bin/sleep\",\"302\")} sleep 1'; echo done",
            5_000,
        )
        .unwrap();
        assert!(o.output.contains("done"), "{o:?}");
        std::thread::sleep(Duration::from_millis(300));
        assert!(!ps().contains("sleep 302"), "orphan survived");
    }

    #[test]
    fn output_is_capped_at_20kb() {
        let o = sh("yes");
        assert!(o.truncated && !o.ok, "{o:?}");
        assert!(o.output.len() <= OUTPUT_CAP + 32, "{}", o.output.len());
    }

    #[test]
    fn fork_bomb_is_contained() {
        let t = Instant::now();
        let o = run(Language::Shell, ":(){ :|:& };:; sleep 5", 2_000).unwrap();
        assert!(!o.ok, "{o:?}");
        assert!(t.elapsed() < Duration::from_secs(8));
        std::thread::sleep(Duration::from_millis(500));
        // The machine still forks, and the bomb is gone.
        assert!(sh("echo alive").ok);
    }

    #[test]
    fn scratch_is_cwd_private_and_deleted() {
        let o = sh("pwd -P; ls -ld . | cut -c1-10");
        let mut lines = o.output.lines();
        let cwd = PathBuf::from(lines.next().unwrap());
        assert!(cwd.file_name().unwrap().to_string_lossy().starts_with(SCRATCH_PREFIX), "{o:?}");
        assert_eq!(lines.next(), Some("drwx------"));
        assert!(!cwd.exists(), "scratch dir must be removed");
        // startup sweep
        let stale = scratch_base().join(format!("{SCRATCH_PREFIX}stale"));
        std::fs::create_dir_all(stale.join("sub")).unwrap();
        let old = std::time::SystemTime::now() - Duration::from_secs(3600);
        std::fs::File::open(&stale).unwrap().set_modified(old).unwrap();
        sweep_older_than(Duration::from_secs(60));
        assert!(!stale.exists());
    }

    #[test]
    fn memory_bomb_is_stopped() {
        let o = run(Language::Python, "x = bytearray(3*1024**3)\nx[::4096] = b'a'*len(x[::4096])\nprint('ALLOCATED')", 10_000)
            .expect("python must be installed for this test");
        eprintln!("memory bomb: {o:?}");
        assert!(!o.output.contains("ALLOCATED"), "{o:?}");
    }

    #[test]
    fn timeout_is_clamped() {
        assert_eq!(clamp_timeout(None), 10_000);
        assert_eq!(clamp_timeout(Some(0)), 10_000);
        assert_eq!(clamp_timeout(Some(999_999)), 60_000);
        assert_eq!(clamp_timeout(Some(5)), 100);
    }
}
