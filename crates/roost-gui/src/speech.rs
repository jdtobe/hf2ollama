//! Reading a reply aloud with the operating system's own speech tools.
//!
//! No speech library is linked: macOS has `say`, Windows has the .NET speech
//! synthesizer through PowerShell, and Linux uses `spd-say` or `espeak` when
//! one is installed. The text goes in on the child's standard input, so it is
//! never part of a command line. If nothing suitable is found, reading is
//! simply unavailable and the button says why.

use crate::browse::markdown::{self, Block, Span};
use std::ffi::OsStr;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

/// What the window needs from a speech engine (a fake in tests).
pub trait Speaker {
    /// `Ok` when reading aloud can work, else a sentence saying why not.
    fn available(&self) -> Result<(), String>;
    /// Starts reading `text`, replacing anything being read.
    fn speak(&mut self, text: &str);
    fn stop(&mut self);
    fn is_speaking(&mut self) -> bool;
}

/// A program and its arguments; the text to read is written to its stdin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub program: String,
    pub args: Vec<String>,
}

const WINDOWS_SCRIPT: &str = "Add-Type -AssemblyName System.Speech; \
(New-Object System.Speech.Synthesis.SpeechSynthesizer).Speak([Console]::In.ReadToEnd())";

/// The speech command for `os` (as in `std::env::consts::OS`), given a way to
/// ask whether a program exists.
pub fn choose(os: &str, has: impl Fn(&str) -> bool) -> Option<Spec> {
    let spec = |program: &str, args: &[&str]| Spec {
        program: program.to_string(),
        args: args.iter().map(|a| a.to_string()).collect(),
    };
    match os {
        "macos" => has("say").then(|| spec("say", &["-f", "-"])),
        "windows" => Some(spec(
            "powershell.exe",
            &["-NoProfile", "-NonInteractive", "-Command", WINDOWS_SCRIPT],
        )),
        _ => {
            if has("spd-say") {
                Some(spec("spd-say", &["-e", "-w"]))
            } else if has("espeak-ng") {
                Some(spec("espeak-ng", &["--stdin"]))
            } else if has("espeak") {
                Some(spec("espeak", &["--stdin"]))
            } else {
                None
            }
        }
    }
}

/// The file `name` in one of the directories of `path_var`, if any.
pub fn locate(name: &str, path_var: &OsStr) -> Option<PathBuf> {
    std::env::split_paths(path_var)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

/// Why reading is unavailable here, in words for a tooltip.
pub fn unavailable_reason(os: &str) -> &'static str {
    match os {
        "macos" => "Reading aloud needs the macOS `say` command, which was not found.",
        "windows" => "Reading aloud needs Windows PowerShell, which was not found.",
        _ => "Reading aloud needs speech-dispatcher (spd-say) or espeak. Install one and restart Roost.",
    }
}

/// Speaks through a child process.
pub struct SystemSpeaker {
    spec: Option<Spec>,
    child: Option<Child>,
}

impl SystemSpeaker {
    /// Looks for a speech command on this computer.
    pub fn detect() -> SystemSpeaker {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let spec = choose(std::env::consts::OS, |name| locate(name, &path).is_some());
        SystemSpeaker { spec, child: None }
    }

    #[cfg(test)]
    pub fn with_spec(spec: Option<Spec>) -> SystemSpeaker {
        SystemSpeaker { spec, child: None }
    }
}

impl Speaker for SystemSpeaker {
    fn available(&self) -> Result<(), String> {
        match self.spec {
            Some(_) => Ok(()),
            None => Err(unavailable_reason(std::env::consts::OS).to_string()),
        }
    }

    fn speak(&mut self, text: &str) {
        self.stop();
        let Some(spec) = &self.spec else {
            return;
        };
        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // No console window flashing up.
            command.creation_flags(0x0800_0000);
        }
        let Ok(mut child) = command.spawn() else {
            return;
        };
        if let Some(mut stdin) = child.stdin.take() {
            // A failed write just means the tool quit early.
            let _ = stdin.write_all(text.as_bytes());
        }
        self.child = Some(child);
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn is_speaking(&mut self) -> bool {
        match self.child.as_mut().map(Child::try_wait) {
            Some(Ok(None)) => true,
            Some(_) => {
                self.child = None;
                false
            }
            None => false,
        }
    }
}

