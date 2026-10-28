//! The workspace jail. Every path a tool uses is resolved through a
//! capability directory handle rooted at the attached folder (`cap-std`), so
//! `..`, absolute paths, escaping symlinks and junctions cannot leave it, and
//! there is no check-then-open race on an absolute path.

use cap_std::ambient_authority;
use cap_std::fs::{Dir, File};
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

/// A tool failed or refused. The text is shown to the model and the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ToolError {
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("access denied: {0}")]
    Denied(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("{0} is not a directory")]
    NotADirectory(String),
    #[error("{0} is not a file")]
    NotAFile(String),
    #[error("{0}")]
    Unreadable(String),
    #[error("invalid arguments: {0}")]
    InvalidArguments(String),
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    #[error("the tool {0} is turned off")]
    Off(String),
    #[error("the tool {0} needs approval, which is not available yet")]
    NeedsApproval(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Dir,
    File,
    Symlink,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
}

pub struct Workspace {
    root: PathBuf,
    dir: Dir,
}

impl std::fmt::Debug for Workspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workspace")
            .field("root", &self.root)
            .finish()
    }
}

/// Paths that are never readable through a tool, whatever the model asks:
/// `.env`, `.env.*`, `*.pem`, `*.key`, `.ssh/**`, `.aws/**`,
/// `.config/gcloud/**`, and files named `credentials`, `credentials.json`,
/// `secrets` or `secrets.json`. Comparison ignores case.
pub fn is_sensitive(rel: &Path) -> bool {
    let parts: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(n) => Some(n.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect();
    if parts.iter().any(|p| p == ".ssh" || p == ".aws") {
        return true;
    }
    if parts
        .windows(2)
        .any(|w| w[0] == ".config" && w[1] == "gcloud")
    {
        return true;
    }
    let Some(name) = parts.last() else {
        return false;
    };
    name == ".env"
        || name.starts_with(".env.")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || matches!(
            name.as_str(),
            "credentials" | "credentials.json" | "secrets" | "secrets.json"
        )
}

fn describe(rel: &Path) -> String {
    if rel.as_os_str().is_empty() {
        ".".to_string()
    } else {
        rel.display().to_string()
    }
}

fn map_io(rel: &Path, e: std::io::Error) -> ToolError {
    match e.kind() {
        ErrorKind::NotFound => ToolError::NotFound(describe(rel)),
        // cap-std reports an escape attempt as a permission error.
        ErrorKind::PermissionDenied => ToolError::Denied(format!(
            "{} is outside the workspace or not readable",
            describe(rel)
        )),
        _ => ToolError::Unreadable(format!("could not read {}: {e}", describe(rel))),
    }
}

impl Workspace {
    /// Opens `root`, which must be an existing directory.
    pub fn open(root: &Path) -> Result<Workspace, ToolError> {
        if !root.is_absolute() {
            return Err(ToolError::InvalidPath(
                "the workspace path must be absolute".into(),
            ));
        }
        let dir = Dir::open_ambient_dir(root, ambient_authority())
            .map_err(|e| ToolError::Unreadable(format!("could not open the workspace: {e}")))?;
        Ok(Workspace {
            root: root.to_path_buf(),
            dir,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// A workspace-relative path from a model-supplied string. Rejects NUL,
    /// absolute paths and any `..`. An empty string means the workspace itself.
    pub fn resolve(&self, input: &str) -> Result<PathBuf, ToolError> {
        if input.contains('\0') {
            return Err(ToolError::InvalidPath("paths cannot contain NUL".into()));
        }
        let path = Path::new(input);
        let mut clean = PathBuf::new();
        for c in path.components() {
            match c {
                Component::Normal(n) => clean.push(n),
                Component::CurDir => {}
                Component::ParentDir => {
                    return Err(ToolError::InvalidPath(
                        "parent directory references (..) are not allowed".into(),
                    ))
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(ToolError::InvalidPath(
                        "absolute paths are not allowed; use a path inside the workspace".into(),
                    ))
                }
            }
        }
        Ok(clean)
    }

    fn here(rel: &Path) -> &Path {
        if rel.as_os_str().is_empty() {
            Path::new(".")
        } else {
            rel
        }
    }

    /// Every entry of a directory, sorted by name. Symlinks are reported, not
    /// followed.
    pub fn list(&self, rel: &Path) -> Result<Vec<Entry>, ToolError> {
        let target = Self::here(rel);
        let meta = self.dir.metadata(target).map_err(|e| map_io(rel, e))?;
        if !meta.is_dir() {
            return Err(ToolError::NotADirectory(describe(rel)));
        }
        let read = self.dir.read_dir(target).map_err(|e| map_io(rel, e))?;
        let mut out = Vec::new();
        for entry in read {
            let entry = entry.map_err(|e| map_io(rel, e))?;
            let kind = match entry.file_type() {
                Ok(t) if t.is_symlink() => EntryKind::Symlink,
                Ok(t) if t.is_dir() => EntryKind::Dir,
                Ok(t) if t.is_file() => EntryKind::File,
                _ => EntryKind::Other,
            };
            out.push(Entry {
                name: entry.file_name().to_string_lossy().into_owned(),
                kind,
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Opens a regular file for reading. Sensitive names are refused first,
    /// whether or not the file exists.
    pub fn open_file(&self, rel: &Path) -> Result<(File, u64), ToolError> {
        if is_sensitive(rel) {
            return Err(ToolError::Denied(format!(
                "{} is protected and cannot be read",
                describe(rel)
            )));
        }
        let meta = self
            .dir
            .metadata(Self::here(rel))
            .map_err(|e| map_io(rel, e))?;
        if meta.is_dir() {
            return Err(ToolError::NotAFile(describe(rel)));
        }
        let file = self.dir.open(Self::here(rel)).map_err(|e| map_io(rel, e))?;
        Ok((file, meta.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn ws() -> (TempDir, TempDir, Workspace) {
        let outside = TempDir::new().unwrap();
        fs::write(outside.path().join("secret.txt"), "top secret").unwrap();
        let root = TempDir::new().unwrap();
        fs::write(root.path().join("a.txt"), "hello\nworld\n").unwrap();
        fs::create_dir(root.path().join("sub")).unwrap();
        fs::write(root.path().join("sub/b.txt"), "inner").unwrap();
        let w = Workspace::open(root.path()).unwrap();
        (root, outside, w)
    }

    #[test]
    fn resolve_accepts_plain_relative_paths() {
        let (_r, _o, w) = ws();
        assert_eq!(w.resolve("a/b").unwrap(), PathBuf::from("a/b"));
        assert_eq!(w.resolve("./a").unwrap(), PathBuf::from("a"));
        assert_eq!(w.resolve("").unwrap(), PathBuf::new());
        assert_eq!(w.resolve(".").unwrap(), PathBuf::new());
    }

    #[test]
    fn resolve_rejects_nul_parents_and_absolute_paths() {
        let (_r, _o, w) = ws();
        for bad in ["..", "../x", "a/../../b", "a/..", "/etc/passwd", "a\0b"] {
            assert!(
                matches!(w.resolve(bad), Err(ToolError::InvalidPath(_))),
                "{bad:?} should be rejected"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn resolve_rejects_drive_and_unc_paths_on_windows() {
        let (_r, _o, w) = ws();
        for bad in ["C:\\Windows", "C:foo", "\\\\server\\share\\x", "\\rooted"] {
            assert!(
                matches!(w.resolve(bad), Err(ToolError::InvalidPath(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn listing_and_opening_inside_the_workspace_work() {
        let (_r, _o, w) = ws();
        let names: Vec<_> = w
            .list(Path::new(""))
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["a.txt", "sub"]);
        let (_f, len) = w.open_file(Path::new("sub/b.txt")).unwrap();
        assert_eq!(len, 5);
    }

    #[test]
    fn missing_and_wrong_kind_paths_are_clear_errors() {
        let (_r, _o, w) = ws();
        assert!(matches!(
            w.list(Path::new("nope")),
            Err(ToolError::NotFound(_))
        ));
        assert!(matches!(
            w.list(Path::new("a.txt")),
            Err(ToolError::NotADirectory(_))
        ));
        assert!(matches!(
            w.open_file(Path::new("sub")),
            Err(ToolError::NotAFile(_))
        ));
        assert!(matches!(
            w.open_file(Path::new("nope.txt")),
            Err(ToolError::NotFound(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_that_escape_the_workspace_are_refused() {
        use std::os::unix::fs::symlink;
        let (r, o, w) = ws();
        symlink(o.path(), r.path().join("out_dir")).unwrap();
        symlink(o.path().join("secret.txt"), r.path().join("out_file")).unwrap();
        symlink("../../../../../../etc", r.path().join("up")).unwrap();
        assert!(matches!(
            w.list(Path::new("out_dir")),
            Err(ToolError::Denied(_))
        ));
        assert!(matches!(
            w.open_file(Path::new("out_file")),
            Err(ToolError::Denied(_))
        ));
        assert!(matches!(
            w.open_file(Path::new("out_dir/secret.txt")),
            Err(ToolError::Denied(_))
        ));
        assert!(w.list(Path::new("up")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_inside_the_workspace_still_work_and_are_reported_not_followed_in_listings() {
        use std::os::unix::fs::symlink;
        let (r, _o, w) = ws();
        symlink("a.txt", r.path().join("alias")).unwrap();
        assert!(w.open_file(Path::new("alias")).is_ok());
        let kinds: Vec<_> = w
            .list(Path::new(""))
            .unwrap()
            .into_iter()
            .map(|e| (e.name, e.kind))
            .collect();
        assert!(kinds.contains(&("alias".to_string(), EntryKind::Symlink)));
    }

    #[test]
    fn sensitive_names_are_recognised_case_insensitively() {
        for p in [
            ".env",
            ".env.local",
            ".ENV",
            "app/.env.production",
            "keys/server.pem",
            "id.KEY",
            ".ssh/id_rsa",
            "home/.aws/credentials",
            ".config/gcloud/application_default_credentials.json",
            "a/.Config/GCloud/x",
            "credentials",
            "deep/Credentials.json",
            "secrets",
            "SECRETS.JSON",
        ] {
            assert!(is_sensitive(Path::new(p)), "{p} should be protected");
        }
    }

    #[test]
    fn ordinary_names_that_only_look_similar_are_not_protected() {
        for p in [
            "environment.md",
            ".environment",
            "keyboard.txt",
            "monkey.rs",
            "secrets.md",
            "credentials.txt",
            "config/gcloud-notes.txt",
            ".config/other/file",
            "ssh/readme",
            "src/main.rs",
        ] {
            assert!(!is_sensitive(Path::new(p)), "{p} should be allowed");
        }
    }

    #[test]
    fn protected_files_are_refused_even_when_they_do_not_exist() {
        let (r, _o, w) = ws();
        fs::write(r.path().join(".env"), "KEY=1").unwrap();
        assert!(matches!(
            w.open_file(Path::new(".env")),
            Err(ToolError::Denied(_))
        ));
        assert!(matches!(
            w.open_file(Path::new("ghost.pem")),
            Err(ToolError::Denied(_))
        ));
    }

    #[test]
    fn the_workspace_must_be_an_absolute_existing_directory() {
        assert!(matches!(
            Workspace::open(Path::new("relative/dir")),
            Err(ToolError::InvalidPath(_))
        ));
        let t = TempDir::new().unwrap();
        assert!(Workspace::open(&t.path().join("missing")).is_err());
        let file = t.path().join("f");
        fs::write(&file, "x").unwrap();
        assert!(Workspace::open(&file).is_err());
    }
}
