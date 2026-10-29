//! Running a shell command for an agent. This is the one tool that is not
//! limited to the agent's folder: the command runs on this computer as the
//! user, with the folder only as its starting directory. It is opt-in (a
//! Settings switch, then per agent), asks before every run unless the user
//! allows a command prefix for the session, stops after a time limit, keeps
//! only the start of its output and does not inherit obvious secrets from
//! the environment.

use super::tools::Limits;
use super::workspace::ToolError;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Longest command Roost will run, in characters.
pub const MAX_COMMAND_CHARS: usize = 4_000;

/// Heads for which "allow for this session" is never offered: shells and
/// interpreters (they run anything), and tools that delete, overwrite, reach
/// the network or raise privileges. Each such command is asked about alone.
const NEVER_PREFIXED: &[&str] = &[
    "sh",
    "bash",
    "zsh",
    "fish",
    "dash",
    "ksh",
    "csh",
    "tcsh",
    "cmd",
    "powershell",
    "pwsh",
    "env",
    "xargs",
    "sudo",
    "su",
    "doas",
    "eval",
    "exec",
    "rm",
    "rmdir",
    "del",
    "rd",
    "erase",
    "mv",
    "move",
    "cp",
    "copy",
    "dd",
    "chmod",
    "chown",
    "mkfs",
    "curl",
    "wget",
    "ssh",
    "scp",
    "sftp",
    "nc",
    "ncat",
    "telnet",
    "ftp",
    "python",
    "python3",
    "py",
    "node",
    "deno",
    "bun",
    "ruby",
    "perl",
    "php",
    "lua",
    "osascript",
    "open",
    "start",
    "reg",
    "schtasks",
    "crontab",
    "kill",
    "pkill",
    "taskkill",
    "git-credential",
];

/// Characters that let one command carry another (chaining, pipes,
/// redirection, substitution, line breaks). A command with any of them is
/// asked about on its own, never covered by an earlier "allow".
const CHAINING: &[char] = &[
    ';', '&', '|', '<', '>', '`', '$', '(', ')', '\n', '\r', '\\', '^', '%',
];

/// The prefix that "Allow `<prefix>` this session" covers: the program, and
/// its subcommand when it has a plain one (`cargo test`, `git status`).
/// `None` when the command may not be covered by an allowance at all.
pub fn allow_prefix(command: &str) -> Option<String> {
    let command = command.trim();
    if command.is_empty() || command.contains(CHAINING) {
        return None;
    }
    let mut words = command.split_whitespace();
    let head = words.next()?;
    let lower = head.to_lowercase();
    let bare = lower.rsplit(['/', '\\']).next().unwrap_or(&lower);
    let bare = bare.strip_suffix(".exe").unwrap_or(bare);
    if NEVER_PREFIXED.contains(&bare) || head.contains(['/', '\\']) {
        return None;
    }
    let plain = |w: &str| {
        !w.is_empty()
            && !w.starts_with('-')
            && w.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    };
    match words.next() {
        Some(sub) if plain(sub) => Some(format!("{head} {sub}")),
        _ => Some(head.to_string()),
    }
}

/// Whether `command` is covered by an allowed prefix: it starts with the
/// prefix as whole words and carries nothing chained to it.
pub fn covered_by(command: &str, prefix: &str) -> bool {
    allow_prefix(command).is_some_and(|own| {
        own == prefix || own.strip_prefix(prefix).is_some_and(|r| r.starts_with(' '))
    })
}

/// Whether an environment variable should not reach a command.
fn withheld(name: &str) -> bool {
    let n = name.to_uppercase();
    n.starts_with("HF_")
        || n.starts_with("ROOST_")
        || n.starts_with("AWS_")
        || n.starts_with("GITHUB_")
        || n.starts_with("GH_")
        || n.starts_with("ANTHROPIC_")
        || n.starts_with("OPENAI_")
        || n.starts_with("SSH_")
        || [
            "TOKEN",
            "SECRET",
            "PASSWORD",
            "PASSWD",
            "CREDENTIAL",
            "APIKEY",
            "API_KEY",
            "PRIVATE_KEY",
        ]
        .iter()
        .any(|w| n.contains(w))
}