impl Drop for SystemSpeaker {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The words of a Markdown reply, without the marks: headings and paragraphs
/// as sentences, list items one per line, table rows as lines, link text
/// kept. Code blocks are left out, since reading symbols aloud helps nobody.
pub fn plain_text(markdown_text: &str) -> String {
    let blocks = markdown::parse_capped(markdown_text).0;
    let mut out = String::new();
    push_blocks(&mut out, &blocks);
    out.trim().to_string()
}

fn spans(list: &[Span]) -> String {
    list.iter().map(|s| s.text.as_str()).collect()
}

fn push_blocks(out: &mut String, blocks: &[Block]) {
    for b in blocks {
        match b {
            Block::Heading(_, s) | Block::Paragraph(s) => {
                let text = spans(s);
                out.push_str(text.trim());
                out.push_str("\n\n");
            }
            Block::List { items, .. } => {
                for item in items {
                    push_blocks(out, item);
                }
            }
            Block::Quote(inner) => push_blocks(out, inner),
            Block::CodeBlock(_) | Block::Rule => {}
            Block::Table(rows) => {
                for row in rows {
                    let cells: Vec<String> = row.iter().map(|c| spans(c)).collect();
                    out.push_str(&cells.join(", "));
                    out.push('\n');
                }
                out.push('\n');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_only<'a>(names: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
        move |n| names.contains(&n)
    }

    #[test]
    fn each_platform_picks_its_own_tool() {
        assert_eq!(choose("macos", has_only(&["say"])).unwrap().program, "say");
        assert_eq!(choose("macos", has_only(&[])), None);
        let win = choose("windows", has_only(&[])).unwrap();
        assert_eq!(win.program, "powershell.exe");
        assert!(win.args.last().unwrap().contains("ReadToEnd"));
        assert_eq!(
            choose("linux", has_only(&["espeak", "spd-say"]))
                .unwrap()
                .program,
            "spd-say",
            "speech-dispatcher first"
        );
        assert_eq!(
            choose("linux", has_only(&["espeak"])).unwrap().program,
            "espeak"
        );
        assert_eq!(choose("linux", has_only(&[])), None);
    }

    #[test]
    fn locate_finds_a_file_in_a_listed_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("spd-say"), b"").unwrap();
        let other = tempfile::tempdir().unwrap();
        let path = std::env::join_paths([other.path(), dir.path()]).unwrap();
        assert_eq!(locate("spd-say", &path), Some(dir.path().join("spd-say")));
        assert_eq!(locate("espeak", &path), None);
    }

    #[test]
    fn no_engine_means_unavailable_with_a_reason() {
        let s = SystemSpeaker::with_spec(None);
        assert!(s.available().unwrap_err().contains("Reading aloud"));
    }

    #[test]
    fn plain_text_drops_the_marks_and_the_code() {
        let md = "# Title\n\nSome **bold** and a [link](https://example.com).\n\n- one\n- two\n\n```\nlet x = 1;\n```\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n---\n\n> quoted";
        let t = plain_text(md);
        assert!(t.contains("Title"));
        assert!(t.contains("Some bold and a link."));
        assert!(t.contains("one") && t.contains("two"));
        assert!(t.contains("1, 2"));
        assert!(t.contains("quoted"));
        for gone in ["#", "**", "](", "let x", "```", "---", "|"] {
            assert!(!t.contains(gone), "{gone:?} in {t:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_running_speaker_can_be_stopped() {
        // `sleep` ignores its input, which stands in for a long speech.
        let mut s = SystemSpeaker::with_spec(Some(Spec {
            program: "sleep".into(),
            args: vec!["30".into()],
        }));
        assert!(s.available().is_ok());
        s.speak("hello");
        assert!(s.is_speaking());
        s.stop();
        assert!(!s.is_speaking());
    }

    #[cfg(unix)]
    #[test]
    fn a_finished_speech_stops_being_speaking() {
        let mut s = SystemSpeaker::with_spec(Some(Spec {
            program: "cat".into(),
            args: vec![],
        }));
        s.speak("hello");
        for _ in 0..200 {
            if !s.is_speaking() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("cat never finished");
    }

    #[test]
    fn a_missing_program_does_not_crash() {
        let mut s = SystemSpeaker::with_spec(Some(Spec {
            program: "definitely-not-a-program-xyz".into(),
            args: vec![],
        }));
        s.speak("hello");
        assert!(!s.is_speaking());
    }
}