/// What an approval card shows about a command that is about to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunPreview {
    pub command: String,
    /// Where it starts.
    pub folder: std::path::PathBuf,
    /// What "allow for this session" would cover; `None` when nothing may be.
    pub prefix: Option<String>,
    pub timeout_secs: u64,
    pub output_kb: usize,
}

struct Capture {
    bytes: Vec<u8>,
    cut: bool,
}

/// Reads a pipe into the shared capture up to `cap` bytes and keeps reading
/// (discarding) afterwards, so the command never blocks on a full pipe.
fn drain(mut pipe: impl Read + Send + 'static, out: Arc<Mutex<Capture>>, cap: usize) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match pipe.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut c = out.lock().unwrap_or_else(|p| p.into_inner());
                    let room = cap.saturating_sub(c.bytes.len());
                    if room >= n {
                        c.bytes.extend_from_slice(&buf[..n]);
                    } else {
                        c.bytes.extend_from_slice(&buf[..room]);
                        c.cut = true;
                    }
                }
            }
        }
    });
}

/// Stops a command and everything it started.
fn kill_tree(child: &mut std::process::Child) {
    let pid = child.id().to_string();
    #[cfg(unix)]
    {
        // The command leads its own process group; signal the whole group.
        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{pid}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/PID", &pid, "/T", "/F"])
            .creation_flags(0x0800_0000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}

/// Runs `command` in `root`, waiting at most `lim.run_timeout` and stopping
/// early when `cancel` is set. A command that exits non-zero is a normal
/// result (the model needs its output); only failing to start is an error.
pub fn run(
    root: &Path,
    command: &str,
    lim: &Limits,
    cancel: &AtomicBool,
) -> Result<String, ToolError> {
    let command = command.trim();
    if command.is_empty() {
        return Err(ToolError::InvalidArguments(
            "command must not be empty".into(),
        ));
    }
    if command.contains('\0') || command.chars().count() > MAX_COMMAND_CHARS {
        return Err(ToolError::InvalidArguments(format!(
            "command must be plain text of at most {MAX_COMMAND_CHARS} characters"
        )));
    }
    #[cfg(unix)]
    let mut cmd = {
        use std::os::unix::process::CommandExt;
        let mut c = Command::new("sh");
        c.arg("-c").arg(command).process_group(0);
        c
    };
    #[cfg(windows)]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        let mut c = Command::new("cmd");
        c.arg("/C").raw_arg(command).creation_flags(0x0800_0000);
        c
    };
    cmd.current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, _) in std::env::vars_os() {
        if name.to_str().is_some_and(withheld) {
            cmd.env_remove(&name);
        }
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| ToolError::Unreadable(format!("could not start the command: {e}")))?;
    let capture = Arc::new(Mutex::new(Capture {
        bytes: Vec::new(),
        cut: false,
    }));
    if let Some(p) = child.stdout.take() {
        drain(p, Arc::clone(&capture), lim.run_output_bytes);
    }
    if let Some(p) = child.stderr.take() {
        drain(p, Arc::clone(&capture), lim.run_output_bytes);
    }
    let started = Instant::now();
    let (status, why) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (Some(status), None),
            Ok(None) => {}
            Err(e) => {
                kill_tree(&mut child);
                return Err(ToolError::Unreadable(format!(
                    "could not wait for the command: {e}"
                )));
            }
        }
        if cancel.load(Ordering::SeqCst) {
            kill_tree(&mut child);
            let _ = child.wait();
            break (None, Some("was stopped"));
        }
        if started.elapsed() >= lim.run_timeout {
            kill_tree(&mut child);
            let _ = child.wait();
            break (None, Some("ran too long and was stopped"));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // Let the readers take what the pipes still hold.
    std::thread::sleep(Duration::from_millis(30));
    let (text, cut) = {
        let c = capture.lock().unwrap_or_else(|p| p.into_inner());
        (String::from_utf8_lossy(&c.bytes).into_owned(), c.cut)
    };
    let mut out = match (status, why) {
        (Some(s), _) => match s.code() {
            Some(code) => format!("exit code: {code}\n"),
            None => "the command was ended by a signal\n".to_string(),
        },
        (None, Some(why)) => format!(
            "The command {why} after {} seconds.\n",
            started.elapsed().as_secs().max(1)
        ),
        (None, None) => String::new(),
    };
    out.push_str(&text);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    if cut {
        out.push_str(&format!(
            "[output cut at {} KB]\n",
            (lim.run_output_bytes / 1024).max(1)
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn lim() -> Limits {
        Limits::default()
    }

    fn go(dir: &TempDir, command: &str, lim: &Limits) -> String {
        run(dir.path(), command, lim, &AtomicBool::new(false)).unwrap()
    }

    #[test]
    fn a_command_runs_and_reports_its_exit_code_and_output() {
        let d = TempDir::new().unwrap();
        let out = go(&d, "echo hello", &lim());
        assert!(out.starts_with("exit code: 0\n"), "{out}");
        assert!(out.contains("hello"), "{out}");
        let out = go(&d, "exit 3", &lim());
        assert!(out.starts_with("exit code: 3\n"), "{out}");
    }

    #[test]
    fn a_command_starts_in_the_workspace_folder() {
        let d = TempDir::new().unwrap();
        std::fs::write(d.path().join("marker.txt"), "x").unwrap();
        #[cfg(unix)]
        let listing = "ls";
        #[cfg(windows)]
        let listing = "dir /b";
        assert!(go(&d, listing, &lim()).contains("marker.txt"));
    }

    #[test]
    fn an_empty_or_enormous_command_is_refused_before_anything_runs() {
        let d = TempDir::new().unwrap();
        let c = AtomicBool::new(false);
        assert!(run(d.path(), "   ", &lim(), &c).is_err());
        assert!(run(d.path(), &"x".repeat(MAX_COMMAND_CHARS + 1), &lim(), &c).is_err());
        assert!(run(d.path(), "echo a\0b", &lim(), &c).is_err());
    }

    #[test]
    fn a_command_that_runs_too_long_is_stopped_with_its_output_so_far() {
        let d = TempDir::new().unwrap();
        let mut l = lim();
        l.run_timeout = Duration::from_millis(400);
        #[cfg(unix)]
        let slow = "echo before; sleep 20";
        #[cfg(windows)]
        let slow = "echo before & ping -n 20 127.0.0.1 >nul";
        let t = Instant::now();
        let out = go(&d, slow, &l);
        assert!(t.elapsed() < Duration::from_secs(8), "{:?}", t.elapsed());
        assert!(out.contains("ran too long and was stopped"), "{out}");
        assert!(out.contains("before"), "{out}");
    }

    #[cfg(unix)]
    #[test]
    fn stopping_ends_what_the_command_started_too() {
        let d = TempDir::new().unwrap();
        let mut l = lim();
        l.run_timeout = Duration::from_millis(300);
        // A background child that holds the output pipe open.
        let t = Instant::now();
        let out = go(&d, "sleep 20 & sleep 20", &l);
        assert!(t.elapsed() < Duration::from_secs(8), "{:?}", t.elapsed());
        assert!(out.contains("was stopped"), "{out}");
    }

    #[test]
    fn output_is_cut_and_says_so() {
        let d = TempDir::new().unwrap();
        let mut l = lim();
        l.run_output_bytes = 1_024;
        #[cfg(unix)]
        let noisy = "yes line | head -n 5000";
        #[cfg(windows)]
        let noisy = "for /L %i in (1,1,3000) do @echo line%i";
        let out = go(&d, noisy, &l);
        assert!(out.contains("[output cut at 1 KB]"), "{out}");
        assert!(out.len() < 1_024 + 200, "{}", out.len());
    }

    #[test]
    fn the_cancel_flag_ends_a_running_command_promptly() {
        let d = TempDir::new().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            flag.store(true, Ordering::SeqCst);
        });
        #[cfg(unix)]
        let slow = "sleep 20";
        #[cfg(windows)]
        let slow = "ping -n 20 127.0.0.1 >nul";
        let t = Instant::now();
        let out = run(d.path(), slow, &lim(), &cancel).unwrap();
        assert!(t.elapsed() < Duration::from_secs(8), "{:?}", t.elapsed());
        assert!(out.contains("was stopped"), "{out}");
    }

    #[test]
    fn secrets_in_the_environment_do_not_reach_the_command() {
        let d = TempDir::new().unwrap();
        // SAFETY of the test: a name nothing else reads.
        std::env::set_var("ROOST_PROBE_SECRET_TOKEN", "hunter2");
        std::env::set_var("ROOST_PROBE_PLAIN_NAME", "visible-value");
        #[cfg(unix)]
        let show = "echo [$ROOST_PROBE_SECRET_TOKEN] [$ROOST_PROBE_PLAIN_NAME]";
        #[cfg(windows)]
        let show = "echo [%ROOST_PROBE_SECRET_TOKEN%] [%ROOST_PROBE_PLAIN_NAME%]";
        let out = go(&d, show, &lim());
        assert!(!out.contains("hunter2"), "{out}");
        // ROOST_ is withheld wholesale, so the plain one is gone too.
        assert!(!out.contains("visible-value"), "{out}");
        assert!(withheld("HF_TOKEN") && withheld("my_api_key") && withheld("AWS_REGION"));
        assert!(!withheld("PATH") && !withheld("HOME") && !withheld("CARGO_HOME"));
    }

    #[test]
    fn the_allow_prefix_is_the_program_and_a_plain_subcommand() {
        assert_eq!(
            allow_prefix("cargo test -p roost-core").as_deref(),
            Some("cargo test")
        );
        assert_eq!(allow_prefix("git status").as_deref(), Some("git status"));
        assert_eq!(allow_prefix("ls -la").as_deref(), Some("ls"));
        assert_eq!(allow_prefix("npm run build").as_deref(), Some("npm run"));
        assert_eq!(allow_prefix("cargo --version").as_deref(), Some("cargo"));
    }

    #[test]
    fn chained_dangerous_or_path_commands_get_no_allowance() {
        for c in [
            "cargo test && rm -rf ~",
            "ls; rm x",
            "cat a | sh",
            "echo $(whoami)",
            "echo `id`",
            "ls > out.txt",
            "ls\nrm x",
            "rm -rf build",
            "bash -c 'x'",
            "python3 script.py",
            "curl https://example.com",
            "sudo ls",
            "./run.sh",
            "C:\\\\tools\\\\x.exe",
            "   ",
        ] {
            assert_eq!(allow_prefix(c), None, "{c}");
        }
    }

    #[test]
    fn an_allowance_covers_that_prefix_only() {
        assert!(covered_by("cargo test", "cargo test"));
        assert!(covered_by("cargo test -p roost-core", "cargo test"));
        assert!(!covered_by("cargo build", "cargo test"));
        assert!(!covered_by("cargo testing", "cargo test"));
        assert!(!covered_by("cargo test && rm -rf ~", "cargo test"));
        assert!(!covered_by("cargo test; ls", "cargo test"));
        assert!(covered_by("cargo --version", "cargo"));
        assert!(!covered_by("cargofoo x", "cargo"));
    }
}
